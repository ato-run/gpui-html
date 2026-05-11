//! Class lowering (stage 2.5): `ClassToken` -> `MethodCall` IR.
//!
//! Currently lowered (growing toward the full spec table at
//! `docs/spec.md` § "class 対応範囲"):
//!
//! ```text
//! Flex direction:
//!   flex            -> .flex()
//!   flex-row        -> .flex_row()
//!   flex-col        -> .flex_col()
//!   flex-wrap       -> .flex_wrap()
//!   flex-nowrap     -> .flex_nowrap()
//!
//! Flex sizing:
//!   flex-1          -> .flex_1()
//!   flex-auto       -> .flex_auto()
//!   flex-none       -> .flex_none()
//!   grow            -> .flex_grow()
//!   shrink          -> .flex_shrink()
//!   shrink-0        -> .flex_shrink_0()
//!
//! Cross-axis alignment:
//!   items-start     -> .items_start()
//!   items-center    -> .items_center()
//!   items-end       -> .items_end()
//!   items-baseline  -> .items_baseline()
//!
//! Main-axis justification:
//!   justify-start   -> .justify_start()
//!   justify-center  -> .justify_center()
//!   justify-end     -> .justify_end()
//!   justify-between -> .justify_between()
//!   justify-around  -> .justify_around()
//!   justify-evenly  -> .justify_evenly()
//!
//! Spacing (all use the v0.1 spacing scale N ∈ {0..=12, 16, 20, 24, 32}):
//!   p-N             -> .p_N()
//!   px-N / py-N     -> .px_N() / .py_N()
//!   pt-N / pr-N / pb-N / pl-N
//!                   -> .pt_N() / .pr_N() / .pb_N() / .pl_N()
//!   m-N             -> .m_N()
//!   mx-N / my-N / mt-N / mr-N / mb-N / ml-N
//!   gap-N           -> .gap_N()
//!   gap-x-N / gap-y-N -> .gap_x_N() / .gap_y_N()
//!
//!   Negative margins (`-m-N`, `-mx-N`, ...) are禁止 in v0.1 (spec line 219).
//!
//! Size (numeric variants share the spacing scale; keywords/fractions
//! are flat identity maps):
//!   w-N / h-N / size-N
//!   min-w-N / min-h-N / max-w-N / max-h-N
//!   w-full / h-full / size-full
//!   w-auto / h-auto
//!   w-1/2 / w-1/3 / w-2/3 / w-3/4   (the four fractions the spec lists)
//!
//!   `w-screen` / `h-screen` and Tailwind-config-extended scales like
//!   `max-w-128` are NOT in the v0.1 Size section and therefore remain
//!   `UnknownClass` even though gpui's `Styled` trait would support
//!   them — opening that surface is a v0.2 concern.
//!
//! Typography:
//!   text-xs / sm / base / lg / xl / 2xl / 3xl
//!                   -> .text_xs() ... .text_3xl()
//!   font-thin / light / normal / medium / semibold / bold / extrabold / black
//!                   -> .font_weight(FontWeight::THIN) ... ::BLACK
//!   italic / not-italic / line-through / truncate
//!                   -> .italic() / .not_italic() / .line_through() / .truncate()
//!   leading-none / tight / snug / normal / relaxed / loose
//!                   -> .line_height(rems(<n>))
//!   line-clamp-N    -> .line_clamp(N)
//!
//!   Rejected with hint: `font-sans` / `font-mono` / `font-serif`
//!   (font-family is an app-shell concern, see #19), `whitespace-nowrap`,
//!   `whitespace-normal`, `text-ellipsis` (subsumed by `truncate`).
//!
//! Border (numeric widths share the v0.1 spacing scale):
//!   border             -> .border_1()                  (bare = 1px)
//!   border-N           -> .border_N()
//!   border-{t,r,b,l}   -> .border_{t,r,b,l}_1()        (bare = 1px,
//!                                                       same rule as bare 'border')
//!   border-{t,r,b,l}-N -> .border_{t,r,b,l}_N()
//!   border-dashed      -> .border_dashed()
//!   border-<token>     -> .border_color(theme.<token>)
//!
//!   Disambiguation: `border-N` is width (digits), `border-<ident>` is a
//!   theme color token. `border-border` is therefore the user's `theme.border`
//!   color, *not* a recursive width. The matcher tries width-shaped values
//!   first, then falls back to theme-token shape.
//!
//! Overflow / Cursor / Opacity:
//!   overflow-{hidden,visible,scroll}        -> .overflow_<v>()
//!   overflow-{x,y}-{hidden,scroll}          -> .overflow_<axis>_<v>()
//!   cursor-{default,pointer,text,move,grab,grabbing,not-allowed,
//!           col-resize,row-resize,ew-resize,ns-resize,nesw-resize,
//!           nwse-resize,crosshair,help,none}
//!                                            -> .cursor_<v>()
//!   opacity-N (N ∈ 0..=100)                 -> .opacity(N.0 / 100.0)
//!
//!   Rejected with hint: `overflow-{auto,x-auto,y-auto}` — gpui's
//!   Overflow enum has no `Auto` value (spec line 348).
//!
//! Color literals and theme tokens:
//!   bg-transparent  -> .bg(gpui::transparent_black())   (literal, NOT theme.transparent)
//!   bg-<token>      -> .bg(theme.<token>)
//!   text-<token>    -> .text_color(theme.<token>)
//!
//!   Theme-token alpha (`bg-<token>/<n>`, n ∈ 0..=100) lowers to a
//!   literal `gpui::rgba(0xRRGGBBAA)` ONLY when a manifest declares the
//!   token's color. The manifest's RGB becomes the upper three bytes
//!   and the slash alpha becomes the A byte (overriding any alpha the
//!   manifest's `#rrggbbaa` may have declared). Without a manifest the
//!   compiler can't know the token's color and rejects with a hint
//!   pointing at `--manifest`. Palette + alpha (`bg-red-500/80`)
//!   continues to reject even with a manifest — see #23.
//!
//! Radius:
//!   rounded                -> .rounded_md()             (bare = md)
//!   rounded-{none,sm,md,lg,xl,2xl,3xl,full}
//!                          -> .rounded_<suffix>()
//!   rounded-{t,r,b,l}-<suffix>
//!                          -> .rounded_<side>_<suffix>()
//!   rounded-{tl,tr,bl,br}-<suffix>
//!                          -> .rounded_<corner>_<suffix>()
//!
//!   Bare directional (`rounded-t`, `rounded-tl`, ...) rejects — the
//!   spec doesn't list a default-suffix behavior for directional radius.
//!
//! Shadow:
//!   shadow                 -> .shadow_md()              (bare = md)
//!   shadow-{sm,md,lg,xl,2xl,none}
//!                          -> .shadow_<suffix>()
//!
//!   `shadow-3xl` rejects (the spec stops shadow at 2xl).
//! ```
//!
//! Anything else surfaces as [`Error::UnknownClass`] with the offending
//! token's exact source span — the diagnostic schema (see
//! [`crate::diagnostic`]) carries that span so LLM consumers can self-correct.
//!
//! Theme `<token>` values are passed through symbolically: the compiler
//! does not validate that the field exists on the caller's `theme` struct —
//! that's deferred to rustc when the generated code is compiled. This is
//! the v0.1 contract; v0.2 will validate against a manifest.

use std::collections::HashMap;

use crate::ast::ClassToken;
use crate::Error;

/// `class name -> ordered MethodCalls from <style> rules`. Built by
/// the CSS pipeline (`crate::css::parse_and_lower`) and threaded
/// through codegen so element-level lowering can apply both utility
/// classes and stylesheet rules in one pass. An empty map is
/// equivalent to "no `<style>` blocks present" — that's the path
/// taken by callers that don't go through the document parser.
pub type StyleMap = HashMap<String, Vec<MethodCall>>;

/// One method call in the emitted gpui builder chain, e.g. `.gap_2()` or
/// `.bg(theme.surface)`. Args are stored as already-formatted Rust source
/// fragments; codegen splices them verbatim between the parens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodCall {
    pub name: String,
    pub args: Vec<String>,
}

impl MethodCall {
    pub(crate) fn nullary(name: &str) -> Self {
        MethodCall {
            name: name.to_string(),
            args: vec![],
        }
    }

    pub(crate) fn unary(name: &str, arg: String) -> Self {
        MethodCall {
            name: name.to_string(),
            args: vec![arg],
        }
    }
}

/// Lower a class attribute's tokens into the method-call sequence the
/// codegen will splice. Order is preserved so authors can rely on the
/// left-to-right precedence of gpui's builder (later calls override
/// earlier ones for conflicting Style fields).
///
/// Equivalent to [`lower_classes_with_styles`] called with an empty
/// `StyleMap` and no manifest — i.e. no `<style>` rules in the
/// document, no host theme manifest, only utility classes.
pub fn lower_classes(classes: &[ClassToken]) -> Result<Vec<MethodCall>, Error> {
    lower_classes_with_styles(classes, &StyleMap::new(), None)
}

/// Lower utility classes *and* `<style>` rule bindings in one pass,
/// optionally validating theme tokens / resolving custom-scale sizing
/// against a host [`ThemeManifest`].
///
/// **Order contract** (pinned by tests; see PR #27 for the rationale):
///
///   1. **Phase 1 — CSS rules**: for each class in source order, if the
///      stylesheet defined a rule for it, emit that rule's MethodCalls.
///   2. **Phase 2 — utility classes**: for each class in source order,
///      emit the utility lowering (`flex`, `gap-2`, …). Classes that
///      have a stylesheet rule but aren't a known utility are skipped
///      here rather than producing `UnknownClass` — Phase 1 already
///      handled them.
///
/// The net effect is "**utility class wins over stylesheet rule** when
/// both touch the same Style field" — gpui builder semantics make the
/// later `.gap_2()` override an earlier `.gap_4()` from a CSS rule.
/// The user-facing rationale is that `class="..."` reads as a local
/// override of any rule the class might also have in `<style>`.
///
/// When `manifest` is `Some`:
/// - `bg-X` / `text-X` / `border-X` and CSS `var(--theme-X)` validate
///   `X` against the manifest's `[colors]`. Unknown names surface as
///   `UnknownThemeToken` with a span.
/// - `max-w-<custom>` / `max-h-<custom>` / `min-w-<custom>` /
///   `min-h-<custom>` not on the v0.1 spacing scale resolve via the
///   manifest's corresponding sizing tables instead of rejecting.
///
/// When `manifest` is `None`, the v0.1 built-in defaults apply: theme
/// tokens pass through symbolically, only the `max-w-128` single
/// app-shell exemption resolves.
pub fn lower_classes_with_styles(
    classes: &[ClassToken],
    style_map: &StyleMap,
    manifest: Option<&crate::manifest::ThemeManifest>,
) -> Result<Vec<MethodCall>, Error> {
    let mut result: Vec<MethodCall> = Vec::new();

    // Phase 1: stylesheet rules.
    for cls in classes {
        if let Some(rule_calls) = style_map.get(&cls.raw) {
            result.extend(rule_calls.iter().cloned());
        }
    }

    // Phase 2: utility lowerings. Classes that are unknown utilities
    // but DO have a stylesheet rule (Phase 1 already handled them)
    // pass silently here. Recognized no-op classes (e.g. `font-sans`,
    // accepted for Tailwind preview compatibility but lowering to no
    // GPUI builder call — see #19) also pass silently: they appear in
    // source order but emit nothing.
    for cls in classes {
        match lower_one(cls, manifest) {
            Ok(Some(call)) => result.push(call),
            Ok(None) => {
                // Recognized no-op (e.g. font-sans). Source position is
                // honoured by the iteration but no MethodCall is emitted.
            }
            Err(Error::UnknownClass { .. }) if style_map.contains_key(&cls.raw) => {
                // Already covered by Phase 1; not really an unknown class.
            }
            Err(e) => return Err(e),
        }
    }

    Ok(result)
}

