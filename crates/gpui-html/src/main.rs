//! gpui-html — CLI that compiles gpuiHTML files into gpui Rust source.
//!
//! Usage:
//!
//! ```text
//! gpui-html compile <input> [-o <output>] [--format human|json]
//! gpui-html check   <input>              [--format human|json]
//! ```
//!
//! Exit codes:
//!
//! - `0` — success
//! - `1` — gpuiHTML compilation error (parse / class / codegen)
//! - `2` — CLI usage error (missing file, bad flags, IO failure)
//!
//! `--format json` emits one [`Diagnostic`](gpui_html_core::diagnostic::Diagnostic)
//! per line on stderr (newline-delimited JSON) so editors and CI can
//! parse it without a streaming JSON reader. The schema is stable across
//! v0.1 patch releases — see docs/spec.md § Diagnostics.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use gpui_html_core::diagnostic::Diagnostic;

#[derive(Debug, Parser)]
#[command(
    name = "gpui-html",
    about = "Compile gpuiHTML files into gpui builder Rust code.",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Compile a gpuiHTML file to gpui Rust source.
    Compile {
        /// Input gpuiHTML file.
        input: PathBuf,
        /// Output file. Writes to stdout if omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Diagnostic output format.
        #[arg(long, default_value_t = Format::Human)]
        format: Format,
    },
    /// Parse and lower an input without writing output. Exits non-zero
    /// on any compilation error.
    Check {
        /// Input gpuiHTML file.
        input: PathBuf,
        /// Diagnostic output format.
        #[arg(long, default_value_t = Format::Human)]
        format: Format,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Json,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::Human => f.write_str("human"),
            Format::Json => f.write_str("json"),
        }
    }
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Compile {
            input,
            output,
            format,
        } => run_compile(input, output, format),
        Cmd::Check { input, format } => run_check(input, format),
    }
}

fn run_compile(input: PathBuf, output: Option<PathBuf>, format: Format) -> ExitCode {
    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gpui-html: cannot read {}: {e}", input.display());
            return ExitCode::from(2);
        }
    };

    let rendered = match gpui_html_core::compile(&src) {
        Ok(s) => s,
        Err(err) => {
            report_error(&err, &src, Some(&input), format);
            return ExitCode::from(1);
        }
    };

    match output {
        None => {
            // Trailing newline so the file ends cleanly when piped to a file.
            println!("{rendered}");
        }
        Some(path) => {
            if let Err(e) = std::fs::write(&path, format!("{rendered}\n")) {
                eprintln!("gpui-html: cannot write {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
    }
    ExitCode::SUCCESS
}

fn run_check(input: PathBuf, format: Format) -> ExitCode {
    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gpui-html: cannot read {}: {e}", input.display());
            return ExitCode::from(2);
        }
    };
    match gpui_html_core::compile(&src) {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err, &src, Some(&input), format);
            ExitCode::from(1)
        }
    }
}

fn report_error(err: &gpui_html_core::Error, src: &str, input: Option<&PathBuf>, format: Format) {
    let file = input.map(|p| p.display().to_string());
    let diag = Diagnostic::from_error(err, src, file.as_deref());
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    match format {
        Format::Json => match serde_json::to_string(&diag) {
            Ok(line) => {
                let _ = writeln!(stderr, "{line}");
            }
            Err(e) => {
                let _ = writeln!(stderr, "gpui-html: failed to encode diagnostic: {e}");
            }
        },
        Format::Human => {
            let location = match &diag.file {
                Some(f) => format!("{f}:{}:{}", diag.line, diag.column),
                None => format!("<input>:{}:{}", diag.line, diag.column),
            };
            let _ = writeln!(stderr, "error[{}]: {}", diag.code, diag.message);
            let _ = writeln!(stderr, "  --> {location}");
            let _ = writeln!(stderr, "  literal: {}", diag.literal);
            if let Some(hint) = &diag.hint {
                let _ = writeln!(stderr, "  hint: {hint}");
            }
        }
    }
}
