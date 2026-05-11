//! Codegen (stage 3): `Node` tree -> gpui builder Rust source.
//!
//! Emits a `String` of Rust source rather than a `TokenStream` so the same
//! code path serves both the CLI (writing files) and a future proc-macro
//! frontend (which can re-tokenise via `proc_macro2::TokenStream::from_str`).
//!
//! v0.1 vertical-slice contract:
//!
//! - exactly one root element per source file (codegen returns
//!   `CodegenError` for 0 or >1 roots; orchestration of multiple roots is
//!   a v0.2 concern)
//! - output is a single dense expression on one line; rustfmt is the
//!   user's responsibility
//! - class lowering is delegated to [`crate::class_map`]
//! - non-class attributes recognised in v0.1: `id` only, lowered to
//!   `.id("...")` (matches `gpui::Styled::id`)
//! - text node children are emitted as Rust string literals (with `\`
//!   and `"` escaped); `<span>x</span>` lowers to `span().child("x")`
//!   so the gpui span element survives the round-trip

use crate::ast::{Element, Node, TextNode};
use crate::class_map::{lower_classes, MethodCall};
use crate::{CodegenError, Error, Span};

/// Compile a parsed node tree into gpui builder Rust source.
///
/// `Node::Style` entries are filtered out before applying the
/// "exactly one root" rule — they're metadata for the static-CSS
/// lowering pipeline (#27), not UI nodes. The same filter applies
/// inside element children so a nested `<style>` doesn't produce an
/// empty `.child()` call.
pub fn emit(nodes: &[Node]) -> Result<String, Error> {
    let ui_nodes: Vec<&Node> = nodes.iter().filter(|n| !is_metadata(n)).collect();

    if ui_nodes.is_empty() {
        return Err(Error::Codegen(CodegenError {
            span: Span::new(0, 0),
            message: "empty document — gpuiHTML requires exactly one root element".into(),
        }));
    }
    if ui_nodes.len() > 1 {
        let span = ui_nodes
            .iter()
            .map(|n| n.span())
            .reduce(Span::merge)
            .unwrap_or(Span::new(0, 0));
        return Err(Error::Codegen(CodegenError {
            span,
            message: format!(
                "expected exactly one root element, found {}",
                ui_nodes.len()
            ),
        }));
    }

    match ui_nodes[0] {
        Node::Element(e) => {
            let mut out = String::new();
            emit_element(e, &mut out)?;
            Ok(out)
        }
        Node::Text(t) => Err(Error::Codegen(CodegenError {
            span: t.span,
            message: "top-level text is not allowed — wrap content in a single root element".into(),
        })),
        Node::Style(_) => unreachable!("filtered out by is_metadata above"),
    }
}

/// Nodes that don't count as UI roots and aren't emitted by codegen.
/// Currently just `Node::Style`; future metadata-bearing variants
/// (e.g. a `Node::Title` for `<title>` if we ever surface it) would
/// land here.
fn is_metadata(node: &Node) -> bool {
    matches!(node, Node::Style(_))
}

fn emit_node(node: &Node, out: &mut String) -> Result<(), Error> {
    match node {
        Node::Element(e) => emit_element(e, out),
        Node::Text(t) => {
            emit_text_literal(t, out);
            Ok(())
        }
        // Reachable only if a future caller forgets to filter; produce
        // no output to keep the builder chain syntactically valid.
        Node::Style(_) => Ok(()),
    }
}

fn emit_element(el: &Element, out: &mut String) -> Result<(), Error> {
    out.push_str(&el.tag);
    out.push_str("()");

    let methods = lower_classes(&el.classes)?;
    for m in &methods {
        emit_method_call(m, out);
    }

    for attr in &el.attrs {
        match attr.name.as_str() {
            "class" => {} // already lowered above via el.classes
            "id" => {
                out.push_str(".id(\"");
                push_escaped(&attr.value, out);
                out.push_str("\")");
            }
            other => {
                return Err(Error::UnsupportedAttribute {
                    attr: other.to_string(),
                    span: attr.name_span,
                });
            }
        }
    }

    for child in el.children.iter().filter(|c| !is_metadata(c)) {
        out.push_str(".child(");
        emit_node(child, out)?;
        out.push(')');
    }

    Ok(())
}

fn emit_method_call(m: &MethodCall, out: &mut String) {
    out.push('.');
    out.push_str(&m.name);
    out.push('(');
    for (i, arg) in m.args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(arg);
    }
    out.push(')');
}

fn emit_text_literal(t: &TextNode, out: &mut String) {
    out.push('"');
    push_escaped(&t.text, out);
    out.push('"');
}

