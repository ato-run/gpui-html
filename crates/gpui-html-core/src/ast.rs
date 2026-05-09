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
}

impl Node {
    pub fn span(&self) -> Span {
        match self {
            Node::Element(e) => e.span,
            Node::Text(t) => t.span,
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
