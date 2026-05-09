//! Intermediate representation between the gpuiHTML parser and the codegen.
//!
//! `Node` is intentionally HTML-shaped (tag/attrs/children) rather than
//! gpui-shaped (builder calls). The mapping from HTML tag + class tokens to
//! gpui builder methods happens in [`crate::codegen`], not here.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag: String,
    pub classes: Vec<String>,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub value: String,
}
