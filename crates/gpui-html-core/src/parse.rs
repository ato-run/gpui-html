//! gpuiHTML parser (stage 1).
//!
//! gpuiHTML is a strict subset of HTML5: tags must be balanced, attribute
//! values must be double-quoted, and only the elements/attributes listed in
//! the spec (see repo README) are recognised. Unknown tags become an
//! [`Error::UnknownTag`] rather than being passed through verbatim — the
//! point of the spec is that everything maps to a known gpui call.

use crate::ast::Node;

#[derive(Debug)]
pub enum Error {
    Unimplemented,
}

pub fn parse(_src: &str) -> Result<Vec<Node>, Error> {
    Err(Error::Unimplemented)
}
