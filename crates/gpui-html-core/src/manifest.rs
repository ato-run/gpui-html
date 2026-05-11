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

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

/// Parsed manifest. Hyphens in TOML keys are preserved as-is — the
/// caller looks them up by the *original* token name (`accent-foreground`),
/// not the snake-case Rust ident (`accent_foreground`). The
/// hyphen-to-snake normalization happens in `class_map.rs` after the
/// manifest validation accepts the token.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeManifest {
    /// Names that may appear after `bg-` / `text-` / `border-` and in
    /// CSS `var(--theme-X)`. Values from the TOML `[colors]` table are
    /// discarded in v0.1 — only the *key set* is used for validation.
    /// Color values become load-bearing once #23 (theme-token alpha)
    /// lands.
    colors: HashSet<String>,
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

        let colors: HashSet<String> = schema.colors.into_keys().collect();
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
        self.colors.contains(name)
    }

    /// The set of declared color names. Exposed for diagnostics that
    /// want to suggest the closest match on rejection.
    pub fn color_names(&self) -> impl Iterator<Item = &str> {
        self.colors.iter().map(String::as_str)
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
