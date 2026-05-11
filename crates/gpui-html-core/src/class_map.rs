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

    if let Some(token) = raw.strip_prefix("bg-") {
        if is_theme_token(token) {
            return Ok(MethodCall::unary("bg", format!("theme.{token}")));
        }
    }

    if let Some(token) = raw.strip_prefix("text-") {
        // Typography size utilities (text-xs..text-3xl) live in a future
        // milestone; reject them with a hint so callers don't silently
        // get `theme.xs` lookups.
        if is_typography_size(token) {
            return Err(Error::UnknownClass {
                class: raw.to_string(),
                span: tok.span,
                hint: hint_for(raw),
            });
        }
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
    if let Some(rest) = raw.strip_prefix("text-") {
        if is_typography_size(rest) {
            return Some(
                "typography size utilities are not in the v0.1 vertical slice. \
                 Use a theme token (e.g. `text-muted`) or wait for full v0.1 typography support."
                    .into(),
            );
        }
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

    #[test]
    fn text_typography_utility_is_rejected_with_hint() {
        let err = lower_classes(&[tok("text-xs")]).unwrap_err();
        match err {
            Error::UnknownClass { class, hint, .. } => {
                assert_eq!(class, "text-xs");
                assert!(hint.is_some(), "typography utilities should carry a hint");
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

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
}
