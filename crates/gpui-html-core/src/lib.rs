//! gpui-html-core — the IR + codegen layer of gpuiHTML.
//!
//! Pipeline: gpuiHTML source -> [`ast::Node`] tree -> gpui builder Rust code.
//!
//! The crate is split into three stages so each can be replaced independently:
//!
//! 1. [`parse`] — gpuiHTML (an HTML subset) into an `ast::Node` tree.
//! 2. [`ast`]   — the intermediate representation: tag, attrs, children, with
//!                class names already split into utility tokens.
//! 3. [`codegen`] — emit gpui builder calls (`div().flex().child(...)`) as a
//!                  `String` of Rust source.
//!
//! Stage boundaries are deliberately narrow so a future proc-macro frontend
//! can reuse stages 2 and 3 without touching the HTML parser, and a future
//! LSP server can reuse stage 1 without touching codegen.

pub mod ast;
pub mod codegen;
pub mod parse;

/// Convenience: gpuiHTML source -> emitted gpui Rust source.
///
/// Equivalent to `codegen::emit(&parse::parse(src)?)`.
pub fn compile(_src: &str) -> Result<String, Error> {
    Err(Error::Unimplemented)
}

#[derive(Debug)]
pub enum Error {
    Unimplemented,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unimplemented => write!(f, "not yet implemented"),
        }
    }
}

impl std::error::Error for Error {}
