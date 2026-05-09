//! gpuiHTML parser (stage 1).
//!
//! gpuiHTML is a strict subset of HTML5: tags must be balanced, attribute
//! values must be double-quoted, and only the elements/attributes listed in
//! the spec are recognised. Unknown tags become [`Error::UnknownTag`] rather
//! than being passed through verbatim — the point of the spec is that
//! everything maps to a known gpui call.

use crate::ast::Node;
use crate::{Error, ParseError, ParseErrorKind};

/// Implemented in a follow-up commit (issue #1). Stub returns `UnexpectedEof`
/// for the empty-input case so downstream code can already pattern-match
/// against the structured error type.
pub fn parse(src: &str) -> Result<Vec<Node>, Error> {
    Err(Error::Parse(ParseError {
        kind: ParseErrorKind::UnexpectedEof,
        span: crate::ast::Span::new(0, src.len()),
        message: "parser not yet implemented".into(),
    }))
}
