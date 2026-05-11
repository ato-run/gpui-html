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
use crate::class_map::{lower_classes_with_styles, MethodCall, StyleMap};
use crate::manifest::ThemeManifest;
use crate::{css, CodegenError, Error, Span};

/// Compile a parsed node tree into gpui builder Rust source using the
/// compiler's built-in defaults (no manifest).
///
/// Equivalent to [`emit_with_manifest`] called with `None`.
pub fn emit(nodes: &[Node]) -> Result<String, Error> {
    emit_with_manifest(nodes, None)
}

/// Compile a parsed node tree, optionally validating theme tokens and
/// resolving custom-scale sizing utilities against a [`ThemeManifest`].
///
/// `Node::Style` entries are filtered out before applying the
/// "exactly one root" rule — they're metadata for the static-CSS
/// lowering pipeline. Their CSS bodies are parsed (#27) into a
/// combined `StyleMap` that's threaded through element lowering, so
/// rules in `<style>` apply alongside utility classes from `class=`.
/// The same filter applies inside element children so a nested
/// `<style>` doesn't produce an empty `.child()` call.
pub fn emit_with_manifest(
    nodes: &[Node],
    manifest: Option<&ThemeManifest>,
) -> Result<String, Error> {
    // Collect every `<style>` node anywhere in the AST, parse and
    // lower its CSS, and merge into one map. Same class appearing in
    // multiple stylesheets (or repeated within one) gets all rules
    // appended in source order — the lowering layer already keeps
    // declaration source order within each rule.
    let style_map = collect_style_map(nodes)?;

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
            emit_element(e, &style_map, manifest, &mut out)?;
            Ok(out)
        }
        Node::Text(t) => Err(Error::Codegen(CodegenError {
            span: t.span,
            message: "top-level text is not allowed — wrap content in a single root element".into(),
        })),
        Node::Style(_) => unreachable!("filtered out by is_metadata above"),
    }
}

/// Walk the node tree (including element subtrees) collecting every
/// `<style>` block, parse each, and merge the lowerings into a single
/// `StyleMap`. Same class appearing across multiple stylesheets gets
/// all rule MethodCalls in document order.
fn collect_style_map(nodes: &[Node]) -> Result<StyleMap, Error> {
    let mut map = StyleMap::new();
    for node in nodes {
        collect_style_map_node(node, &mut map)?;
    }
    Ok(map)
}

fn collect_style_map_node(node: &Node, map: &mut StyleMap) -> Result<(), Error> {
    match node {
        Node::Style(style) => {
            let local = css::parse_and_lower(&style.css, style.content_start)?;
            for (class, calls) in local {
                map.entry(class).or_default().extend(calls);
            }
            Ok(())
        }
        Node::Element(el) => {
            for child in &el.children {
                collect_style_map_node(child, map)?;
            }
            Ok(())
        }
        Node::Text(_) => Ok(()),
    }
}

/// Nodes that don't count as UI roots and aren't emitted by codegen.
/// Currently just `Node::Style`; future metadata-bearing variants
/// (e.g. a `Node::Title` for `<title>` if we ever surface it) would
/// land here.
fn is_metadata(node: &Node) -> bool {
    matches!(node, Node::Style(_))
}

fn emit_node(
    node: &Node,
    style_map: &StyleMap,
    manifest: Option<&ThemeManifest>,
    out: &mut String,
) -> Result<(), Error> {
    match node {
        Node::Element(e) => emit_element(e, style_map, manifest, out),
        Node::Text(t) => {
            emit_text_literal(t, out);
            Ok(())
        }
        // Reachable only if a future caller forgets to filter; produce
        // no output to keep the builder chain syntactically valid.
        Node::Style(_) => Ok(()),
    }
}

