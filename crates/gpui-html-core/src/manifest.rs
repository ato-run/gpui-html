//! Host-app theme manifest loader.
//!
//! The compiler is intentionally ignorant of the host's `Theme` struct
//! by default — `bg-surface` lowers to `theme.surface` and rustc later
//! validates the field exists. That keeps `gpui-html-core` independent
//! of any host's color choices.
//!
//! When a manifest is supplied, the compiler tightens the contract:
//!
//! - **Color tokens** under `[colors]` validate `bg-X` / `text-X` /
//!   `border-X` / CSS `var(--theme-X)` at lowering time. Unknown tokens
//!   surface as `UnknownThemeToken` with a span instead of being
//!   discovered at the Rust compile step.
//! - **Sizing tokens** under `[max-width]` / `[max-height]` /
//!   `[min-width]` / `[min-height]` resolve custom-scale utilities
//!   (e.g. `max-w-128` from a Tailwind-config-extended scale). The
//!   value is a CSS length string — currently only `<n>rem` and
//!   `<n>px` are accepted; the compiler converts them to `rems(N.0)`
//!   for the emitted GPUI builder call.
//!
//! Without a manifest, the compiler falls back to its built-in
//! behavior:
//!
//! - Theme tokens pass through symbolically (no validation).
//! - Only the `max-w-128` single-token compatibility exemption is
//!   recognized; other custom scales reject with a hint pointing at
//!   the manifest path.
//!
//! Schema example (TOML):
//!
//! ```toml
//! [colors]
//! base = "#09090b"
//! surface = "#09090b"
//! accent = "#6366f1"
//! "accent-foreground" = "#ffffff"
//!
//! [max-width]
//! "128" = "32rem"
//!
//! [max-height]
//! "screen-90" = "90vh"   # rejected — only rem/px supported in v0.1
//! ```
//!
//! See `examples/example.manifest.toml` for a fuller example matched
//! to the Ato Desktop preview fixture.

use std::collections::HashMap;

use serde::Deserialize;

/// Parsed manifest. Hyphens in TOML keys are preserved as-is — the
/// caller looks them up by the *original* token name (`accent-foreground`),
/// not the snake-case Rust ident (`accent_foreground`). The
/// hyphen-to-snake normalization happens in `class_map.rs` after the
/// manifest validation accepts the token.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeManifest {
    /// Names that may appear after `bg-` / `text-` / `border-` and in
    /// CSS `var(--theme-X)`, mapped to their packed RGBA bytes parsed
    /// from the TOML `[colors]` table. Values are load-bearing for #23
    /// (theme-token alpha): `bg-<name>/<n>` lowers to a literal
    /// `gpui::rgba(0xRRGGBBAA)` by combining the manifest's RGB with
    /// the slash-alpha. Plain `bg-<name>` (no slash) still lowers as
    /// `theme.<name>` and ignores the manifest's value — host runtime
    /// owns the color.
    colors: HashMap<String, [u8; 4]>,
    /// `max-w-<suffix>` custom scale entries. Value is a CSS length
    /// string converted to a `rems(N.0)` Rust expression.
    max_width: HashMap<String, String>,
    /// `max-h-<suffix>` custom scale entries.
    max_height: HashMap<String, String>,
    /// `min-w-<suffix>` custom scale entries.
    min_width: HashMap<String, String>,
    /// `min-h-<suffix>` custom scale entries.
    min_height: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// TOML syntax error (mismatched braces, malformed table header,
    /// unclosed string, etc.).
    Parse(String),
    /// A value in a sizing table doesn't parse as a supported CSS
    /// length. v0.1 accepts `<n>rem` and `<n>px` only.
    UnsupportedScaleValue {
        section: &'static str,
        key: String,
        value: String,
    },
    /// A `[colors]` entry value isn't a hex string the parser
    /// recognises. v0.1 accepts `#rgb`, `#rrggbb`, and `#rrggbbaa`
    /// (case-insensitive).
    InvalidColor { name: String, value: String },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Parse(msg) => write!(f, "manifest TOML parse error: {msg}"),
            ManifestError::UnsupportedScaleValue {
                section,
                key,
                value,
            } => write!(
                f,
                "manifest [{section}] entry `{key} = \"{value}\"` is not a \
                 supported CSS length (v0.1 accepts only `<n>rem` and `<n>px`)"
            ),
            ManifestError::InvalidColor { name, value } => write!(
                f,
                "manifest [colors] entry `{name} = \"{value}\"` is not a \
                 supported hex color (v0.1 accepts `#rgb`, `#rrggbb`, or \
                 `#rrggbbaa`)"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}

