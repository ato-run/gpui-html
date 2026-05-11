//! Class lowering (stage 2.5): `ClassToken` -> `MethodCall` IR.
//!
//! v0.1 vertical-slice scope (kept deliberately narrow so the end-to-end
//! pipeline can be exercised before fleshing out the full spec table):
//!
//! ```text
//! flex            -> .flex()
//! flex-col        -> .flex_col()
//! gap-N           -> .gap_N()         for N in 0..=12
//! p-N             -> .p_N()           for N in 0..=12
//! bg-<token>      -> .bg(theme.<token>)
//! text-<token>    -> .text_color(theme.<token>)
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
    match raw {
        "flex" => return Ok(MethodCall::nullary("flex")),
        "flex-col" => return Ok(MethodCall::nullary("flex_col")),
        _ => {}
    }

    if let Some(rest) = raw.strip_prefix("gap-") {
        if let Some(n) = parse_spacing_step(rest) {
            return Ok(MethodCall::nullary(&format!("gap_{n}")));
        }
    }

    if let Some(rest) = raw.strip_prefix("p-") {
        if let Some(n) = parse_spacing_step(rest) {
            return Ok(MethodCall::nullary(&format!("p_{n}")));
        }
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

/// v0.1 spacing scale: 0..=12 inclusive. The gpui `Styled` trait exposes
/// `gap_0`..`gap_12` (and `p_0`..`p_12`) as the matching builder methods.
fn parse_spacing_step(rest: &str) -> Option<u32> {
    let n: u32 = rest.parse().ok()?;
    (n <= 12).then_some(n)
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
        assert_eq!(out, vec![MethodCall::nullary("flex"), MethodCall::nullary("flex_col")]);
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
        assert_eq!(
            out,
            vec![MethodCall::unary("bg", "theme.surface".into())]
        );
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
}