fn emit_element(
    el: &Element,
    style_map: &StyleMap,
    manifest: Option<&ThemeManifest>,
    out: &mut String,
) -> Result<(), Error> {
    // Map the source tag to its gpui constructor + tag-specific
    // default method calls. `div`/`span` keep their literal name; the
    // semantic tags (`p`, `h1..h3`, `button`) all use `div()` because
    // gpui doesn't expose dedicated constructors for those — the
    // browser-shaped semantics are encoded purely as default styling.
    let (constructor, defaults) = tag_constructor_and_defaults(&el.tag);
    out.push_str(constructor);
    out.push_str("()");

    // Tag defaults come BEFORE the user's class chain so the user's
    // `class="..."` (or matching `<style>` rule) wins via gpui
    // builder semantics — later calls override earlier ones. Authors
    // can therefore tweak heading defaults with the usual utility
    // shorthand, e.g. `<h1 class="text-sm font-normal">`.
    for m in defaults {
        emit_method_call(&m, out);
    }

    let methods = lower_classes_with_styles(&el.classes, style_map, manifest)?;
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
        emit_node(child, style_map, manifest, out)?;
        out.push(')');
    }

    Ok(())
}

/// Map an HTML UI tag to its gpui constructor name + the canonical
/// default `MethodCall`s the codegen emits before the user's class
/// chain.
///
/// Reflects the v0.1 spec's tag table (`docs/spec.md` § 対応タグ):
///
/// - `<div>` / `<span>` keep their literal names (gpui's `div()` and
///   `span()`).
/// - `<p>` / `<button>` lower to `div()` with no inherent default —
///   the spec lists "paragraph default text style" / "clickable
///   element" but treating them as plain divs lets `class=` carry
///   any styling the author wants without baking opinions in.
/// - `<h1>` / `<h2>` / `<h3>` lower to `div()` with the explicit
///   defaults the spec specifies (text size + font weight). Users
///   override via `class=`.
///
/// The remaining spec-listed tags (`img`, `icon`, `slot`) are still
/// rejected at parse time — they need asset / component-registry /
/// runtime support that's not part of the v0.1 lowering surface.
fn tag_constructor_and_defaults(tag: &str) -> (&'static str, Vec<MethodCall>) {
    match tag {
        "div" => ("div", Vec::new()),
        "span" => ("span", Vec::new()),
        "p" => ("div", Vec::new()),
        "button" => ("div", Vec::new()),
        "h1" => (
            "div",
            vec![
                MethodCall::nullary("text_2xl"),
                MethodCall::unary("font_weight", "FontWeight::BOLD".into()),
            ],
        ),
        "h2" => (
            "div",
            vec![
                MethodCall::nullary("text_xl"),
                MethodCall::unary("font_weight", "FontWeight::SEMIBOLD".into()),
            ],
        ),
        "h3" => (
            "div",
            vec![
                MethodCall::nullary("text_lg"),
                MethodCall::unary("font_weight", "FontWeight::SEMIBOLD".into()),
            ],
        ),
        // Parser's SUPPORTED_TAGS keeps this exhaustive; anything else
        // never reaches the codegen stage.
        other => unreachable!(
            "codegen reached unknown UI tag `{other}` — parser SUPPORTED_TAGS and \
             codegen tag_constructor_and_defaults are out of sync"
        ),
    }
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
        // <style> survives parse (Node::Style), gets parsed for CSS
        // rules (#27), but its rules don't appear as children — they
        // bind to elements that carry the matching class. With no
        // element using `.a`, the rule is parsed and discarded; output
        // is still just the body's div.
        let src = r#"<!DOCTYPE html>
<html>
  <head>
    <meta charset="utf-8">
    <link rel="preconnect" href="https://fonts.example/">
    <title>x</title>
    <style>.a { color: var(--theme-primary); }</style>
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
        // (Use a v0.1-valid CSS rule; the test is about the metadata
        // filter, not the CSS parser.)
        let src = "<div><style>.foo { color: var(--theme-primary); }</style><span>hi</span></div>";
        assert_eq!(compile(src).unwrap(), r#"div().child(span().child("hi"))"#);
    }

    // ---------- issue #27: <style> CSS lowering integration ---------------

    #[test]
    fn compile_full_html_with_style_class_rule() {
        // The smoke from issue #27: full HTML, a single class rule
        // applied to a div whose `class=` references that rule.
        let src = r#"<!DOCTYPE html>
<html>
  <head>
    <style>.shell { display: flex; color: var(--theme-primary); }</style>
  </head>
  <body>
    <div class="shell"></div>
  </body>
</html>"#;
        let out = compile(src).unwrap();
        // Phase 1 (CSS rule) emits flex + text_color before any utility
        // would (no utility for `shell` exists).
        assert_eq!(out, "div().flex().text_color(theme.primary)");
    }

    #[test]
    fn utility_class_overrides_style_rule_by_order() {
        // CSS rule first (Phase 1), utility second (Phase 2). When both
        // touch the same Style field — gap here — the utility's later
        // call wins per gpui builder semantics. The test pins the
        // EMITTED ORDER; semantic override is a property of gpui.
        let src = r#"<style>.a { gap: 1rem; }</style><div class="a gap-2"></div>"#;
        let out = compile(src).unwrap();
        // Source order in class=: "a gap-2".
        // Phase 1 (CSS for "a"): gap_4
        // Phase 1 (CSS for "gap-2"): nothing (no rule)
        // Phase 2 (utility for "a"): nothing (not a utility, but rule
        //   covered it so no error)
        // Phase 2 (utility for "gap-2"): gap_2
        // Final: gap_4 then gap_2 — utility wins.
        assert_eq!(out, "div().gap_4().gap_2()");
    }

    #[test]
    fn multiple_style_blocks_preserve_source_order() {
        // Two separate <style> blocks, same class .foo. Rules
        // concatenate in source order: first stylesheet's rule, then
        // second's, then any utility lowering for "foo" (none here).
        let src = r#"<html>
  <head>
    <style>.foo { padding: 1rem; }</style>
    <style>.foo { gap: 0.5rem; }</style>
  </head>
  <body>
    <div class="foo"></div>
  </body>
</html>"#;
        assert_eq!(compile(src).unwrap(), "div().p_4().gap_2()");
    }

    #[test]
    fn style_rule_without_matching_class_is_silently_unused() {
        // CSS rule for `.unused` is parsed and lowered but the document
        // has no element with that class — the lowering simply isn't
        // emitted. (No warning channel in v0.1; this could become a
        // soft diagnostic later.)
        let src = r#"<html>
  <head><style>.unused { padding: 1rem; }</style></head>
  <body><div></div></body>
</html>"#;
        assert_eq!(compile(src).unwrap(), "div()");
    }

    #[test]
    fn class_with_only_a_css_rule_is_not_unknown() {
        // `.shell` is not a utility class. Pre-#27 this would be
        // `UnknownClass`. With #27, having a CSS rule for it is enough
        // — the lowering layer skips Phase 2's utility-lookup error
        // when the style map has it covered.
        let src = r#"<style>.shell { display: flex; }</style><div class="shell"></div>"#;
        assert_eq!(compile(src).unwrap(), "div().flex()");
    }

    #[test]
    fn unknown_class_with_no_style_rule_still_errors() {
        // Sanity check: classes that are NEITHER a known utility NOR
        // covered by a CSS rule still error (we didn't accidentally
        // turn the strict UnknownClass into a no-op).
        let src = r#"<div class="not-a-utility-or-rule"></div>"#;
        let err = compile(src).unwrap_err();
        assert!(matches!(err, Error::UnknownClass { .. }));
    }

    #[test]
    fn style_node_inside_element_subtree_still_contributes() {
        // The collector walks element children too — a `<style>` deep
        // inside the AST should still build the StyleMap.
        let src = r#"<div class="shell">
  <style>.shell { display: flex; }</style>
  <span>hi</span>
</div>"#;
        let out = compile(src).unwrap();
        assert_eq!(out, r#"div().flex().child(span().child("hi"))"#);
    }

    // ---------- broader UI tags --------------------------------------------

    #[test]
    fn p_lowers_to_plain_div() {
        // `<p>` has no inherent heading-style defaults — Tailwind's
        // `<p>` resets typography but doesn't add specific sizes.
        // Class attribute carries any styling.
        let out = compile("<p>hello</p>").unwrap();
        assert_eq!(out, r#"div().child("hello")"#);
    }

    #[test]
    fn button_lowers_to_plain_div() {
        // Interactivity (`on:click`) is a separate feature track;
        // for v0.1 a `<button>` is a `div()` styled via classes.
        let out = compile(r#"<button class="bg-accent">Go</button>"#).unwrap();
        assert_eq!(out, r#"div().bg(theme.accent).child("Go")"#);
    }

    #[test]
    fn h1_emits_heading_defaults_before_class_chain() {
        // Per spec: <h1> → div() + text_2xl + font_weight BOLD.
        // Defaults precede the class chain so user classes can
        // override.
        let out = compile("<h1>Title</h1>").unwrap();
        assert_eq!(
            out,
            r#"div().text_2xl().font_weight(FontWeight::BOLD).child("Title")"#
        );
    }

    #[test]
    fn h2_emits_heading_defaults() {
        let out = compile("<h2>Sub</h2>").unwrap();
        assert_eq!(
            out,
            r#"div().text_xl().font_weight(FontWeight::SEMIBOLD).child("Sub")"#
        );
    }

    #[test]
    fn h3_emits_heading_defaults() {
        let out = compile("<h3>Three</h3>").unwrap();
        assert_eq!(
            out,
            r#"div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Three")"#
        );
    }

    #[test]
    fn user_class_overrides_heading_defaults_via_builder_order() {
        // h1 defaults: text_2xl, font_weight(BOLD).
        // User class:  text-sm, font-normal.
        // Output: defaults first, then user — later calls override
        // earlier (gpui builder semantics). The user gets text-sm /
        // font-normal as the effective Style fields.
        let out = compile(r#"<h1 class="text-sm font-normal">x</h1>"#).unwrap();
        assert_eq!(
            out,
            r#"div().text_2xl().font_weight(FontWeight::BOLD).text_sm().font_weight(FontWeight::NORMAL).child("x")"#
        );
    }

    #[test]
    fn span_still_emits_span_constructor() {
        // Regression: only div/span keep their literal constructors.
        // span() must still be span(), not div() — the spec calls out
        // that v0.2 may add an inline-flex shim but for v0.1 gpui has
        // span() and we use it directly.
        let out = compile("<span>hi</span>").unwrap();
        assert_eq!(out, r#"span().child("hi")"#);
    }

    #[test]
    fn deferred_spec_tags_still_unknown_tag() {
        // <img>, <icon>, <slot> are in the spec table but explicitly
        // deferred — they need asset / component / runtime support.
        // Make sure they don't accidentally slip through.
        for raw in ["<img/>", "<icon/>", "<slot/>"] {
            let err = compile(raw).unwrap_err();
            assert!(
                matches!(err, Error::UnknownTag { .. }),
                "expected UnknownTag for {raw:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn heading_with_id_attr_preserves_attr() {
        // The `id="..."` attribute path is shared with div/span and
        // must still work for the semantic tags.
        let out = compile(r#"<h2 id="exec-plan">Execution Plan</h2>"#).unwrap();
        assert_eq!(
            out,
            r#"div().text_xl().font_weight(FontWeight::SEMIBOLD).id("exec-plan").child("Execution Plan")"#
        );
    }

    // ---------- theme manifest integration ---------------------------------

    fn compile_with(src: &str, manifest: &crate::ThemeManifest) -> Result<String, Error> {
        let nodes = crate::parse::parse(src)?;
        emit_with_manifest(&nodes, Some(manifest))
    }

    fn small_manifest() -> crate::ThemeManifest {
        crate::ThemeManifest::from_toml(
            r##"
            [colors]
            accent = "#6366f1"
            "accent-foreground" = "#ffffff"
            primary = "#fafafa"

            [max-width]
            "128" = "32rem"
            "page" = "48rem"
            "##,
        )
        .unwrap()
    }

    #[test]
    fn manifest_unknown_color_token_rejects_at_bg() {
        let m = small_manifest();
        let err = compile_with(r#"<div class="bg-not-a-real"></div>"#, &m).unwrap_err();
        match err {
            Error::UnknownThemeToken { token, .. } => {
                assert_eq!(token, "not-a-real");
            }
            other => panic!("expected UnknownThemeToken, got {other:?}"),
        }
    }

    #[test]
    fn manifest_known_color_passes_at_bg_text_border() {
        let m = small_manifest();
        // Hyphenated theme tokens still normalize for the Rust ident
        // (`accent-foreground` → `theme.accent_foreground`) even with
        // a manifest in scope.
        let out = compile_with(
            r#"<div class="bg-accent text-primary border-accent-foreground"></div>"#,
            &m,
        )
        .unwrap();
        assert_eq!(
            out,
            "div().bg(theme.accent).text_color(theme.primary).border_color(theme.accent_foreground)"
        );
    }

    #[test]
    fn manifest_custom_max_w_resolves_via_lookup() {
        let m = small_manifest();
        // `max-w-128` exists in the manifest as `rems(32.0)` and
        // resolves through the manifest path (not the built-in
        // compat exemption).
        let out = compile_with(r#"<div class="max-w-128 max-w-page"></div>"#, &m).unwrap();
        assert_eq!(out, "div().max_w(rems(32.0)).max_w(rems(48.0))");
    }

    #[test]
    fn manifest_unknown_custom_max_w_still_unknown_class() {
        let m = small_manifest();
        // `max-w-card` isn't in the manifest → falls through to the
        // existing `UnknownClass` + v0.2-manifest hint path.
        let err = compile_with(r#"<div class="max-w-card"></div>"#, &m).unwrap_err();
        assert!(matches!(err, Error::UnknownClass { .. }));
    }

    #[test]
    fn no_manifest_back_compat_max_w_128_still_resolves() {
        // Without a manifest, the built-in single-token compatibility
        // exemption still resolves `max-w-128`. Pre-manifest fixtures
        // (the Ato Desktop preview) must keep compiling unchanged.
        let out = compile(r#"<div class="max-w-128"></div>"#).unwrap();
        assert_eq!(out, "div().max_w(rems(32.0))");
    }

    #[test]
    fn no_manifest_back_compat_theme_tokens_pass_through() {
        // Without a manifest, theme tokens pass through symbolically
        // (no UnknownThemeToken even for unknown names — rustc
        // validates downstream).
        let out = compile(r#"<div class="bg-totally-made-up text-foo"></div>"#).unwrap();
        assert_eq!(out, "div().bg(theme.totally_made_up).text_color(theme.foo)");
    }

    #[test]
    fn manifest_unknown_color_token_rejects_at_text_and_border_too() {
        let m = small_manifest();
        // text-X and border-X paths both validate against [colors].
        let err = compile_with(r#"<div class="text-unknown"></div>"#, &m).unwrap_err();
        assert!(matches!(err, Error::UnknownThemeToken { .. }));

        let err = compile_with(r#"<div class="border-unknown"></div>"#, &m).unwrap_err();
        assert!(matches!(err, Error::UnknownThemeToken { .. }));
    }

    // ---------- #23: theme-token alpha lowering ---------------------------

    #[test]
    fn manifest_bg_token_alpha_lowers_to_packed_rgba() {
        // #23: `bg-accent/50` reads RGB from the manifest's [colors]
        // (`accent = "#6366f1"`) and stitches the slash alpha (50%) on
        // as the A byte. Rounding follows the CSS convention
        // (`(n * 255 + 50) / 100`), so 50% → 128 = 0x80, matching
        // `rgba(_, _, _, 0.5)`.
        let m = small_manifest();
        let out = compile_with(r#"<div class="bg-accent/50"></div>"#, &m).unwrap();
        assert_eq!(out, "div().bg(gpui::rgba(0x6366f180))");
    }

    #[test]
    fn manifest_bg_token_alpha_endpoint_values() {
        // Boundary values: /0 → fully transparent, /100 → fully opaque.
        // /10 is the common Tailwind low-opacity value; pin its rounding
        // (10 * 255 + 50) / 100 = 26 = 0x1a so the spec-rendered
        // canonical example is stable.
        let m = small_manifest();
        let cases = [
            ("bg-accent/0", "div().bg(gpui::rgba(0x6366f100))"),
            ("bg-accent/10", "div().bg(gpui::rgba(0x6366f11a))"),
            ("bg-accent/100", "div().bg(gpui::rgba(0x6366f1ff))"),
        ];
        for (cls, expected) in cases {
            let html = format!(r#"<div class="{cls}"></div>"#);
            let out = compile_with(&html, &m).unwrap();
            assert_eq!(out, expected, "case `{cls}`");
        }
    }

    #[test]
    fn manifest_bg_hyphenated_token_alpha_lowers() {
        // Hyphenated theme tokens (`accent-foreground`) must work the
        // same way — the manifest key is hyphenated, but the lowering
        // emits a packed RGBA literal (no `theme.` field access),
        // so the snake_case normalization doesn't matter here.
        let m = small_manifest();
        // accent-foreground = #ffffff → 100% opacity literal alpha
        // suffix /25 → (25*255+50)/100 = 6425/100 = 64 = 0x40
        let out = compile_with(r#"<div class="bg-accent-foreground/25"></div>"#, &m).unwrap();
        assert_eq!(out, "div().bg(gpui::rgba(0xffffff40))");
    }

    #[test]
    fn manifest_bg_token_alpha_slash_overrides_manifest_alpha() {
        // If the manifest declared `#rrggbbaa` (with alpha), the slash
        // alpha wins. The manifest's A byte is for plain `bg-<token>`
        // semantics (which we still don't emit as a literal — only
        // `theme.<name>` reads it at runtime); the slash alpha is what
        // the author wrote here and now.
        let m = crate::ThemeManifest::from_toml(
            r##"
            [colors]
            translucent = "#11223380"
            "##,
        )
        .unwrap();
        // /50 overrides the manifest's 0x80 → 50% = 0x80 (rounding
        // happens to land on the same byte here; the point of this
        // test is the override, not the rounding).
        let out = compile_with(r#"<div class="bg-translucent/50"></div>"#, &m).unwrap();
        assert_eq!(out, "div().bg(gpui::rgba(0x11223380))");
        // /25 overrides the manifest's 0x80 → 25% = 0x40, proving
        // the override is real (not just coincidentally the same).
        let out = compile_with(r#"<div class="bg-translucent/25"></div>"#, &m).unwrap();
        assert_eq!(out, "div().bg(gpui::rgba(0x11223340))");
    }

    #[test]
    fn manifest_bg_unknown_token_alpha_rejects_with_unknown_theme_token() {
        // `bg-mystery/30` with a manifest in scope is an unknown theme
        // token — same diagnostic as plain `bg-mystery` would emit.
        // Don't silently fall through to UnknownClass.
        let m = small_manifest();
        let err = compile_with(r#"<div class="bg-mystery/30"></div>"#, &m).unwrap_err();
        match err {
            Error::UnknownThemeToken { token, .. } => assert_eq!(token, "mystery"),
            other => panic!("expected UnknownThemeToken, got {other:?}"),
        }
    }

    #[test]
    fn manifest_bg_palette_alpha_still_rejects() {
        // `bg-red-500/80` is palette + alpha. The user-emphasised
        // contract is: even with a manifest, palette tokens don't
        // lower (the compiler doesn't know Tailwind palette colors).
        // Keep the palette hint, not a packed-RGBA emission.
        let m = small_manifest();
        let err = compile_with(r#"<div class="bg-red-500/80"></div>"#, &m).unwrap_err();
        match err {
            Error::UnknownClass { hint, .. } => {
                let hint = hint.unwrap_or_default();
                assert!(
                    hint.contains("palette"),
                    "palette+alpha must keep palette hint, got: {hint}"
                );
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    #[test]
    fn manifest_bg_token_alpha_out_of_range_rejects() {
        // Alpha 0..=100 only. `/101`, `/200` etc. fall outside the
        // Tailwind opacity scale and reject with a range hint, not a
        // surprising wrap-around.
        let m = small_manifest();
        for raw in ["bg-accent/101", "bg-accent/200", "bg-accent/255"] {
            let html = format!(r#"<div class="{raw}"></div>"#);
            let err = compile_with(&html, &m).unwrap_err();
            match err {
                Error::UnknownClass { class, hint, .. } => {
                    assert_eq!(class, raw);
                    let hint = hint.unwrap_or_default();
                    assert!(
                        hint.contains("0..=100"),
                        "out-of-range hint should mention 0..=100, got: {hint}"
                    );
                }
                other => panic!("expected UnknownClass for `{raw}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn no_manifest_bg_token_alpha_still_rejects_with_manifest_hint() {
        // Backstop: without a manifest, `bg-accent/50` rejects with the
        // hint pointing at `--manifest`, not a packed-RGBA literal —
        // the compiler can't invent RGB bytes from thin air.
        let err = compile(r#"<div class="bg-accent/50"></div>"#).unwrap_err();
        match err {
            Error::UnknownClass { hint, .. } => {
                let hint = hint.unwrap_or_default();
                assert!(
                    hint.contains("--manifest"),
                    "no-manifest path must point at --manifest, got: {hint}"
                );
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }
}