impl ThemeManifest {
    /// Parse a TOML manifest source. The schema is permissive: any
    /// section is optional, unknown sections are quietly ignored (so
    /// future extensions don't break older parsers).
    ///
    /// Sizing values are eagerly converted to `rems(N.0)` Rust source
    /// fragments so the codegen path can splice them verbatim without
    /// re-parsing.
    pub fn from_toml(src: &str) -> Result<Self, ManifestError> {
        #[derive(Deserialize, Default)]
        struct Schema {
            #[serde(default)]
            colors: HashMap<String, toml::Value>,
            #[serde(default, rename = "max-width")]
            max_width: HashMap<String, String>,
            #[serde(default, rename = "max-height")]
            max_height: HashMap<String, String>,
            #[serde(default, rename = "min-width")]
            min_width: HashMap<String, String>,
            #[serde(default, rename = "min-height")]
            min_height: HashMap<String, String>,
        }

        let schema: Schema =
            toml::from_str(src).map_err(|e| ManifestError::Parse(e.to_string()))?;

        let colors = convert_colors_table(schema.colors)?;
        let max_width = convert_scale_table("max-width", schema.max_width)?;
        let max_height = convert_scale_table("max-height", schema.max_height)?;
        let min_width = convert_scale_table("min-width", schema.min_width)?;
        let min_height = convert_scale_table("min-height", schema.min_height)?;

        Ok(ThemeManifest {
            colors,
            max_width,
            max_height,
            min_width,
            min_height,
        })
    }

    /// Does the manifest declare a color token named `name`? Used by
    /// `class_map::normalize_theme_token` to gate `bg-<name>` /
    /// `text-<name>` / `border-<name>` when a manifest is in scope.
    /// Names are matched by the *original* hyphenated form, not the
    /// snake-case Rust ident — TOML keys can be hyphenated freely.
    pub fn knows_color(&self, name: &str) -> bool {
        self.colors.contains_key(name)
    }

    /// The set of declared color names. Exposed for diagnostics that
    /// want to suggest the closest match on rejection.
    pub fn color_names(&self) -> impl Iterator<Item = &str> {
        self.colors.keys().map(String::as_str)
    }

    /// Look up the parsed RGBA bytes for a declared color token. Used by
    /// the `bg-<name>/<alpha>` lowering path (#23) to build the
    /// `gpui::rgba(0xRRGGBBAA)` literal: the RGB channels come from this
    /// table and the alpha byte is overwritten by the slash suffix.
    /// Returns `None` for unknown names — callers should pair with
    /// `knows_color` for the validation message rather than relying on
    /// the `None` here.
    pub fn lookup_color_rgba(&self, name: &str) -> Option<[u8; 4]> {
        self.colors.get(name).copied()
    }

    /// Look up `max-w-<key>` → pre-formatted `rems(N.0)` expression
    /// (e.g. `"32.0".into()` for `"128" = "32rem"`). Returns `None`
    /// when the suffix isn't in the manifest.
    pub fn lookup_max_width(&self, suffix: &str) -> Option<&str> {
        self.max_width.get(suffix).map(String::as_str)
    }

    pub fn lookup_max_height(&self, suffix: &str) -> Option<&str> {
        self.max_height.get(suffix).map(String::as_str)
    }

