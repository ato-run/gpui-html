//! Intermediate representation between the gpuiHTML parser and the codegen.
//!
//! Every node carries a [`Span`] so diagnostics can point at exact source
//! offsets — see the spec's "Diagnostics" section for the requirement.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(TextNode),
    /// Raw `<style>...</style>` content captured verbatim. The parser
    /// consumes the inner CSS as raw text (no nested element parsing)
    /// and stores it here for the static-CSS lowering pipeline (#27).
    /// Codegen ignores `Style` nodes for v0.1: they don't count toward
    /// root-element rules and don't emit any builder calls until #27
    /// wires the lowering through.
    Style(StyleNode),
}

impl Node {
    pub fn span(&self) -> Span {
        match self {
            Node::Element(e) => e.span,
            Node::Text(t) => t.span,
            Node::Style(s) => s.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag: String,
    pub tag_span: Span,
    pub classes: Vec<ClassToken>,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// Span covering the entire element from `<` of the open tag to `>` of
    /// the close tag (or `/>` for self-closing).
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextNode {
    pub text: String,
    pub span: Span,
}

/// Captured `<style>` content. The `css` field holds the raw CSS source
/// between the open and close tags (verbatim, no entity decoding). The
/// `span` covers the entire `<style>...</style>` block so diagnostics
/// can point at the exact source location of an offending rule once
/// #27 lands the CSS lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleNode {
    pub css: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassToken {
    pub raw: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub name_span: Span,
    pub value: String,
    pub value_span: Span,
}