/// Lower one class to either a method call, a recognized no-op, or
/// an `UnknownClass` error.
///
/// `Ok(Some(call))` is the common case — the class has a builder
/// method to splice into the chain.
///
/// `Ok(None)` is a "recognized no-op": the class is explicitly accepted
/// (so callers don't get `UnknownClass`) but has no GPUI builder
/// equivalent. v0.1 uses this for `font-sans`, where font-family lives
/// at the gpui app/Theme level rather than per-element. See #19 for
/// the design discussion.
///
/// `manifest` enables host-side validation when present:
/// - `bg-X` / `text-X` / `border-X` whose `X` isn't declared in
///   `manifest.colors` surfaces as `UnknownThemeToken` with a span.
/// - `max-w-<custom>` / `max-h-<custom>` / `min-w-<custom>` /
///   `min-h-<custom>` not on the v0.1 spacing scale resolve via the
///   manifest's sizing tables instead of rejecting.
fn lower_one(
    tok: &ClassToken,
    manifest: Option<&crate::manifest::ThemeManifest>,
) -> Result<Option<MethodCall>, Error> {
    let raw = tok.raw.as_str();

    // Recognized no-op classes: accepted for Tailwind preview compat
    // (so callers don't get UnknownClass) but lower to no GPUI builder
    // call. See #19. Currently just `font-sans` — font-family is an
    // app-shell / Theme concern in gpui, not a per-element class.
    if is_noop_class(raw) {
        return Ok(None);
    }

    // Spec line 179-181: inline-flex and items-stretch are explicitly out
    // of scope (no matching `Styled` shorthand). Reject with a hint that
    // points back at the spec rationale so the diagnostic is actionable.
    if raw == "inline-flex" || raw == "items-stretch" {
        return Err(Error::UnknownClass {
            class: raw.to_string(),
            span: tok.span,
            hint: Some(
                "no matching `Styled` shorthand in gpui — \
                 spec rejects this; use Style.display / Style.align_items \
                 directly in Rust if you really need it."
                    .into(),
            ),
        });
    }

    // Spec line 219: negative margins (e.g. `-m-2`, `-mx-4`) are禁止
    // in v0.1. Catch them before prefix parsing so the diagnostic
    // explains *why* rather than getting a generic "unknown class".
    if is_negative_margin(raw) {
        return Err(Error::UnknownClass {
            class: raw.to_string(),
            span: tok.span,
            hint: Some(
                "negative margins are out of scope for v0.1 (spec line 219). \
                 The underlying gpui method (`m_neg_2()` etc.) exists, so \
                 future versions can open this up if needed."
                    .into(),
            ),
        });
    }

    if let Some(call) = lower_layout_keyword(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_spacing(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_sizing(raw, manifest) {
        return Ok(Some(call));
    }

    // Typography utilities the spec deliberately rejects (with reasons):
    // surface a focused hint instead of falling through to a generic
    // UnknownClass. Has to run *before* `lower_typography` because the
    // rejected names (`font-sans`, `text-ellipsis`) would otherwise just
    // miss every match arm and yield a hint-less error.
    if let Some(hint) = typography_rejection_hint(raw) {
        return Err(Error::UnknownClass {
            class: raw.to_string(),
            span: tok.span,
            hint: Some(hint),
        });
    }

    if let Some(call) = lower_typography(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_border(tok, manifest)? {
        return Ok(Some(call));
    }

    // Spec lines 348-352: gpui has no Overflow::Auto. Reject before
    // lower_overflow runs so the diagnostic explains the gpui-side
    // reason, not a generic UnknownClass.
    if matches!(raw, "overflow-auto" | "overflow-x-auto" | "overflow-y-auto") {
        return Err(Error::UnknownClass {
            class: raw.to_string(),
            span: tok.span,
            hint: Some(
                "gpui's Overflow enum has only {Visible, Hidden, Scroll} — no Auto. \
                 Use `overflow-{x|y}-scroll` for scrollable content (web's auto/scroll \
                 behave the same on macOS/Linux/Windows for native UI)."
                    .into(),
            ),
        });
    }

    if let Some(call) = lower_overflow(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_cursor(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_opacity(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_radius(raw) {
        return Ok(Some(call));
    }

    if let Some(call) = lower_shadow(raw) {
        return Ok(Some(call));
    }

    if let Some(rest) = raw.strip_prefix("bg-") {
        // Spec line 272: `bg-transparent` is a literal that lowers to
        // `gpui::transparent_black()`, NOT a theme.transparent lookup.
        // Has to come before the generic theme-token branch because
        // `transparent` happens to be a valid Rust identifier.
        if rest == "transparent" {
            return Ok(Some(MethodCall::unary(
                "bg",
                "gpui::transparent_black()".into(),
            )));
        }

        // Theme-token alpha (`bg-<token>/<n>`) — #23. The compiler can
        // only lower this when a manifest supplies the color value, so
        // the RGB bytes come from the manifest and the slash alpha
        // becomes the A channel of a packed `gpui::rgba(0xRRGGBBAA)`
        // literal. Without a manifest, the compiler doesn't know the
        // theme's colors and rejects with a hint pointing at
        // `--manifest`.
        //
        // Palette utilities with alpha (`bg-red-500/80`) don't match
        // this branch — the palette token shape is hyphen-separated
        // with a numeric trailing segment, which `normalize_theme_token`
        // rejects — so they fall through to the existing palette
        // rejection through `hint_for`.
        if let Some((token, alpha)) = rest.split_once('/') {
            if normalize_theme_token(token).is_some()
                && !alpha.is_empty()
                && alpha.chars().all(|c| c.is_ascii_digit())
            {
                let alpha_n: u32 = alpha.parse().unwrap_or(u32::MAX);
                if alpha_n > 100 {
                    return Err(Error::UnknownClass {
                        class: raw.to_string(),
                        span: tok.span,
                        hint: Some(
                            "theme-token alpha must be in the range 0..=100 \
                             (Tailwind's opacity scale)."
                                .into(),
                        ),
                    });
                }
                let Some(m) = manifest else {
                    return Err(Error::UnknownClass {
                        class: raw.to_string(),
                        span: tok.span,
                        hint: Some(
                            "theme-token alpha (`bg-<token>/<n>`) needs a theme \
                             manifest so the compiler can read the token's RGB \
                             value. Pass `--manifest <path>` with a `[colors]` \
                             table entry for the token, or drop the `/<n>` \
                             suffix and use `bg-<token>` (the host theme owns \
                             the color at runtime)."
                                .into(),
                        ),
                    });
                };
                let Some(rgba) = m.lookup_color_rgba(token) else {
                    return Err(Error::UnknownThemeToken {
                        token: token.to_string(),
                        span: tok.span,
                    });
                };
                // Slash alpha overrides whatever alpha the manifest's
                // `#rrggbbaa` declared — `bg-accent/50` means 50% alpha
                // regardless of accent's manifest alpha byte.
                let alpha_byte = ((alpha_n * 255 + 50) / 100) as u8;
                let packed = u32::from_be_bytes([rgba[0], rgba[1], rgba[2], alpha_byte]);
                return Ok(Some(MethodCall::unary(
                    "bg",
                    format!("gpui::rgba(0x{packed:08x})"),
                )));
            }
        }

        if let Some(normalized) = normalize_theme_token(rest) {
            validate_theme_token(rest, tok.span, manifest)?;
            return Ok(Some(MethodCall::unary("bg", format!("theme.{normalized}"))));
        }
    }

    if let Some(token) = raw.strip_prefix("text-") {
        // text-xs..text-3xl are typography sizes (handled by
        // `lower_typography` above). Anything else under `text-` is a
        // color theme token — possibly hyphenated, normalized to
        // snake_case for the Rust field access. When a manifest is
        // supplied, the original (hyphenated) token is validated
        // against the host's `[colors]` table.
        if let Some(normalized) = normalize_theme_token(token) {
            validate_theme_token(token, tok.span, manifest)?;
            return Ok(Some(MethodCall::unary(
                "text_color",
                format!("theme.{normalized}"),
            )));
        }
    }

    Err(Error::UnknownClass {
        class: raw.to_string(),
        span: tok.span,
        hint: hint_for(raw),
    })
}

/// Map a Tailwind layout keyword (no prefix-and-value structure — flat
/// identity mapping) to its gpui builder method. Returns `None` if the
/// raw token isn't one of the layout keywords this PR covers; the caller
/// then falls through to prefix-based lowering (gap-, p-, bg-, text-).
///
/// Centralising the table here means adding a new utility is a one-line
/// match arm and the doc comment at the top of the module stays the
/// single source of truth for what's lowered.
fn lower_layout_keyword(raw: &str) -> Option<MethodCall> {
    let method = match raw {
        // Flex direction
        "flex" => "flex",
        "flex-row" => "flex_row",
        "flex-col" => "flex_col",
        "flex-wrap" => "flex_wrap",
        "flex-nowrap" => "flex_nowrap",

        // Flex sizing
        "flex-1" => "flex_1",
        "flex-auto" => "flex_auto",
        "flex-none" => "flex_none",
        "grow" => "flex_grow",
        "shrink" => "flex_shrink",
        "shrink-0" => "flex_shrink_0",

        // Cross-axis alignment
        "items-start" => "items_start",
        "items-center" => "items_center",
        "items-end" => "items_end",
        "items-baseline" => "items_baseline",

        // Main-axis justification
        "justify-start" => "justify_start",
        "justify-center" => "justify_center",
        "justify-end" => "justify_end",
        "justify-between" => "justify_between",
        "justify-around" => "justify_around",
        "justify-evenly" => "justify_evenly",

        _ => return None,
    };
    Some(MethodCall::nullary(method))
}

/// Lower a Tailwind spacing utility (padding, margin, or gap, with
/// optional axis/side suffix) to the matching gpui builder method.
///
/// All entries share the same shape: `<prefix>-<n>` (or `<prefix>-x-<n>`
/// for axis variants) where `<n>` is the spacing scale (0..=12 in v0.1).
/// Returns `None` when no prefix matches; returns `None` (not an error)
/// when a prefix matches but the value isn't a valid scale step, so
/// `m-foo` falls through to the generic `UnknownClass` path with the
/// same shape as `gap-13` did before this PR.
///
/// Prefix-search ordering matters: longer/more-specific prefixes must
/// come first (e.g. `px-` before `p-`, `gap-x-` before `gap-`) so a
/// utility like `gap-x-2` doesn't get matched by the bare `gap-` and
/// then fail on the value `x-2`.
fn lower_spacing(raw: &str) -> Option<MethodCall> {
    const PREFIXES: &[(&str, &str)] = &[
        // Padding — directional first, then bare
        ("px-", "px_"),
        ("py-", "py_"),
        ("pt-", "pt_"),
        ("pr-", "pr_"),
        ("pb-", "pb_"),
        ("pl-", "pl_"),
        ("p-", "p_"),
        // Margin — same order
        ("mx-", "mx_"),
        ("my-", "my_"),
        ("mt-", "mt_"),
        ("mr-", "mr_"),
        ("mb-", "mb_"),
        ("ml-", "ml_"),
        ("m-", "m_"),
        // Gap — axis variants before bare
        ("gap-x-", "gap_x_"),
        ("gap-y-", "gap_y_"),
        ("gap-", "gap_"),
    ];
    for (prefix, method_prefix) in PREFIXES {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return parse_spacing_step(rest)
                .map(|n| MethodCall::nullary(&format!("{method_prefix}{n}")));
        }
    }
    None
}

/// Is this class name shaped like a Tailwind negative margin?
/// (`-m-2`, `-mx-4`, `-mt-1`, etc.) These are explicitly禁止 in v0.1
/// (spec line 219); detecting them here lets the diagnostic explain
/// *why* rather than reporting a generic UnknownClass.
fn is_negative_margin(raw: &str) -> bool {
    const NEG_MARGIN_PREFIXES: &[&str] = &["-m-", "-mx-", "-my-", "-mt-", "-mr-", "-mb-", "-ml-"];
    NEG_MARGIN_PREFIXES.iter().any(|p| raw.starts_with(p))
}

/// Lower a Tailwind sizing utility (width / height / size, with optional
/// min/max prefix) to the matching gpui builder method. Tries the flat
/// keyword/fraction map first (`w-full`, `w-1/2`, etc.) so prefix
/// matching can't accidentally swallow them, then falls back to the
/// numeric prefix table.
///
/// Out-of-spec numeric prefixes (`max-w-128`) and viewport keywords
/// (`w-screen` / `h-screen`) are deliberately NOT handled here — see the
/// module-level doc comment.
fn lower_sizing(
    raw: &str,
    manifest: Option<&crate::manifest::ThemeManifest>,
) -> Option<MethodCall> {
    if let Some(call) = lower_sizing_keyword(raw) {
        return Some(call);
    }

    // Manifest-provided custom scales for min/max width and height
    // take priority. The manifest stores pre-formatted `rems(N.0)`
    // strings so codegen can splice them verbatim.
    if let Some(m) = manifest {
        if let Some(suffix) = raw.strip_prefix("max-w-") {
            if let Some(value) = m.lookup_max_width(suffix) {
                return Some(MethodCall::unary("max_w", value.to_string()));
            }
        }
        if let Some(suffix) = raw.strip_prefix("max-h-") {
            if let Some(value) = m.lookup_max_height(suffix) {
                return Some(MethodCall::unary("max_h", value.to_string()));
            }
        }
        if let Some(suffix) = raw.strip_prefix("min-w-") {
            if let Some(value) = m.lookup_min_width(suffix) {
                return Some(MethodCall::unary("min_w", value.to_string()));
            }
        }
        if let Some(suffix) = raw.strip_prefix("min-h-") {
            if let Some(value) = m.lookup_min_height(suffix) {
                return Some(MethodCall::unary("min_h", value.to_string()));
            }
        }
    }

    // App-shell compatibility (#19): when no manifest declares a
    // `max-w-128` entry, the built-in single-token exemption still
    // resolves it. The Ato Desktop preview's Tailwind config defines
    // `128 = 32rem` and the fixture relies on this without needing a
    // manifest at all. Once a manifest is supplied, the manifest
    // takes precedence (handled above) — including overriding the
    // built-in 32rem if the host wants a different value.
    if raw == "max-w-128" {
        return Some(MethodCall::unary("max_w", "rems(32.0)".into()));
    }

    // Numeric prefixes — longest-first within each axis so `min-w-` and
    // `max-w-` are tried before bare `w-`, and `size-` before `s-`-shaped
    // future prefixes (none exist today; ordering is defensive).
    const PREFIXES: &[(&str, &str)] = &[
        ("min-w-", "min_w_"),
        ("min-h-", "min_h_"),
        ("max-w-", "max_w_"),
        ("max-h-", "max_h_"),
        ("size-", "size_"),
        ("w-", "w_"),
        ("h-", "h_"),
    ];
    for (prefix, method_prefix) in PREFIXES {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return parse_spacing_step(rest)
                .map(|n| MethodCall::nullary(&format!("{method_prefix}{n}")));
        }
    }
    None
}

/// Flat identity map for the non-numeric sizing utilities the spec
/// enumerates: `w-full` / `h-full` / `size-full`, `w-auto` / `h-auto`,
/// the four fractional widths, and (added in #19) the viewport keywords
/// `w-screen` / `h-screen` / `size-screen`.
///
/// Tokenizer note: `w-1/2` arrives here as a single class token because
/// `split_classes` only splits on whitespace. The `/` is preserved
/// verbatim, which is what makes the literal match below work — see
/// `fractional_width_token_survives_tokenizer` below.
fn lower_sizing_keyword(raw: &str) -> Option<MethodCall> {
    let method = match raw {
        "w-full" => "w_full",
        "h-full" => "h_full",
        "size-full" => "size_full",
        "w-auto" => "w_auto",
        "h-auto" => "h_auto",
        // Viewport keywords (#19) — gpui's `Styled` exposes
        // `w_screen()` / `h_screen()` / `size_screen()` directly.
        "w-screen" => "w_screen",
        "h-screen" => "h_screen",
        "size-screen" => "size_screen",
        "w-1/2" => "w_1_2",
        "w-1/3" => "w_1_3",
        "w-2/3" => "w_2_3",
        "w-3/4" => "w_3_4",
        _ => return None,
    };
    Some(MethodCall::nullary(method))
}

/// Lower a Tailwind typography utility to its gpui builder method.
/// Covers text size, font weight, decoration, line-height, line-clamp,
/// and the standalone keywords (`italic`, `truncate`, ...).
///
/// `text-<size>` returns here only for the seven sizes the spec lists
/// (`xs`, `sm`, `base`, `lg`, `xl`, `2xl`, `3xl`); any other `text-…`
/// shape falls through, so the color theme-token path can claim it
/// downstream. That ordering is the disambiguation contract:
/// **typography size wins over theme color** when the suffix collides.
fn lower_typography(raw: &str) -> Option<MethodCall> {
    // text-<size> → .text_<size>()
    if let Some(suffix) = raw.strip_prefix("text-") {
        if is_typography_size(suffix) {
            return Some(MethodCall::nullary(&format!("text_{suffix}")));
        }
    }

    // font-<weight> → .font_weight(FontWeight::<VARIANT>)
    if let Some(weight) = font_weight_variant(raw) {
        return Some(MethodCall::unary(
            "font_weight",
            format!("FontWeight::{weight}"),
        ));
    }

    // Flat keyword utilities
    let nullary_method = match raw {
        "italic" => "italic",
        "not-italic" => "not_italic",
        "line-through" => "line_through",
        "truncate" => "truncate",
        _ => return lower_typography_argful(raw),
    };
    Some(MethodCall::nullary(nullary_method))
}

/// Tail of `lower_typography` for the utilities that produce a unary
/// builder call with a non-trivial Rust expression argument
/// (`leading-*`, `line-clamp-N`). Split out to keep the main function
/// flat and easy to scan against the spec table.
fn lower_typography_argful(raw: &str) -> Option<MethodCall> {
    // leading-<keyword> → .line_height(rems(<n>))
    let line_height_arg = match raw {
        "leading-none" => "rems(1.0)",
        "leading-tight" => "rems(1.25)",
        "leading-snug" => "rems(1.375)",
        "leading-normal" => "rems(1.5)",
        "leading-relaxed" => "rems(1.625)",
        "leading-loose" => "rems(2.0)",
        _ => "",
    };
    if !line_height_arg.is_empty() {
        return Some(MethodCall::unary("line_height", line_height_arg.into()));
    }

    // line-clamp-<n> → .line_clamp(<n>); reject 0 (semantically nonsense)
    if let Some(rest) = raw.strip_prefix("line-clamp-") {
        if let Ok(n) = rest.parse::<u32>() {
            if n > 0 {
                return Some(MethodCall::unary("line_clamp", n.to_string()));
            }
        }
    }

    None
}

fn font_weight_variant(raw: &str) -> Option<&'static str> {
    let variant = match raw {
        "font-thin" => "THIN",
        "font-light" => "LIGHT",
        "font-normal" => "NORMAL",
        "font-medium" => "MEDIUM",
        "font-semibold" => "SEMIBOLD",
        "font-bold" => "BOLD",
        "font-extrabold" => "EXTRA_BOLD",
        "font-black" => "BLACK",
        _ => return None,
    };
    Some(variant)
}

/// Typography classes the spec deliberately rejects (lines 320-329)
/// plus font-family utilities (deferred per #19). Returning `Some(_)`
/// short-circuits to a structured `UnknownClass` carrying the hint;
/// returning `None` lets the rest of the lowering pipeline run.
fn typography_rejection_hint(raw: &str) -> Option<String> {
    match raw {
        "whitespace-nowrap" | "whitespace-normal" => Some(
            "no `Styled` shorthand for whitespace utilities — write to \
             `Style.text` directly, or use `truncate` for single-line \
             truncation."
                .into(),
        ),
        "text-ellipsis" => Some(
            "no standalone `text_ellipsis()` method in gpui; `truncate` \
             already handles ellipsis behavior. Use `truncate` instead."
                .into(),
        ),
        "font-mono" | "font-serif" => Some(
            "font-family utilities are not lowered in v0.1 — font-family \
             lives at the gpui app/Theme level, not per-element. \
             `font-sans` is accepted as a recognized no-op (#19) for \
             Tailwind preview compatibility; `font-mono`/`font-serif` \
             are not currently exempt. Configure the host app's font \
             stack instead."
                .into(),
        ),
        _ => None,
    }
}

/// Lower a Tailwind border utility (width / directional width / dashed /
/// theme-color) to its gpui builder method.
///
/// Disambiguation precedence after the bare keyword and style cases:
///   1. directional + numeric (`border-t-2`)
///   2. directional bare (`border-t`)        → spec extension, see PR/below
///   3. plain numeric (`border-12`)          → width
///   4. theme token (`border-border`)        → color
///
/// Bare directional handling — `border-t`, `border-r`, `border-b`,
/// `border-l` (no number) — is a small *spec interpretation* rather
/// than a literal reading: the spec lists `border` (no suffix) → 1px
/// and `border-<side>-<n>` (with a number) explicitly, but the standard
/// Tailwind/preview UI convention is that bare directional shorthands
/// also resolve to 1px on that side. We extend the "bare = 1px" rule
/// from `border` to the directional variants because (a) gpui's `Styled`
/// already exposes `border_t_1`/`border_r_1`/`border_b_1`/`border_l_1`,
/// (b) the Ato Desktop preview fixture in #9 relies on this, and
/// (c) refusing it would make the diagnostic misleading
/// (`border-b` would lower as `border_color(theme.b)`, which is even
/// worse than UnknownClass). PR description calls this out for review.
fn lower_border(
    tok: &ClassToken,
    manifest: Option<&crate::manifest::ThemeManifest>,
) -> Result<Option<MethodCall>, Error> {
    let raw = tok.raw.as_str();
    // Bare keyword and style keyword cases short-circuit before any
    // prefix manipulation.
    if raw == "border" {
        return Ok(Some(MethodCall::nullary("border_1")));
    }
    if raw == "border-dashed" {
        return Ok(Some(MethodCall::nullary("border_dashed")));
    }

    let Some(rest) = raw.strip_prefix("border-") else {
        return Ok(None);
    };

    // Directional cases: bare side (`t` / `r` / `b` / `l`) and side+N
    // (`t-2`, `b-3`, ...). These must come before the plain numeric and
    // theme-token branches so `border-b` doesn't get claimed as a theme
    // color named `b`.
    for side in ["t", "r", "b", "l"] {
        if rest == side {
            return Ok(Some(MethodCall::nullary(&format!("border_{side}_1"))));
        }
        if let Some(num_str) = rest.strip_prefix(&format!("{side}-")) {
            return Ok(parse_spacing_step(num_str)
                .map(|n| MethodCall::nullary(&format!("border_{side}_{n}"))));
        }
    }

    // Plain numeric width: `border-N`.
    if let Some(n) = parse_spacing_step(rest) {
        return Ok(Some(MethodCall::nullary(&format!("border_{n}"))));
    }

    // Theme color: `border-<token>`. Hyphenated multi-word tokens
    // (`border-accent-foreground`) normalize to snake_case for the
    // Rust field name. Palette shapes (`border-red-500`) reject via
    // `normalize_theme_token`'s numeric-segment guard. When a
    // manifest is supplied, the original token (hyphenated form)
    // is validated against the host's [colors] table.
    if let Some(normalized) = normalize_theme_token(rest) {
        validate_theme_token(rest, tok.span, manifest)?;
        return Ok(Some(MethodCall::unary(
            "border_color",
            format!("theme.{normalized}"),
        )));
    }

    Ok(None)
}

/// When a manifest is provided, ensure the token name appears in the
/// host's `[colors]` table. The manifest stores token names in their
/// **original** (hyphenated, pre-normalization) form, so this check
/// uses the hyphenated source string, not the snake_case Rust ident.
fn validate_theme_token(
    name: &str,
    span: crate::ast::Span,
    manifest: Option<&crate::manifest::ThemeManifest>,
) -> Result<(), Error> {
    if let Some(m) = manifest {
        if !m.knows_color(name) {
            return Err(Error::UnknownThemeToken {
                token: name.to_string(),
                span,
            });
        }
    }
    Ok(())
}

/// Lower a Tailwind overflow utility to its gpui builder method.
/// Spec lines 336-342 enumerate exactly the seven supported variants;
/// `overflow-auto` and friends are rejected upstream in `lower_one`
/// because gpui's `Overflow` enum has no `Auto` value.
fn lower_overflow(raw: &str) -> Option<MethodCall> {
    let method = match raw {
        "overflow-hidden" => "overflow_hidden",
        "overflow-visible" => "overflow_visible",
        "overflow-scroll" => "overflow_scroll",
        "overflow-x-hidden" => "overflow_x_hidden",
        "overflow-y-hidden" => "overflow_y_hidden",
        "overflow-x-scroll" => "overflow_x_scroll",
        "overflow-y-scroll" => "overflow_y_scroll",
        _ => return None,
    };
    Some(MethodCall::nullary(method))
}

/// Lower a Tailwind cursor utility to its gpui builder method.
/// Spec lines 363-373 enumerate the v0.1-supported cursor names.
fn lower_cursor(raw: &str) -> Option<MethodCall> {
    let suffix = raw.strip_prefix("cursor-")?;
    let method_suffix = match suffix {
        "default" => "default",
        "pointer" => "pointer",
        "text" => "text",
        "move" => "move",
        "grab" => "grab",
        "grabbing" => "grabbing",
        "not-allowed" => "not_allowed",
        "col-resize" => "col_resize",
        "row-resize" => "row_resize",
        "ew-resize" => "ew_resize",
        "ns-resize" => "ns_resize",
        "nesw-resize" => "nesw_resize",
        "nwse-resize" => "nwse_resize",
        "crosshair" => "crosshair",
        "help" => "help",
        "none" => "none",
        _ => return None,
    };
    Some(MethodCall::nullary(&format!("cursor_{method_suffix}")))
}

/// Lower a Tailwind opacity utility to `.opacity(<f>)`.
///
/// Spec line 361: `opacity-<n>` for `n ∈ 0..=100`. The lowered argument
/// is rendered as `<n>.0 / 100.0` (a constant-folded f32 expression
/// rather than a pre-computed decimal) to keep the Rust source readable
/// and to avoid format-precision bikeshed: `opacity-50` → `50.0 / 100.0`.
fn lower_opacity(raw: &str) -> Option<MethodCall> {
    let n_str = raw.strip_prefix("opacity-")?;
    let n: u32 = n_str.parse().ok()?;
    if n > 100 {
        return None;
    }
    Some(MethodCall::unary("opacity", format!("{n}.0 / 100.0")))
}

/// Lower a Tailwind border-radius utility to its gpui builder method.
/// Spec lines 253-257 enumerate the bare keyword (= md) plus named
/// suffixes plus directional/corner variants.
///
/// Disambiguation: corner prefixes (`tl-`, `tr-`, `bl-`, `br-`) must
/// be tried before side prefixes (`t-`, `r-`, `b-`, `l-`) so
/// `rounded-tl-md` is parsed as a corner, not as side `t` with suffix
/// `l-md`. Implemented by listing corners first in the SIDES table.
fn lower_radius(raw: &str) -> Option<MethodCall> {
    if raw == "rounded" {
        return Some(MethodCall::nullary("rounded_md"));
    }

    let rest = raw.strip_prefix("rounded-")?;

    const SIDES: &[(&str, &str)] = &[
        // Corners first (longest-prefix wins)
        ("tl-", "tl_"),
        ("tr-", "tr_"),
        ("bl-", "bl_"),
        ("br-", "br_"),
        // Sides
        ("t-", "t_"),
        ("r-", "r_"),
        ("b-", "b_"),
        ("l-", "l_"),
    ];
    for (prefix, method_prefix) in SIDES {
        if let Some(suffix) = rest.strip_prefix(prefix) {
            return radius_suffix_to_method_suffix(suffix)
                .map(|m| MethodCall::nullary(&format!("rounded_{method_prefix}{m}")));
        }
    }

    // Plain suffix: rounded-<suffix>
    radius_suffix_to_method_suffix(rest).map(|m| MethodCall::nullary(&format!("rounded_{m}")))
}

/// Map a radius suffix string to its method-name suffix. Returns
/// `None` for any suffix outside the spec-enumerated set, including
/// the empty string (so a bare directional like `rounded-t` rejects).
fn radius_suffix_to_method_suffix(suffix: &str) -> Option<&'static str> {
    let m = match suffix {
        "none" => "none",
        "sm" => "sm",
        "md" => "md",
        "lg" => "lg",
        "xl" => "xl",
        "2xl" => "2xl",
        "3xl" => "3xl",
        "full" => "full",
        _ => return None,
    };
    Some(m)
}

/// Lower a Tailwind shadow utility to its gpui builder method.
/// Spec line 260 lists `sm / md / lg / xl / 2xl / none` (no `3xl`),
/// and the bare `shadow` resolves to `shadow_md` per line 259.
fn lower_shadow(raw: &str) -> Option<MethodCall> {
    if raw == "shadow" {
        return Some(MethodCall::nullary("shadow_md"));
    }
    let suffix = raw.strip_prefix("shadow-")?;
    let method_suffix = match suffix {
        "sm" => "sm",
        "md" => "md",
        "lg" => "lg",
        "xl" => "xl",
        "2xl" => "2xl",
        "none" => "none",
        _ => return None,
    };
    Some(MethodCall::nullary(&format!("shadow_{method_suffix}")))
}

/// v0.1 spacing scale per `docs/spec.md`: contiguous 0..=12 plus the
/// jump steps 16, 20, 24, 32. The Border section (spec line 248)
/// enumerates this set explicitly for `border-<n>`, and the Size
/// section (line 201) declares "spacing scale と同じ" while explicitly
/// forbidding 13/14/15 — the only consistent reading is one shared
/// scale with a gap from 12 to 16.
///
/// The gpui `Styled` trait exposes the matching builder methods
/// (`p_16`, `p_20`, `p_24`, `p_32`, etc.); higher Tailwind steps
/// (40, 48, 56, …, 96) are intentionally out of v0.1 scope and remain
/// `UnknownClass` until the spec opens them up.
fn parse_spacing_step(rest: &str) -> Option<u32> {
    let n: u32 = rest.parse().ok()?;
    if n <= 12 || n == 16 || n == 20 || n == 24 || n == 32 {
        Some(n)
    } else {
        None
    }
}

/// Normalize a Tailwind-style theme-token name to the Rust field name
/// the codegen will splice into `theme.<...>`.
///
/// Accepts hyphenated multi-word tokens (`accent-foreground`,
/// `muted-foreground`, `primary-hover`) and converts hyphens to
/// underscores. Each hyphen-separated segment must be ident-shaped
/// (`[A-Za-z_][A-Za-z0-9_]*`). Returns `None` if any segment is empty
/// or starts with a digit, *or* if any segment is purely numeric — the
/// purely-numeric-last-segment shape is what makes Tailwind palette
/// utilities (`red-500`, `slate-200`) palette utilities, and v0.1
/// rejects those: the host theme is the source of truth, palette
/// literals are out of scope.
///
/// Examples:
///   `accent`              → `Some("accent")`
///   `accent-foreground`   → `Some("accent_foreground")`
///   `red-500`             → `None`        (palette: numeric segment)
///   `2accent`             → `None`        (segment starts with digit)
///   ``                    → `None`        (empty)
///   `accent-`             → `None`        (empty trailing segment)
///
/// Used by both the utility-class path here and the CSS
/// `var(--theme-X)` path in [`crate::css`] so the two surfaces apply
/// identical disambiguation.
pub(crate) fn normalize_theme_token(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let segments: Vec<&str> = raw.split('-').collect();
    for seg in &segments {
        if seg.is_empty() {
            return None;
        }
        // Each segment must be ident-shaped on its own.
        let mut chars = seg.chars();
        let first = chars.next().unwrap();
        if !(first.is_ascii_alphabetic() || first == '_') {
            return None;
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        // A purely-numeric segment marks the palette pattern. Since
        // each segment must already pass the ident-shape check above
        // (which forbids leading digits), this only matters once we
        // change the ident rule — but it's free defense in depth.
        if seg.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    Some(segments.join("_"))
}

// Note: an `is_theme_token` (single-segment, no-hyphens) check used to
// live here. After #7 every call site moved to `normalize_theme_token`
// (which subsumes the single-segment case), so the narrower variant
// fell out of use and was removed. If a future caller really needs
// "is this a plain ident, not a hyphenated multi-word token",
// reintroduce it then — keeping dead code for hypothetical needs is
// just churn.

fn is_typography_size(s: &str) -> bool {
    matches!(s, "xs" | "sm" | "base" | "lg" | "xl" | "2xl" | "3xl")
}

/// Detect the Tailwind palette-utility shape (`<color>-<number>`,
/// optionally with a `/<alpha>` opacity suffix).
/// True when the input has at least two hyphen-separated segments and
/// the last segment (before any `/alpha`) is purely numeric — that's
/// what distinguishes `bg-red-500` (palette) from `bg-accent-foreground`
/// (theme token), and lets `bg-red-500/80` still match the palette
/// pattern even with the slash suffix attached.
fn looks_like_palette(rest: &str) -> bool {
    // Strip optional `/<alpha>` suffix so the palette check looks at
    // `red-500` regardless of whether `/80` was appended.
    let core = rest.split('/').next().unwrap_or(rest);
    let segments: Vec<&str> = core.split('-').collect();
    if segments.len() < 2 {
        return false;
    }
    let last = segments.last().unwrap();
    !last.is_empty() && last.chars().all(|c| c.is_ascii_digit())
}

/// Best-effort hint shown on the LLM-facing diagnostic when a class is
/// rejected. Kept terse — the diagnostic itself already carries the span,
/// the literal, and the error code.
fn hint_for(raw: &str) -> Option<String> {
    // Palette-shape rejection across bg/text/border. The hint is
    // shared because the underlying reason is the same on every
    // prefix: v0.1 doesn't ship a Tailwind palette, only theme
    // tokens against the host's `theme` struct.
    for prefix in ["bg-", "text-", "border-"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            if looks_like_palette(rest) {
                return Some(format!(
                    "v0.1 only allows `{prefix}<token>` against the caller's theme; \
                     palette utilities like `{prefix}red-500` are out of scope."
                ));
            }
        }
    }

    // Custom-scale sizing tokens (e.g. `max-w-200`, `max-w-card`) — the
    // v0.1 spacing scale is fixed at {0..=12, 16, 20, 24, 32}, with
    // `max-w-128` exempted as the single app-shell compatibility token
    // (#19). Anything else under these prefixes that didn't match the
    // numeric scale gets a hint pointing at the v0.2 manifest direction.
    for prefix in ["max-w-", "max-h-", "min-w-", "min-h-"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            if !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Some(format!(
                    "custom sizing tokens (`{prefix}<custom>`) require the v0.2 \
                     theme manifest. Use the v0.1 spacing scale ({prefix}N for \
                     N ∈ {{0..=12, 16, 20, 24, 32}}) — `max-w-128` is the only \
                     app-shell compatibility exemption (see #19)."
                ));
            }
        }
    }

    None
}

/// Recognized no-op classes — accepted for Tailwind preview compatibility
/// (so callers don't get UnknownClass) but lower to no GPUI builder call.
/// v0.1 currently exempts only `font-sans`. See #19.
fn is_noop_class(raw: &str) -> bool {
    matches!(raw, "font-sans")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

    fn tok(raw: &str) -> ClassToken {
        ClassToken {
            raw: raw.to_string(),
            span: Span::new(0, raw.len()),
        }
    }

    #[test]
    fn lower_flex_and_flex_col() {
        let out = lower_classes(&[tok("flex"), tok("flex-col")]).unwrap();
        assert_eq!(
            out,
            vec![MethodCall::nullary("flex"), MethodCall::nullary("flex_col")]
        );
    }

    #[test]
    fn lower_gap_and_p_in_range() {
        for n in 0u32..=12 {
            let raw = format!("gap-{n}");
            let out = lower_classes(&[tok(&raw)]).unwrap();
            assert_eq!(out, vec![MethodCall::nullary(&format!("gap_{n}"))]);

            let raw = format!("p-{n}");
            let out = lower_classes(&[tok(&raw)]).unwrap();
            assert_eq!(out, vec![MethodCall::nullary(&format!("p_{n}"))]);
        }
    }

    #[test]
    fn gap_out_of_range_is_unknown() {
        let err = lower_classes(&[tok("gap-13")]).unwrap_err();
        match err {
            Error::UnknownClass { class, .. } => assert_eq!(class, "gap-13"),
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn gap_negative_is_unknown() {
        let err = lower_classes(&[tok("gap--1")]).unwrap_err();
        assert!(matches!(err, Error::UnknownClass { .. }));
    }

    #[test]
    fn bg_theme_token_passthrough() {
        let out = lower_classes(&[tok("bg-surface")]).unwrap();
        assert_eq!(out, vec![MethodCall::unary("bg", "theme.surface".into())]);
    }

    #[test]
    fn text_theme_token_passthrough() {
        let out = lower_classes(&[tok("text-muted")]).unwrap();
        assert_eq!(
            out,
            vec![MethodCall::unary("text_color", "theme.muted".into())]
        );
    }

    #[test]
    fn bg_palette_utility_is_rejected_with_hint() {
        let err = lower_classes(&[tok("bg-red-500")]).unwrap_err();
        match err {
            Error::UnknownClass { class, hint, .. } => {
                assert_eq!(class, "bg-red-500");
                assert!(hint.is_some(), "palette utilities should carry a hint");
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    // Note: the previous `text_typography_utility_is_rejected_with_hint`
    // test pinned the v0.1 vertical-slice behavior where `text-xs..3xl`
    // were rejected with a "deferred" hint. With #13 (this PR's
    // landing), those *do* lower — see `lower_text_size_utilities`
    // below. The old test is intentionally removed, not skipped, so the
    // contract change is explicit in the diff.

    #[test]
    fn unknown_class_carries_token_span() {
        let t = ClassToken {
            raw: "wat".to_string(),
            span: Span::new(42, 45),
        };
        let err = lower_classes(&[t]).unwrap_err();
        match err {
            Error::UnknownClass { span, .. } => {
                assert_eq!(span, Span::new(42, 45));
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn empty_token_after_prefix_is_rejected() {
        let err = lower_classes(&[tok("bg-")]).unwrap_err();
        assert!(matches!(err, Error::UnknownClass { .. }));
    }

    // (Removed in #7: `hyphenated_theme_token_is_rejected` pinned the
    // pre-#7 contract where theme tokens were strictly single-segment
    // idents. After #7, `bg-some-color` normalizes to
    // `theme.some_color` — see `lower_hyphenated_bg_theme_token` below.)

    // ---------- issue #11: layout completion ------------------------------

    fn lowered_method_names(classes: &[&str]) -> Vec<String> {
        let toks: Vec<ClassToken> = classes.iter().copied().map(tok).collect();
        let calls = lower_classes(&toks).expect("layout classes should lower cleanly");
        // Layout utilities are all nullary; the test asserts that and
        // collects the method names so the assertion at the call site
        // reads as one flat list.
        calls
            .into_iter()
            .map(|c| {
                assert!(
                    c.args.is_empty(),
                    "layout utility unexpectedly emitted args: {:?}",
                    c
                );
                c.name
            })
            .collect()
    }

    #[test]
    fn lower_flex_direction_utilities() {
        assert_eq!(
            lowered_method_names(&["flex", "flex-row", "flex-col", "flex-wrap", "flex-nowrap"]),
            vec!["flex", "flex_row", "flex_col", "flex_wrap", "flex_nowrap"],
        );
    }

    #[test]
    fn lower_flex_sizing_utilities() {
        assert_eq!(
            lowered_method_names(&[
                "flex-1",
                "flex-auto",
                "flex-none",
                "grow",
                "shrink",
                "shrink-0",
            ]),
            vec![
                "flex_1",
                "flex_auto",
                "flex_none",
                "flex_grow",
                "flex_shrink",
                "flex_shrink_0",
            ],
        );
    }

    #[test]
    fn lower_items_alignment_utilities() {
        assert_eq!(
            lowered_method_names(&["items-start", "items-center", "items-end", "items-baseline"]),
            vec!["items_start", "items_center", "items_end", "items_baseline",],
        );
    }

    #[test]
    fn lower_justify_utilities() {
        assert_eq!(
            lowered_method_names(&[
                "justify-start",
                "justify-center",
                "justify-end",
                "justify-between",
                "justify-around",
                "justify-evenly",
            ]),
            vec![
                "justify_start",
                "justify_center",
                "justify_end",
                "justify_between",
                "justify_around",
                "justify_evenly",
            ],
        );
    }

    #[test]
    fn unknown_layout_like_class_still_errors() {
        // Spec line 181: items-stretch has no `Styled` shorthand. Must
        // surface as UnknownClass with a hint and the original token's
        // span — otherwise diagnostics regress.
        let t = ClassToken {
            raw: "items-stretch".into(),
            span: Span::new(7, 20),
        };
        let err = lower_classes(&[t]).unwrap_err();
        match err {
            Error::UnknownClass { class, span, hint } => {
                assert_eq!(class, "items-stretch");
                assert_eq!(span, Span::new(7, 20), "span must round-trip exactly");
                assert!(hint.is_some(), "rejected layout class must carry a hint");
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }

        // `justify-normal` doesn't exist in any spec section — must remain
        // UnknownClass without a hint (no specific guidance to give).
        let err = lower_classes(&[tok("justify-normal")]).unwrap_err();
        match err {
            Error::UnknownClass { class, .. } => assert_eq!(class, "justify-normal"),
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn layout_classes_preserve_source_order() {
        // Builder method order matters: later calls override earlier ones
        // for conflicting Style fields (gpui builder semantics). The
        // lowering must not reorder.
        assert_eq!(
            lowered_method_names(&["justify-center", "items-center", "flex-1", "flex-col"]),
            vec!["justify_center", "items_center", "flex_1", "flex_col"],
        );
    }

    // ---------- issue #10: directional spacing ----------------------------

    #[test]
    fn lower_padding_directional_utilities() {
        // Each axis/side at one representative scale step. The full 0..=12
        // sweep is exercised below via lower_spacing_full_scale_for_each_prefix.
        assert_eq!(
            lowered_method_names(&["p-4", "px-2", "py-3", "pt-1", "pr-5", "pb-6", "pl-7",]),
            vec!["p_4", "px_2", "py_3", "pt_1", "pr_5", "pb_6", "pl_7"],
        );
    }

    #[test]
    fn lower_margin_utilities() {
        assert_eq!(
            lowered_method_names(&["m-0", "mx-1", "my-2", "mt-3", "mr-4", "mb-5", "ml-6",]),
            vec!["m_0", "mx_1", "my_2", "mt_3", "mr_4", "mb_5", "ml_6"],
        );
    }

    #[test]
    fn lower_gap_axis_utilities() {
        // The bare `gap-N` was already handled before #10; this test
        // pins both that path and the new gap-x-/gap-y- variants so the
        // longest-prefix-first ordering can't regress.
        assert_eq!(
            lowered_method_names(&["gap-2", "gap-x-3", "gap-y-4"]),
            vec!["gap_2", "gap_x_3", "gap_y_4"],
        );
    }

    #[test]
    fn negative_margin_is_rejected_with_hint() {
        // Spec line 219: -m-N etc. are explicitly禁止 in v0.1. Diagnostic
        // must call out the spec rationale, not surface a bare UnknownClass.
        for raw in ["-m-2", "-mx-4", "-mt-1", "-ml-3"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    assert!(hint.is_some(), "negative margin `{raw}` must carry a hint");
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn out_of_range_spacing_is_unknown() {
        // 13/14/15 are the contiguous-range gap (spec line 201) and 13
        // is the first value past 12. 99 is well past every allowed step.
        // 17/18/21/25 sit between the jump steps and must also reject.
        // `py-foo` pins the contract that `m-foo` (non-numeric) doesn't
        // accidentally lower.
        for raw in [
            "m-13", "m-14", "m-15", "px-99", "gap-x-13", "py-foo", "p-17", "m-25",
        ] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn spacing_jump_steps_are_accepted() {
        // The non-contiguous portion of the v0.1 scale: {16, 20, 24, 32}.
        // The fixture in #9 uses pt-24 and pb-20, which means lowering
        // these correctly is part of "fixture advances past spacing".
        assert_eq!(
            lowered_method_names(&["p-16", "pt-24", "pb-20", "m-32", "gap-x-16"]),
            vec!["p_16", "pt_24", "pb_20", "m_32", "gap_x_16"],
        );
    }

    #[test]
    fn existing_p_and_gap_lowering_unchanged() {
        // Sanity check: the consolidation into `lower_spacing` must not
        // regress the bare `p-N` and `gap-N` shapes that PR #6 already
        // shipped and that hello.html depends on.
        assert_eq!(
            lowered_method_names(&["p-0", "p-12", "gap-0", "gap-12"]),
            vec!["p_0", "p_12", "gap_0", "gap_12"],
        );
    }

    #[test]
    fn spacing_classes_preserve_source_order() {
        // Same builder-order contract as the layout test above, but for
        // the new spacing prefixes.
        assert_eq!(
            lowered_method_names(&["px-4", "py-2", "mt-1", "gap-3"]),
            vec!["px_4", "py_2", "mt_1", "gap_3"],
        );
    }

    // ---------- issue #12: sizing -----------------------------------------

    #[test]
    fn lower_width_height_size_numeric() {
        assert_eq!(
            lowered_method_names(&["w-0", "h-1", "size-2", "w-12", "h-12", "size-12"]),
            vec!["w_0", "h_1", "size_2", "w_12", "h_12", "size_12"],
        );
    }

    #[test]
    fn lower_min_max_width_height() {
        assert_eq!(
            lowered_method_names(&["min-w-2", "min-h-3", "max-w-4", "max-h-5"]),
            vec!["min_w_2", "min_h_3", "max_w_4", "max_h_5"],
        );
    }

    #[test]
    fn lower_full_and_auto_keywords() {
        assert_eq!(
            lowered_method_names(&["w-full", "h-full", "size-full", "w-auto", "h-auto"]),
            vec!["w_full", "h_full", "size_full", "w_auto", "h_auto"],
        );
    }

    #[test]
    fn lower_fractional_widths_listed_in_spec() {
        // The spec enumerates these four fractions and only these four
        // (lines 198). w-1/4, w-1/5, etc. must remain UnknownClass.
        assert_eq!(
            lowered_method_names(&["w-1/2", "w-1/3", "w-2/3", "w-3/4"]),
            vec!["w_1_2", "w_1_3", "w_2_3", "w_3_4"],
        );
    }

    #[test]
    fn fractional_width_token_survives_tokenizer() {
        // The class tokenizer in `parse::split_classes` only splits on
        // whitespace, so `/` stays inside a single class token. Drive
        // this through the full pipeline (parse -> class lower) to pin
        // the contract end-to-end, not just the unit-level lowering.
        let nodes = crate::parse::parse(r#"<div class="flex w-1/2 gap-2"></div>"#).unwrap();
        let crate::ast::Node::Element(e) = &nodes[0] else {
            panic!("expected element");
        };
        let classes: Vec<&str> = e.classes.iter().map(|c| c.raw.as_str()).collect();
        assert_eq!(classes, vec!["flex", "w-1/2", "gap-2"]);

        let methods = lower_classes(&e.classes).expect("classes should lower");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["flex", "w_1_2", "gap_2"]);
    }

    #[test]
    fn out_of_spec_sizing_is_unknown() {
        // Each of these is something a Tailwind user would expect to
        // work but the v0.1 Size section doesn't list:
        //   - w-1/4 / w-2/4 / w-1/5: fractions outside the four the
        //     spec lists
        //   - w-13: contiguous-range gap
        //   - w-99: well past every allowed step
        //   - w-foo: non-numeric, non-keyword
        //
        // (`w-screen`, `h-screen`, `max-w-128` were here pre-#19 — they
        // now accept. See `lower_w_screen_and_h_screen` and
        // `lower_max_w_128_app_shell_compat` below.)
        for raw in ["w-1/4", "w-2/4", "w-1/5", "w-13", "w-99", "w-foo"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn sizing_jump_steps_share_spacing_scale() {
        // The spacing scale extension landed in #10 covers sizing too
        // because `lower_sizing` reuses `parse_spacing_step`. Pin that
        // contract here so it can't regress without the test breaking.
        assert_eq!(
            lowered_method_names(&["w-16", "h-20", "size-24", "max-w-32"]),
            vec!["w_16", "h_20", "size_24", "max_w_32"],
        );
    }

    #[test]
    fn sizing_classes_preserve_source_order() {
        assert_eq!(
            lowered_method_names(&["w-full", "h-full", "min-w-4", "max-w-12"]),
            vec!["w_full", "h_full", "min_w_4", "max_w_12"],
        );
    }

    // ---------- issue #13: typography -------------------------------------

    #[test]
    fn lower_text_size_utilities() {
        // All seven sizes the spec lists (line 289). `text-base`, `text-2xl`,
        // `text-3xl` are the easy-to-fumble ones because of the `base` and
        // digit-prefix suffixes — pin them explicitly.
        assert_eq!(
            lowered_method_names(&[
                "text-xs",
                "text-sm",
                "text-base",
                "text-lg",
                "text-xl",
                "text-2xl",
                "text-3xl",
            ]),
            vec![
                "text_xs",
                "text_sm",
                "text_base",
                "text_lg",
                "text_xl",
                "text_2xl",
                "text_3xl",
            ],
        );
    }

    #[test]
    fn lower_font_weight_utilities_emit_unary_call() {
        // Font weights expand to .font_weight(FontWeight::<VARIANT>) — the
        // spec is explicit about this not being a shorthand. Verify both
        // the method name and the constructed argument.
        let cases = [
            ("font-thin", "THIN"),
            ("font-light", "LIGHT"),
            ("font-normal", "NORMAL"),
            ("font-medium", "MEDIUM"),
            ("font-semibold", "SEMIBOLD"),
            ("font-bold", "BOLD"),
            ("font-extrabold", "EXTRA_BOLD"),
            ("font-black", "BLACK"),
        ];
        for (raw, variant) in cases {
            let calls = lower_classes(&[tok(raw)]).expect("font-weight should lower");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "font_weight");
            assert_eq!(calls[0].args, vec![format!("FontWeight::{variant}")]);
        }
    }

    #[test]
    fn lower_text_decoration_and_truncate_keywords() {
        assert_eq!(
            lowered_method_names(&["italic", "not-italic", "line-through", "truncate"]),
            vec!["italic", "not_italic", "line_through", "truncate"],
        );
    }

    #[test]
    fn lower_leading_utilities_emit_rems_arg() {
        // leading-* expand to .line_height(rems(<n>)) — spec line 305-310
        // pegs the rem values, so this also pins the rem rendering.
        let cases = [
            ("leading-none", "rems(1.0)"),
            ("leading-tight", "rems(1.25)"),
            ("leading-snug", "rems(1.375)"),
            ("leading-normal", "rems(1.5)"),
            ("leading-relaxed", "rems(1.625)"),
            ("leading-loose", "rems(2.0)"),
        ];
        for (raw, expected_arg) in cases {
            let calls = lower_classes(&[tok(raw)]).expect("leading-* should lower");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "line_height");
            assert_eq!(calls[0].args, vec![expected_arg.to_string()]);
        }
    }

    #[test]
    fn lower_line_clamp_with_numeric_arg() {
        // Spec line 312: line-clamp-<n> → .line_clamp(<n>) for any positive n.
        for n in [1u32, 2, 3, 5, 99] {
            let raw = format!("line-clamp-{n}");
            let calls = lower_classes(&[tok(&raw)]).expect("line-clamp should lower");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "line_clamp");
            assert_eq!(calls[0].args, vec![n.to_string()]);
        }
        // Zero is semantically nonsense; non-numeric and negative reject.
        for raw in ["line-clamp-0", "line-clamp-foo", "line-clamp-"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn font_mono_and_font_serif_still_reject_with_hint() {
        // After #19, `font-sans` is a recognized no-op (see
        // `accept_font_sans_as_noop` below) but `font-mono` and
        // `font-serif` remain rejected: the host's font stack is one
        // font-family at a time, set on the gpui app/Theme rather
        // than per-element. Hint mentions `font-sans` is the only
        // accepted font-family class for compatibility.
        for raw in ["font-mono", "font-serif"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    let hint = hint.expect("font-family must carry a hint");
                    assert!(
                        hint.contains("font-family"),
                        "hint should mention font-family, got: {hint}"
                    );
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn whitespace_and_text_ellipsis_rejected_with_hint() {
        // Spec lines 322-326 explicitly禁止 these. Reject with a hint
        // citing the spec's stated alternative (truncate / direct
        // Style.text manipulation).
        for raw in ["whitespace-nowrap", "whitespace-normal", "text-ellipsis"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { hint, .. } => {
                    assert!(hint.is_some(), "{raw} should carry a hint");
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn typography_size_wins_over_theme_color_disambiguation() {
        // Disambiguation contract: `text-xs..3xl` are typography sizes,
        // not theme color tokens. If a theme had a `xs` color field
        // (silly but legal), `text-xs` still lowers as typography.
        let calls = lower_classes(&[tok("text-xs")]).unwrap();
        assert_eq!(calls, vec![MethodCall::nullary("text_xs")]);

        // Conversely, anything outside the seven listed sizes is a
        // theme color: `text-muted` doesn't accidentally try to lower
        // as a typography size.
        let calls = lower_classes(&[tok("text-muted")]).unwrap();
        assert_eq!(
            calls,
            vec![MethodCall::unary("text_color", "theme.muted".into())]
        );
    }

    #[test]
    fn typography_classes_preserve_source_order() {
        // Mixed typography utilities — sizes, weights, decorations,
        // leading, clamp — must keep source order through lowering so
        // the gpui builder chain matches what the author wrote.
        let calls = lower_classes(&[
            tok("text-sm"),
            tok("font-semibold"),
            tok("italic"),
            tok("leading-tight"),
            tok("truncate"),
            tok("line-clamp-2"),
        ])
        .unwrap();
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "text_sm",
                "font_weight",
                "italic",
                "line_height",
                "truncate",
                "line_clamp",
            ]
        );
    }

    // ---------- issue #14: border -----------------------------------------

    #[test]
    fn lower_bare_border_is_one_pixel() {
        // Spec line 247: bare 'border' resolves to 1px width.
        assert_eq!(lowered_method_names(&["border"]), vec!["border_1"],);
    }

    #[test]
    fn lower_numeric_border_widths() {
        // Contiguous + jump-step values from the v0.1 spacing scale
        // (spec line 248).
        assert_eq!(
            lowered_method_names(&[
                "border-0",
                "border-1",
                "border-4",
                "border-12",
                "border-16",
                "border-32",
            ]),
            vec![
                "border_0",
                "border_1",
                "border_4",
                "border_12",
                "border_16",
                "border_32",
            ],
        );
    }

    #[test]
    fn lower_directional_border_widths() {
        // Explicit numeric on each side (spec line 249).
        assert_eq!(
            lowered_method_names(&[
                "border-t-1",
                "border-r-2",
                "border-b-3",
                "border-l-4",
                "border-t-12",
                "border-b-16",
            ]),
            vec![
                "border_t_1",
                "border_r_2",
                "border_b_3",
                "border_l_4",
                "border_t_12",
                "border_b_16",
            ],
        );
    }

    #[test]
    fn lower_bare_directional_border_resolves_to_one_pixel() {
        // Spec interpretation (see lower_border doc comment): the
        // "bare = 1px" rule from the spec extends to the directional
        // shorthands. The Ato Desktop preview fixture relies on this
        // (`border-b`, `border-r`).
        assert_eq!(
            lowered_method_names(&["border-t", "border-r", "border-b", "border-l"]),
            vec!["border_t_1", "border_r_1", "border_b_1", "border_l_1"],
        );
    }

    #[test]
    fn lower_border_dashed_keyword() {
        assert_eq!(
            lowered_method_names(&["border-dashed"]),
            vec!["border_dashed"],
        );
    }

    #[test]
    fn lower_border_theme_color_token() {
        // `border-<ident>` is the theme-color path. Verify the unary
        // method call shape (`.border_color(theme.<token>)`).
        let calls = lower_classes(&[tok("border-border")]).unwrap();
        assert_eq!(
            calls,
            vec![MethodCall::unary("border_color", "theme.border".into())]
        );

        // Other identifier-shaped tokens follow the same pattern.
        let calls = lower_classes(&[tok("border-accent")]).unwrap();
        assert_eq!(
            calls,
            vec![MethodCall::unary("border_color", "theme.accent".into())]
        );
    }

    #[test]
    fn border_width_and_color_compose_on_one_element() {
        // The acceptance criterion from issue #14:
        //   class="border border-border" → .border_1().border_color(theme.border)
        //
        // Source order is preserved (gpui builder semantics) and the
        // disambiguation works without ambiguity.
        let calls = lower_classes(&[tok("border"), tok("border-border")]).unwrap();
        assert_eq!(
            calls,
            vec![
                MethodCall::nullary("border_1"),
                MethodCall::unary("border_color", "theme.border".into()),
            ]
        );
    }

    #[test]
    fn out_of_range_border_width_is_unknown() {
        // 13/14/15 are the contiguous-range gap. 99 is well past every
        // allowed step. `border-foo` is non-numeric and is also not a
        // valid Rust identifier, so it falls through to UnknownClass
        // (and is_theme_token rejects "foo" with an internal hyphen
        // shape — but plain "foo" is a valid ident, so this verifies
        // *only* the numeric path rejects out-of-range).
        for raw in [
            "border-13",
            "border-14",
            "border-15",
            "border-99",
            "border-t-13",
        ] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn border_with_invalid_identifier_is_unknown() {
        // Tails that don't shape as a theme token must reject. The
        // hint check verifies it's UnknownClass (not theme color).
        // (`border-some-color` was here pre-#7 — it now normalizes to
        // `theme.some_color`. See `lower_hyphenated_border_theme_token`.)
        for raw in [
            "border-1px",     // mixed digit/letter, neither width nor ident
            "border--accent", // empty leading segment
            "border-accent-", // empty trailing segment
        ] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn border_classes_preserve_source_order() {
        // Mixed border utilities (width + directional + dashed + color)
        // keep the source order through lowering.
        assert_eq!(
            lowered_method_names(&["border-2", "border-t-1", "border-dashed"]),
            vec!["border_2", "border_t_1", "border_dashed"],
        );
    }

    // ---------- issue #15: overflow + cursor + opacity + bg-transparent ----

    #[test]
    fn lower_overflow_utilities() {
        // All seven variants the spec lists (lines 336-342).
        assert_eq!(
            lowered_method_names(&[
                "overflow-hidden",
                "overflow-visible",
                "overflow-scroll",
                "overflow-x-hidden",
                "overflow-y-hidden",
                "overflow-x-scroll",
                "overflow-y-scroll",
            ]),
            vec![
                "overflow_hidden",
                "overflow_visible",
                "overflow_scroll",
                "overflow_x_hidden",
                "overflow_y_hidden",
                "overflow_x_scroll",
                "overflow_y_scroll",
            ],
        );
    }

    #[test]
    fn reject_overflow_auto_with_hint() {
        // Spec lines 348-352: gpui has no Overflow::Auto. Reject with a
        // hint pointing at the gpui-side reason.
        for raw in ["overflow-auto", "overflow-x-auto", "overflow-y-auto"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    let hint = hint.unwrap_or_default();
                    assert!(
                        hint.contains("Auto") || hint.contains("auto"),
                        "hint should mention the missing Auto variant, got: {hint}"
                    );
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn lower_cursor_pointer() {
        // The fixture-relevant case.
        assert_eq!(
            lowered_method_names(&["cursor-pointer"]),
            vec!["cursor_pointer"],
        );
    }

    #[test]
    fn lower_all_cursor_variants_listed_in_spec() {
        // Spec lines 363-373 enumerate these. Hyphenated suffixes
        // (`not-allowed`, `col-resize`, ...) should produce
        // underscore-separated method names.
        let cases = [
            ("cursor-default", "cursor_default"),
            ("cursor-pointer", "cursor_pointer"),
            ("cursor-text", "cursor_text"),
            ("cursor-move", "cursor_move"),
            ("cursor-grab", "cursor_grab"),
            ("cursor-grabbing", "cursor_grabbing"),
            ("cursor-not-allowed", "cursor_not_allowed"),
            ("cursor-col-resize", "cursor_col_resize"),
            ("cursor-row-resize", "cursor_row_resize"),
            ("cursor-ew-resize", "cursor_ew_resize"),
            ("cursor-ns-resize", "cursor_ns_resize"),
            ("cursor-nesw-resize", "cursor_nesw_resize"),
            ("cursor-nwse-resize", "cursor_nwse_resize"),
            ("cursor-crosshair", "cursor_crosshair"),
            ("cursor-help", "cursor_help"),
            ("cursor-none", "cursor_none"),
        ];
        for (raw, method) in cases {
            assert_eq!(lowered_method_names(&[raw]), vec![method.to_string()]);
        }
    }

    #[test]
    fn unknown_cursor_variant_is_unknown() {
        // `cursor-zoom-in` and `cursor-wait` are real Tailwind utilities
        // but aren't in the v0.1 spec. Keep them out until they're added
        // explicitly.
        for raw in ["cursor-zoom-in", "cursor-wait", "cursor-foo"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn lower_opacity_utilities() {
        // Boundary values + a midpoint. Argument format is "<n>.0 / 100.0"
        // — a constant-folded Rust expression, readable and unambiguous.
        let cases = [
            ("opacity-0", "0.0 / 100.0"),
            ("opacity-50", "50.0 / 100.0"),
            ("opacity-100", "100.0 / 100.0"),
        ];
        for (raw, expected_arg) in cases {
            let calls = lower_classes(&[tok(raw)]).expect("opacity should lower");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "opacity");
            assert_eq!(calls[0].args, vec![expected_arg.to_string()]);
        }
    }

    #[test]
    fn out_of_range_opacity_is_unknown() {
        // n > 100 is out of spec; non-numeric is non-numeric.
        for raw in ["opacity-101", "opacity-200", "opacity-foo", "opacity-"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn lower_bg_transparent_literal() {
        // Spec line 272: `bg-transparent` is a literal, NOT a theme.transparent
        // lookup. This was a real bug before #15 — `bg-transparent` lowered
        // as `theme.transparent` because `is_theme_token("transparent")` is
        // true. Pin the contract here.
        let calls = lower_classes(&[tok("bg-transparent")]).unwrap();
        assert_eq!(
            calls,
            vec![MethodCall::unary("bg", "gpui::transparent_black()".into())]
        );
    }

    #[test]
    fn reject_bg_token_alpha_without_manifest_points_at_manifest_flag() {
        // `bg-<theme-token>/<n>` only lowers when a manifest declares
        // the token's color. Without one, the compiler has no RGB bytes
        // to combine with the slash alpha, so it rejects with a hint
        // pointing at `--manifest`. This is the v0.1 behavior — keep
        // the contract pinned so a future regression doesn't silently
        // fall through to UnknownClass with the generic palette hint.
        for raw in ["bg-accent/10", "bg-rose/80", "bg-surface/50"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    let hint = hint.unwrap_or_default();
                    assert!(
                        hint.contains("--manifest"),
                        "no-manifest hint should mention --manifest, got: {hint}"
                    );
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn reject_palette_opacity_suffix_keeps_palette_hint() {
        // `bg-red-500/80` is palette + alpha. The token `red-500` has a
        // hyphen so `is_theme_token` rejects it, meaning the new
        // theme-token-alpha branch doesn't claim it. It falls through
        // to the existing palette UnknownClass with `hint_for`'s
        // palette message.
        let err = lower_classes(&[tok("bg-red-500/80")]).unwrap_err();
        match err {
            Error::UnknownClass { hint, .. } => {
                let hint = hint.unwrap_or_default();
                assert!(
                    hint.contains("palette"),
                    "palette+alpha should keep the palette hint, got: {hint}"
                );
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn misc_classes_preserve_source_order() {
        // The five #15 buckets composed: overflow + cursor + opacity +
        // bg-transparent. Order through the lowering must match source.
        let calls = lower_classes(&[
            tok("overflow-y-scroll"),
            tok("cursor-pointer"),
            tok("opacity-50"),
            tok("bg-transparent"),
        ])
        .unwrap();
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["overflow_y_scroll", "cursor_pointer", "opacity", "bg"]
        );
    }

    // ---------- issue #16: radius + shadow --------------------------------

    #[test]
    fn lower_bare_radius_and_named_radius_suffixes() {
        // Bare `rounded` resolves to md (spec line 253). Each named
        // suffix lowers as itself.
        assert_eq!(
            lowered_method_names(&[
                "rounded",
                "rounded-none",
                "rounded-sm",
                "rounded-md",
                "rounded-lg",
                "rounded-xl",
                "rounded-2xl",
                "rounded-3xl",
                "rounded-full",
            ]),
            vec![
                "rounded_md",
                "rounded_none",
                "rounded_sm",
                "rounded_md",
                "rounded_lg",
                "rounded_xl",
                "rounded_2xl",
                "rounded_3xl",
                "rounded_full",
            ],
        );
    }

    #[test]
    fn lower_directional_radius_utilities() {
        // Sides: t / r / b / l (spec line 256).
        assert_eq!(
            lowered_method_names(&[
                "rounded-t-md",
                "rounded-r-lg",
                "rounded-b-sm",
                "rounded-l-full",
                "rounded-t-2xl",
            ]),
            vec![
                "rounded_t_md",
                "rounded_r_lg",
                "rounded_b_sm",
                "rounded_l_full",
                "rounded_t_2xl",
            ],
        );
    }

    #[test]
    fn lower_corner_radius_utilities() {
        // Corners: tl / tr / bl / br (spec line 257). Crucially these
        // must take precedence over single-side prefixes — e.g.
        // `rounded-tl-md` is corner top-left, not side `t` with
        // suffix `l-md`.
        assert_eq!(
            lowered_method_names(&[
                "rounded-tl-md",
                "rounded-tr-lg",
                "rounded-bl-sm",
                "rounded-br-full",
                "rounded-tr-3xl",
            ]),
            vec![
                "rounded_tl_md",
                "rounded_tr_lg",
                "rounded_bl_sm",
                "rounded_br_full",
                "rounded_tr_3xl",
            ],
        );
    }

    #[test]
    fn lower_bare_shadow_and_named_shadow_suffixes() {
        // Bare `shadow` resolves to md (spec line 259). Spec line 260
        // lists sm/md/lg/xl/2xl/none — no 3xl.
        assert_eq!(
            lowered_method_names(&[
                "shadow",
                "shadow-sm",
                "shadow-md",
                "shadow-lg",
                "shadow-xl",
                "shadow-2xl",
                "shadow-none",
            ]),
            vec![
                "shadow_md",
                "shadow_sm",
                "shadow_md",
                "shadow_lg",
                "shadow_xl",
                "shadow_2xl",
                "shadow_none",
            ],
        );
    }

    #[test]
    fn reject_unknown_radius_suffix() {
        // Invalid suffix, invalid side, bare directional, and unknown
        // tail — all must surface as UnknownClass without lowering.
        for raw in [
            "rounded-4xl",       // suffix not in spec
            "rounded-middle-md", // invalid pseudo-side
            "rounded-t",         // bare directional, no spec backing
            "rounded-tl",        // bare corner, no spec backing
            "rounded-l-4xl",     // valid side, invalid suffix
            "rounded-foo",       // unknown plain suffix
        ] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn reject_unknown_shadow_suffix() {
        // Spec stops shadow at 2xl — `shadow-3xl` rejects. Other shapes
        // (typo, pseudo-side, etc.) also reject.
        for raw in [
            "shadow-3xl",   // out of spec
            "shadow-large", // wrong vocabulary
            "shadow-t-md",  // shadow has no directional form
            "shadow-foo",
        ] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn radius_2xl_and_3xl_tokenize_cleanly() {
        // The tokenizer keeps `2xl`/`3xl` as single suffixes (no
        // accidental split on the digit). Drive end-to-end through
        // parse + lower so both layers are pinned together.
        let nodes =
            crate::parse::parse(r#"<div class="rounded-2xl rounded-tr-3xl"></div>"#).unwrap();
        let crate::ast::Node::Element(e) = &nodes[0] else {
            panic!("expected element");
        };
        let raws: Vec<&str> = e.classes.iter().map(|c| c.raw.as_str()).collect();
        assert_eq!(raws, vec!["rounded-2xl", "rounded-tr-3xl"]);

        let calls = lower_classes(&e.classes).expect("classes should lower");
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["rounded_2xl", "rounded_tr_3xl"]);
    }

    #[test]
    fn radius_and_shadow_preserve_source_order() {
        // Mixed radius variants and shadow keep source order through
        // lowering.
        assert_eq!(
            lowered_method_names(&["rounded-lg", "shadow-lg", "rounded-tr-md", "shadow"]),
            vec!["rounded_lg", "shadow_lg", "rounded_tr_md", "shadow_md"],
        );
    }

    // ---------- issue #7: hyphenated theme tokens ------------------------

    fn lowered_calls(raw: &str) -> Vec<MethodCall> {
        lower_classes(&[tok(raw)]).expect("class should lower")
    }

    #[test]
    fn lower_hyphenated_bg_theme_token() {
        // The classic case: a multi-word color name from the host theme.
        // Hyphens become underscores at the Rust ident boundary; the
        // class attribute keeps Tailwind-shape spelling.
        assert_eq!(
            lowered_calls("bg-accent-foreground"),
            vec![MethodCall::unary("bg", "theme.accent_foreground".into())]
        );
        assert_eq!(
            lowered_calls("bg-muted-foreground"),
            vec![MethodCall::unary("bg", "theme.muted_foreground".into())]
        );
    }

    #[test]
    fn lower_hyphenated_text_theme_token() {
        assert_eq!(
            lowered_calls("text-accent-foreground"),
            vec![MethodCall::unary(
                "text_color",
                "theme.accent_foreground".into()
            )]
        );
        // Three segments still works.
        assert_eq!(
            lowered_calls("text-primary-hover-state"),
            vec![MethodCall::unary(
                "text_color",
                "theme.primary_hover_state".into()
            )]
        );
    }

    #[test]
    fn lower_hyphenated_border_theme_token() {
        assert_eq!(
            lowered_calls("border-accent-foreground"),
            vec![MethodCall::unary(
                "border_color",
                "theme.accent_foreground".into()
            )]
        );
        // Single-segment still works (existing behavior preserved).
        assert_eq!(
            lowered_calls("border-border"),
            vec![MethodCall::unary("border_color", "theme.border".into())]
        );
    }

    #[test]
    fn border_numeric_still_lowers_to_width() {
        // The numeric-width branch in `lower_border` must keep
        // priority over the (now hyphen-friendly) theme-token branch.
        // `border-12` is width, not a theme color named `12`.
        assert_eq!(
            lowered_method_names(&["border-1", "border-2", "border-12"]),
            vec!["border_1", "border_2", "border_12"],
        );
    }

    #[test]
    fn text_size_still_lowers_to_typography() {
        // Typography sizes (text-xs..3xl) dispatch via `lower_typography`
        // before the theme-token branch, so the new hyphenated theme
        // logic doesn't change their behavior.
        assert_eq!(
            lowered_method_names(&["text-xs", "text-sm", "text-base", "text-lg", "text-2xl"]),
            vec!["text_xs", "text_sm", "text_base", "text_lg", "text_2xl"],
        );
    }

    #[test]
    fn palette_utility_still_rejects_with_hint() {
        // The crucial disambiguation: `bg-red-500` shape (numeric last
        // segment) is palette, not a hyphenated theme token. Must still
        // reject with the same actionable hint.
        for raw in ["bg-red-500", "text-blue-300", "border-slate-200"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    let hint = hint.unwrap_or_default();
                    assert!(
                        hint.contains("palette"),
                        "expected palette hint for `{raw}`, got: {hint}"
                    );
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn slash_alpha_for_hyphenated_theme_token_routes_to_manifest_hint() {
        // `bg-accent-foreground/10` is theme-token alpha — hyphenated
        // theme tokens are theme tokens, not palette. Without a
        // manifest, must produce the manifest-pointer hint, not the
        // palette hint.
        let err = lower_classes(&[tok("bg-accent-foreground/10")]).unwrap_err();
        match err {
            Error::UnknownClass { hint, .. } => {
                let hint = hint.unwrap_or_default();
                assert!(
                    hint.contains("--manifest"),
                    "expected manifest-aware hint for hyphenated theme alpha, got: {hint}"
                );
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn slash_alpha_palette_still_rejects_with_palette_hint() {
        // `bg-red-500/80` is palette + alpha. The token shape rejects
        // via `normalize_theme_token`'s palette guard, so it falls
        // through to `hint_for`, which now also recognizes the slash
        // suffix when checking palette shape.
        let err = lower_classes(&[tok("bg-red-500/80")]).unwrap_err();
        match err {
            Error::UnknownClass { hint, .. } => {
                let hint = hint.unwrap_or_default();
                assert!(
                    hint.contains("palette"),
                    "palette+alpha should keep the palette hint, got: {hint}"
                );
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn malformed_theme_token_rejects() {
        // Empty segments (leading, trailing, or doubled hyphens),
        // segments starting with digits, and pure-numeric segments
        // all reject — these aren't legal Rust idents.
        for raw in [
            "bg--accent",        // empty leading segment
            "bg-accent-",        // empty trailing segment
            "bg-2accent",        // segment starts with digit
            "bg-accent--state",  // empty middle segment
            "bg-accent-3-state", // numeric middle segment
            "text-",             // empty after prefix
        ] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            assert!(
                matches!(err, Error::UnknownClass { .. }),
                "expected UnknownClass for `{raw}`, got {err:?}"
            );
        }
    }

    #[test]
    fn css_and_utility_theme_normalization_match() {
        // The same shared `normalize_theme_token` helper drives both
        // the utility-class side here and the CSS `var(--theme-X)`
        // path in css.rs. Whatever Rust ident the utility produces
        // for `text-accent-foreground` must match what CSS produces
        // for `var(--theme-accent-foreground)`. Pinned end-to-end via
        // the codegen integration so both surfaces stay in lockstep.
        let utility = lowered_calls("text-accent-foreground");
        assert_eq!(utility[0].args, vec!["theme.accent_foreground".to_string()]);

        let nodes = crate::parse::parse(
            r#"<html><head><style>.x { color: var(--theme-accent-foreground); }</style></head><body><div class="x"></div></body></html>"#,
        )
        .unwrap();
        let out = crate::codegen::emit(&nodes).unwrap();
        // `.x` from CSS should also reference `theme.accent_foreground`.
        assert!(
            out.contains("text_color(theme.accent_foreground)"),
            "CSS theme-var normalization diverged from utility, got: {out}"
        );
    }

    // ---------- issue #19: app-shell utilities ---------------------------

    #[test]
    fn lower_w_screen_and_h_screen() {
        // Viewport keywords (#19). gpui's `Styled` exposes
        // `w_screen()` / `h_screen()` directly.
        assert_eq!(
            lowered_method_names(&["w-screen", "h-screen"]),
            vec!["w_screen", "h_screen"],
        );
    }

    #[test]
    fn lower_size_screen_keyword() {
        // Symmetry with `size-full`: same shape, viewport variant.
        assert_eq!(lowered_method_names(&["size-screen"]), vec!["size_screen"],);
    }

    #[test]
    fn lower_max_w_128_app_shell_compat() {
        // The Ato Desktop preview's Tailwind config defines
        // `maxWidth: { '128': '32rem' }`. v0.1 doesn't ship a generic
        // custom-scale manifest, but exempts this single token so the
        // #9 acceptance fixture compiles end-to-end.
        let calls = lowered_calls("max-w-128");
        assert_eq!(calls, vec![MethodCall::unary("max_w", "rems(32.0)".into())]);
    }

    #[test]
    fn reject_other_custom_max_w_tokens_with_hint() {
        // Anything other than the explicit `max-w-128` exemption must
        // still reject, with a hint pointing at the v0.2 manifest path.
        for raw in ["max-w-200", "max-w-card", "max-w-129", "max-h-card"] {
            let err = lower_classes(&[tok(raw)]).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    let hint = hint.unwrap_or_default();
                    assert!(
                        hint.contains("manifest"),
                        "expected v0.2 manifest hint for `{raw}`, got: {hint}"
                    );
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn accept_font_sans_as_noop() {
        // `font-sans` is recognized for Tailwind preview compatibility
        // but emits no MethodCall. The class doesn't error; the call
        // chain is empty.
        let calls = lower_classes(&[tok("font-sans")]).unwrap();
        assert!(
            calls.is_empty(),
            "font-sans must lower to no MethodCall, got: {calls:?}"
        );
    }

    #[test]
    fn font_sans_does_not_reorder_or_block_other_classes() {
        // Mixed with real utilities, font-sans's source position is
        // honored by the iteration but it doesn't appear in the output
        // — the other utilities lower in their normal order. Each
        // arrangement (front, middle, end) must produce the same two
        // calls in the same order.
        let expected = vec![
            MethodCall::nullary("flex"),
            MethodCall::unary("text_color", "theme.primary".into()),
        ];
        for arrangement in [
            ["font-sans", "flex", "text-primary"],
            ["flex", "font-sans", "text-primary"],
            ["flex", "text-primary", "font-sans"],
        ] {
            let toks: Vec<ClassToken> = arrangement.iter().copied().map(tok).collect();
            assert_eq!(
                lower_classes(&toks).unwrap(),
                expected,
                "font-sans changed the lowered output for {arrangement:?}",
            );
        }
    }

    #[test]
    fn full_preview_remaining_app_shell_classes_no_longer_unknown() {
        // The four classes the FULL Ato Desktop preview fixture had
        // outstanding before #19: w-screen, h-screen, max-w-128,
        // font-sans. Each must lower (or no-op) without surfacing
        // UnknownClass — that's the #9 acceptance criterion.
        for raw in ["w-screen", "h-screen", "max-w-128", "font-sans"] {
            let result = lower_classes(&[tok(raw)]);
            assert!(
                result.is_ok(),
                "fixture-blocking class `{raw}` should no longer error, got {result:?}"
            );
        }
    }
}