    pub fn lookup_min_width(&self, suffix: &str) -> Option<&str> {
        self.min_width.get(suffix).map(String::as_str)
    }

    pub fn lookup_min_height(&self, suffix: &str) -> Option<&str> {
        self.min_height.get(suffix).map(String::as_str)
    }
}

/// Translate each `(name, toml_value)` entry in `[colors]` into
/// `(name, [r, g, b, a])`. Values must be strings of the form `#rgb`,
/// `#rrggbb`, or `#rrggbbaa` — anything else (a non-string, an unknown
/// hex shape, non-hex digits, etc.) returns `InvalidColor` so manifest
/// authors learn at load time, not at lowering time.
///
/// `#rgb` short form expands `#abc` → `#aabbcc` per CSS rules. `#rrggbb`
/// defaults alpha to `0xff`. `#rrggbbaa` keeps its alpha — though the
/// theme-token alpha lowering (#23) overrides it with the slash alpha
/// at the call site.
fn convert_colors_table(
    table: HashMap<String, toml::Value>,
) -> Result<HashMap<String, [u8; 4]>, ManifestError> {
    let mut out = HashMap::with_capacity(table.len());
    for (name, value) in table {
        let raw = match value {
            toml::Value::String(s) => s,
            other => {
                return Err(ManifestError::InvalidColor {
                    name,
                    value: format!("{other:?}"),
                });
            }
        };
        let rgba = parse_hex_color(&raw).ok_or_else(|| ManifestError::InvalidColor {
            name: name.clone(),
            value: raw.clone(),
        })?;
        out.insert(name, rgba);
    }
    Ok(out)
}

/// Parse a CSS hex color literal into packed `[r, g, b, a]` bytes.
/// Accepts the three v0.1 shapes:
///
/// - `#rgb`    → each nibble doubled (`#abc` → `[0xaa, 0xbb, 0xcc, 0xff]`)
/// - `#rrggbb` → alpha defaults to `0xff`
/// - `#rrggbbaa`
///
/// Case-insensitive. Returns `None` for any other shape — `#rgba`
/// (short form with alpha), 4-digit hex, named CSS colors (`red`),
/// and `rgb()`/`hsl()` function calls all reject.
fn parse_hex_color(raw: &str) -> Option<[u8; 4]> {
    let hex = raw.strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let r = expand_nibble(&hex[0..1])?;
            let g = expand_nibble(&hex[1..2])?;
            let b = expand_nibble(&hex[2..3])?;
            Some([r, g, b, 0xff])
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b, 0xff])
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some([r, g, b, a])
        }
        _ => None,
    }
}

/// Expand a single hex nibble (`"a"`) into a byte (`0xaa`) per CSS
/// short-form rules.
fn expand_nibble(s: &str) -> Option<u8> {
    let n = u8::from_str_radix(s, 16).ok()?;
    Some(n * 17) // n * 0x11 == nibble doubled
}

/// Translate each `(suffix, css_length)` entry into
/// `(suffix, "rems(N.0)")` so the codegen layer can splice the value
/// directly into a `MethodCall::unary("max_w", ...)` etc.
///
/// Only the `<n>rem` and `<n>px` shapes are accepted. `<n>px` is
/// converted using the standard `16px = 1rem` ratio.
fn convert_scale_table(
    section: &'static str,
    table: HashMap<String, String>,
) -> Result<HashMap<String, String>, ManifestError> {
    let mut out = HashMap::with_capacity(table.len());
    for (key, value) in table {
        let rems = parse_css_length_to_rems(&value).ok_or_else(|| {
            ManifestError::UnsupportedScaleValue {
                section,
                key: key.clone(),
                value: value.clone(),
            }
        })?;
        out.insert(key, format!("rems({rems})"));
    }
    Ok(out)
}

