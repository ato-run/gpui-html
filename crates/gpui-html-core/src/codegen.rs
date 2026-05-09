//! Codegen (stage 3): rsx AST -> gpui builder Rust source.
//!
//! Emits a `String` of Rust source rather than a `TokenStream` so the same
//! code path serves both the CLI (writing files) and a future proc-macro
//! frontend (which can re-tokenise via `proc_macro2::TokenStream::from_str`).
//!
//! Class tokens are mapped to gpui builder methods one-to-one, e.g.
//! `flex` -> `.flex()`, `gap-2` -> `.gap_2()`. The mapping table lives here
//! and is the source of truth for "what gpuiHTML supports".

use crate::ast::Node;

pub fn emit(_nodes: &[Node]) -> String {
    String::new()
}
