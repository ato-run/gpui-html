//! gpui-html — CLI that compiles gpuiHTML files into gpui Rust source.
//!
//! Usage (planned):
//!     gpui-html <input.html> [-o <output.rs>]
//!     gpui-html --check <input.html>     # parse only, no codegen
//!
//! The CLI is a thin shell over [`gpui_rsx::compile`]; the interesting code
//! lives in the `gpui-rsx` crate. Keeping the CLI thin means an editor
//! plugin or `build.rs` invocation can call the library directly without
//! shelling out.

fn main() -> std::process::ExitCode {
    eprintln!("gpui-html: not yet implemented");
    std::process::ExitCode::from(2)
}