/// Accept `<n>rem` or `<n>px` and return the equivalent rem value as a
/// Rust f32-shaped literal (`"32.0"`, `"1.5"`, ...). 16px == 1rem.
fn parse_css_length_to_rems(value: &str) -> Option<String> {
    let v = value.trim();
    let (number_str, scale): (&str, f64) = if let Some(stripped) = v.strip_suffix("rem") {
        (stripped.trim(), 1.0)
    } else if let Some(stripped) = v.strip_suffix("px") {
        (stripped.trim(), 1.0 / 16.0)
    } else {
        return None;
    };
    let n: f64 = number_str.parse().ok()?;
    if n < 0.0 {
        return None;
    }
    let rems = n * scale;
    // Render a Rust-valid f32 literal. `format!("{rems}")` produces
    // `32` for whole numbers (not a valid f32 literal); force at least
    // one decimal place.
    if rems.fract() == 0.0 {
        Some(format!("{rems:.1}"))
    } else {
        Some(format!("{rems}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> ThemeManifest {
        ThemeManifest::from_toml(src).expect("manifest should parse")
    }

    #[test]
    fn empty_manifest_parses() {
        let m = parse_ok("");
        assert!(m.color_names().next().is_none());
        assert!(m.lookup_max_width("128").is_none());
    }

    #[test]
    fn colors_table_loads_keys_for_validation() {
        let m = parse_ok(
            r##"
            [colors]
            base = "#09090b"
            surface = "#09090b"
            accent = "#6366f1"
            "accent-foreground" = "#ffffff"
            "##,
        );
        assert!(m.knows_color("base"));
        assert!(m.knows_color("accent"));
        assert!(m.knows_color("accent-foreground"));
        assert!(!m.knows_color("red-500"));
        // Hyphenated keys round-trip unchanged — snake_case
        // normalization happens later in class_map.
        let mut names: Vec<&str> = m.color_names().collect();
        names.sort();
        assert_eq!(
            names,
            vec!["accent", "accent-foreground", "base", "surface"]
        );
    }

    #[test]
    fn colors_table_parses_rgba_for_alpha_lowering() {
        // #23: alpha lowering reads the manifest's RGB and stitches the
        // slash alpha onto it. Lock down the parser for the three
        // accepted hex shapes.
        let m = parse_ok(
            r##"
            [colors]
            accent  = "#6366f1"
            short   = "#abc"
            "with-a" = "#11223344"
            "##,
        );
        assert_eq!(
            m.lookup_color_rgba("accent"),
            Some([0x63, 0x66, 0xf1, 0xff])
        );
        assert_eq!(m.lookup_color_rgba("short"), Some([0xaa, 0xbb, 0xcc, 0xff]));
        assert_eq!(
            m.lookup_color_rgba("with-a"),
            Some([0x11, 0x22, 0x33, 0x44])
        );
        assert_eq!(m.lookup_color_rgba("unknown"), None);
    }

    #[test]
    fn colors_table_accepts_uppercase_hex() {
        // Manifest authors may copy-paste from design tools that emit
        // uppercase hex. Accept either case — output is lowercase by
        // codegen convention, not parser convention.
        let m = parse_ok(
            r##"
            [colors]
            accent = "#6366F1"
            "##,
        );
        assert_eq!(
            m.lookup_color_rgba("accent"),
            Some([0x63, 0x66, 0xf1, 0xff])
        );
    }

    #[test]
    fn colors_table_rejects_invalid_hex_shapes() {
        // 4-digit hex (short-form with alpha) and named colors aren't
        // in v0.1's accepted set — the parser rejects them at load time
        // so authors learn before lowering.
        for value in ["#rgba", "#1234", "red", "rgb(0, 0, 0)", "#12", "#1234567"] {
            let toml = format!("[colors]\n\"x\" = \"{value}\"");
            let err = ThemeManifest::from_toml(&toml).unwrap_err();
            match err {
                ManifestError::InvalidColor {
                    name,
                    value: got_value,
                } => {
                    assert_eq!(name, "x");
                    assert_eq!(got_value, value);
                }
                other => panic!("expected InvalidColor for {value:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn colors_table_rejects_non_hex_digits() {
        // `#xxyyzz` has the right shape but the hex digits aren't valid.
        let err = ThemeManifest::from_toml(
            r##"[colors]
"foo" = "#xxyyzz""##,
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::InvalidColor { .. }));
    }

    #[test]
    fn colors_table_rejects_non_string_values() {
        // A misformatted manifest that puts a number where a hex string
        // belongs surfaces as InvalidColor, not silent ignore.
        let err = ThemeManifest::from_toml(
            r#"[colors]
"foo" = 42"#,
        )
        .unwrap_err();
        match err {
            ManifestError::InvalidColor { name, .. } => assert_eq!(name, "foo"),
            other => panic!("expected InvalidColor, got {other:?}"),
        }
    }

    #[test]
    fn max_width_rem_value_converts_to_rems_literal() {
        let m = parse_ok(
            r#"
            [max-width]
            "128" = "32rem"
            "page" = "48rem"
            "#,
        );
        assert_eq!(m.lookup_max_width("128"), Some("rems(32.0)"));
        assert_eq!(m.lookup_max_width("page"), Some("rems(48.0)"));
        assert_eq!(m.lookup_max_width("unknown"), None);
    }

    #[test]
    fn max_width_px_value_converts_via_16px_per_rem() {
        let m = parse_ok(
            r#"
            [max-width]
            "small" = "16px"
            "big" = "1024px"
            "fraction" = "8px"
            "#,
        );
        assert_eq!(m.lookup_max_width("small"), Some("rems(1.0)"));
        assert_eq!(m.lookup_max_width("big"), Some("rems(64.0)"));
        assert_eq!(m.lookup_max_width("fraction"), Some("rems(0.5)"));
    }

    #[test]
    fn min_and_max_height_tables_load_too() {
        let m = parse_ok(
            r#"
            [max-height]
            "screen" = "100rem"
            [min-width]
            "card" = "20rem"
            [min-height]
            "field" = "2.5rem"
            "#,
        );
        assert_eq!(m.lookup_max_height("screen"), Some("rems(100.0)"));
        assert_eq!(m.lookup_min_width("card"), Some("rems(20.0)"));
        assert_eq!(m.lookup_min_height("field"), Some("rems(2.5)"));
    }

    #[test]
    fn malformed_toml_hard_fails() {
        let err = ThemeManifest::from_toml("[colors\nfoo = bar").unwrap_err();
        assert!(matches!(err, ManifestError::Parse(_)));
    }

    #[test]
    fn unsupported_length_value_rejects_with_section_and_key() {
        // `vh` / `%` / arbitrary CSS function values aren't lowered in v0.1.
        for value in ["100vh", "50%", "calc(100% - 16px)", "auto"] {
            let toml = format!("[max-width]\n\"foo\" = \"{value}\"");
            let err = ThemeManifest::from_toml(&toml).unwrap_err();
            match err {
                ManifestError::UnsupportedScaleValue {
                    section,
                    key,
                    value: got_value,
                } => {
                    assert_eq!(section, "max-width");
                    assert_eq!(key, "foo");
                    assert_eq!(got_value, value);
                }
                other => panic!("expected UnsupportedScaleValue for {value:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn negative_lengths_reject() {
        let err = ThemeManifest::from_toml(
            r#"[max-width]
"x" = "-1rem""#,
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::UnsupportedScaleValue { .. }));
    }

    #[test]
    fn unknown_sections_are_silently_ignored_for_forward_compat() {
        // A future manifest might add `[shadow]` or `[font]` tables.
        // Older parsers should tolerate them rather than refusing the
        // whole document.
        let m = parse_ok(
            r##"
            [colors]
            base = "#000"
            [shadow]
            card = "0 6px 20px rgba(0,0,0,0.20)"
            [font]
            sans = "Inter"
            "##,
        );
        // Known sections still load.
        assert!(m.knows_color("base"));
        // Unknown sections don't surface — they're forward-compat
        // breathing room.
    }
}
