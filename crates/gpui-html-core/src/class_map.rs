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
//! Color (symbolic theme tokens):
//!   bg-<token>      -> .bg(theme.<token>)
//!   text-<token>    -> .text_color(theme.<token>)
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

use crate::ast::ClassToken;
use crate::Error;

/// One method call in the emitted gpui builder chain, e.g. `.gap_2()` or
/// `.bg(theme.surface)`. Args are stored as already-formatted Rust source
/// fragments; codegen splices them verbatim between the parens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodCall {
    pub name: String,
    pub args: Vec<String>,
}

impl MethodCall {
    fn nullary(name: &str) -> Self {
        MethodCall {
            name: name.to_string(),
            args: vec![],
        }
    }

    fn unary(name: &str, arg: String) -> Self {
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
pub fn lower_classes(classes: &[ClassToken]) -> Result<Vec<MethodCall>, Error> {
    classes.iter().map(lower_one).collect()
}

fn lower_one(tok: &ClassToken) -> Result<MethodCall, Error> {
    let raw = tok.raw.as_str();

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
        return Ok(call);
    }

    if let Some(call) = lower_spacing(raw) {
        return Ok(call);
    }

    if let Some(call) = lower_sizing(raw) {
        return Ok(call);
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
        return Ok(call);
    }

    if let Some(token) = raw.strip_prefix("bg-") {
        if is_theme_token(token) {
            return Ok(MethodCall::unary("bg", format!("theme.{token}")));
        }
    }

    if let Some(token) = raw.strip_prefix("text-") {
        // text-xs..text-3xl are typography sizes (handled by
        // `lower_typography` above). Anything else under `text-` is a
        // color theme token.
        if is_theme_token(token) {
            return Ok(MethodCall::unary("text_color", format!("theme.{token}")));
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
fn lower_sizing(raw: &str) -> Option<MethodCall> {
    if let Some(call) = lower_sizing_keyword(raw) {
        return Some(call);
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
/// and the four fractional widths. Anything else (`w-screen`, `w-1/5`,
/// `w-1/4`) falls through to `UnknownClass` because the spec doesn't
/// list it.
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
        "font-sans" | "font-mono" | "font-serif" => Some(
            "font-family utilities are not in the v0.1 Typography section. \
             Set font-family at the gpui app/theme level — it's usually a \
             one-time configuration, not a per-element class. See #19 for \
             the design discussion."
                .into(),
        ),
        _ => None,
    }
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

/// Theme tokens are identifier-shaped (Rust ident rules: leading letter
/// or underscore, then letters/digits/underscores). Hyphens are not
/// allowed because they'd collide with class-name segmentation.
fn is_theme_token(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_typography_size(s: &str) -> bool {
    matches!(s, "xs" | "sm" | "base" | "lg" | "xl" | "2xl" | "3xl")
}

/// Best-effort hint shown on the LLM-facing diagnostic when a class is
/// rejected. Kept terse — the diagnostic itself already carries the span,
/// the literal, and the error code.
fn hint_for(raw: &str) -> Option<String> {
    if raw.starts_with("bg-") && raw.matches('-').count() >= 2 {
        return Some(
            "v0.1 only allows `bg-<ident>` against the caller's theme; \
             palette utilities like `bg-red-500` are out of scope."
                .into(),
        );
    }
    None
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

    #[test]
    fn hyphenated_theme_token_is_rejected() {
        // `bg-some-color` would be ambiguous with palette utilities, so
        // theme tokens are restricted to identifier shape.
        let err = lower_classes(&[tok("bg-some-color")]).unwrap_err();
        assert!(matches!(err, Error::UnknownClass { .. }));
    }

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
        //   - w-screen / h-screen: viewport keywords (gpui has w_screen,
        //     spec just doesn't enumerate it yet)
        //   - max-w-128: tailwind-config-extended scale (custom token)
        //   - w-1/4 / w-2/4 / w-1/5: fractions outside the four the
        //     spec lists
        //   - w-13: contiguous-range gap
        //   - w-99: well past every allowed step
        //   - w-foo: non-numeric, non-keyword
        for raw in [
            "w-screen",
            "h-screen",
            "max-w-128",
            "w-1/4",
            "w-2/4",
            "w-1/5",
            "w-13",
            "w-99",
            "w-foo",
        ] {
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
    fn font_family_utilities_are_rejected_with_hint() {
        // Per #19: font-family is host-app concern, not gpuiHTML's. Reject
        // with a hint pointing at that issue so the diagnostic is actionable.
        for raw in ["font-sans", "font-mono", "font-serif"] {
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
}