fn push_escaped(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn compile(src: &str) -> Result<String, Error> {
        let nodes = parse::parse(src)?;
        emit(&nodes)
    }

    #[test]
    fn empty_document_is_codegen_error() {
        let err = compile("").unwrap_err();
        assert!(matches!(err, Error::Codegen(_)));
    }

    #[test]
    fn two_roots_is_codegen_error() {
        let err = compile("<div></div><div></div>").unwrap_err();
        assert!(matches!(err, Error::Codegen(_)));
    }

    #[test]
    fn empty_div_is_a_bare_call() {
        let out = compile("<div></div>").unwrap();
        assert_eq!(out, "div()");
    }

    #[test]
    fn span_with_text_keeps_span_wrapper() {
        let out = compile("<span>hi</span>").unwrap();
        assert_eq!(out, r#"span().child("hi")"#);
    }

    #[test]
    fn id_attr_lowers_to_id_method() {
        let out = compile(r#"<div id="root"></div>"#).unwrap();
        assert_eq!(out, r#"div().id("root")"#);
    }

    #[test]
    fn classes_emit_in_source_order() {
        let out = compile(r#"<div class="flex flex-col gap-2"></div>"#).unwrap();
        assert_eq!(out, "div().flex().flex_col().gap_2()");
    }

    #[test]
    fn theme_tokens_are_passed_through_symbolically() {
        let out = compile(r#"<div class="bg-surface text-muted"></div>"#).unwrap();
        assert_eq!(out, "div().bg(theme.surface).text_color(theme.muted)");
    }

    #[test]
    fn nested_tree_emits_chained_child_calls() {
        let out = compile(r#"<div class="flex"><span>a</span><span>b</span></div>"#).unwrap();
        assert_eq!(
            out,
            r#"div().flex().child(span().child("a")).child(span().child("b"))"#
        );
    }

    #[test]
    fn text_with_quotes_is_escaped() {
        let out = compile(r#"<span>say "hi"</span>"#).unwrap();
        assert_eq!(out, r#"span().child("say \"hi\"")"#);
    }

    #[test]
    fn unknown_class_propagates_with_span() {
        let err = compile(r#"<div class="overflow-auto"></div>"#).unwrap_err();
        match err {
            Error::UnknownClass { class, .. } => assert_eq!(class, "overflow-auto"),
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn hello_html_snapshot() {
        // Mirrors examples/hello.html. If you change the example, update
        // this snapshot in the same commit so the file and the test
        // always agree on the v0.1 contract.
        let src = "<div class=\"flex flex-col gap-2 p-4 bg-surface\">\n  <span>Hello, gpui!</span>\n  <div class=\"text-muted\">Compiled from HTML.</div>\n</div>";
        let out = compile(src).unwrap();
        assert_eq!(
            out,
            r#"div().flex().flex_col().gap_2().p_4().bg(theme.surface).child(span().child("Hello, gpui!")).child(div().text_color(theme.muted).child("Compiled from HTML."))"#
        );
    }

    // ---------- issue #26: full HTML document codegen ---------------------

    #[test]
    fn compile_full_html_document_uses_body_root() {
        // Boilerplate around a single body root must produce the same
        // output as the bare element — wrappers and metadata are
        // stripped, the body's UI subtree is the codegen root.
        let full = r#"<!DOCTYPE html>
<html lang="ja">
  <head>
    <meta charset="utf-8">
    <title>Hello</title>
  </head>
  <body>
    <div class="flex"><span>hi</span></div>
  </body>
</html>"#;
        let bare = r#"<div class="flex"><span>hi</span></div>"#;
        assert_eq!(compile(full).unwrap(), compile(bare).unwrap());
    }

    #[test]
    fn compile_full_html_with_head_style_script_ignores_non_ui_nodes() {
        // <style> survives parse (Node::Style) but is filtered from
        // root counting and from element children. <script>, <meta>,
        // <link>, <title> never produce nodes at all. Net effect: the
        // body's div is the only root.
        let src = r#"<!DOCTYPE html>
<html>
  <head>
    <meta charset="utf-8">
    <link rel="preconnect" href="https://fonts.example/">
    <title>x</title>
    <style>.a { color: red; }</style>
    <script>console.log("hi");</script>
  </head>
  <body>
    <div></div>
  </body>
</html>"#;
        assert_eq!(compile(src).unwrap(), "div()");
    }

    #[test]
    fn compile_body_with_two_roots_still_errors() {
        // Multi-root rule applies to the flattened post-wrapper node
        // list, not just to bare-element input.
        let err = compile("<html><body><div></div><div></div></body></html>").unwrap_err();
        assert!(matches!(err, Error::Codegen(_)));
    }

    #[test]
    fn compile_body_with_no_roots_errors() {
        // An HTML document whose body contains only metadata / no UI
        // elements produces the standard empty-document codegen error.
        let err = compile("<html><body></body></html>").unwrap_err();
        assert!(matches!(err, Error::Codegen(_)));
    }

    #[test]
    fn compile_without_body_falls_back_to_first_document_ui_root() {
        // No html/body wrappers — the existing single-root rule applies
        // at document level. (This is the pre-#26 path; verify it
        // still works.)
        assert_eq!(compile("<div></div>").unwrap(), "div()");
    }

    #[test]
    fn style_node_does_not_appear_as_child() {
        // A `<style>` inside an element body shouldn't produce a
        // `.child()` call — the metadata filter applies to children too.
        let src = "<div><style>.foo { color: red; }</style><span>hi</span></div>";
        assert_eq!(compile(src).unwrap(), r#"div().child(span().child("hi"))"#);
    }
}
