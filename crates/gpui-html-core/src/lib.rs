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
pub mod class_map;
pub mod codegen;
pub mod diagnostic;
pub mod parse;

pub use ast::Span;

/// Convenience: gpuiHTML source -> emitted gpui Rust source.
pub fn compile(src: &str) -> Result<String, Error> {
    let nodes = parse::parse(src)?;
    codegen::emit(&nodes)
}

/// Structured compiler error. Every variant carries the source [`Span`] of
/// the offending token so diagnostics can render line/column and a caret —
/// see [`diagnostic::Diagnostic`] for the wire schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Parse(ParseError),
    UnknownTag {
        tag: String,
        span: Span,
    },
    UnknownClass {
        class: String,
        span: Span,
        hint: Option<String>,
    },
    UnsupportedAttribute {
        attr: String,
        span: Span,
    },
    UnknownThemeToken {
        token: String,
        span: Span,
    },
    InvalidEventHandler {
        name: String,
        span: Span,
    },
    InvalidInterpolation {
        raw: String,
        span: Span,
    },
    Codegen(CodegenError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// `<a>...</b>` — close tag name doesn't match the open.
    UnbalancedTag { expected: String, found: String },
    /// `</div>` at document root, or where no element is open.
    UnexpectedClosingTag { tag: String },
    /// `<div>` with no matching `</div>` before EOF.
    UnclosedTag,
    /// Open tag was not closed before EOF (EOF inside `<div ...`).
    EofInTag,
    /// Attribute value was not double-quoted (`<div class=flex>`).
    UnquotedAttrValue,
    /// Attribute value used single quotes (`<div class='x'>`).
    SingleQuotedAttrValue,
    /// Opening `"` was found but the closing `"` never arrived.
    UnclosedAttribute,
    /// Catch-all for "I expected character X here, found Y".
    InvalidCharacter(char),
    /// Generic premature EOF that doesn't fit a more specific kind above.
    UnexpectedEof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenError {
    pub span: Span,
    pub message: String,
}

impl Error {
    pub fn span(&self) -> Span {
        match self {
            Error::Parse(e) => e.span,
            Error::UnknownTag { span, .. } => *span,
            Error::UnknownClass { span, .. } => *span,
            Error::UnsupportedAttribute { span, .. } => *span,
            Error::UnknownThemeToken { span, .. } => *span,
            Error::InvalidEventHandler { span, .. } => *span,
            Error::InvalidInterpolation { span, .. } => *span,
            Error::Codegen(e) => e.span,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Error::Parse(_) => "Parse",
            Error::UnknownTag { .. } => "UnknownTag",
            Error::UnknownClass { .. } => "UnknownClass",
            Error::UnsupportedAttribute { .. } => "UnsupportedAttribute",
            Error::UnknownThemeToken { .. } => "UnknownThemeToken",
            Error::InvalidEventHandler { .. } => "InvalidEventHandler",
            Error::InvalidInterpolation { .. } => "InvalidInterpolation",
            Error::Codegen(_) => "Codegen",
        }
    }

    pub fn literal(&self) -> &str {
        match self {
            Error::Parse(e) => match &e.kind {
                ParseErrorKind::UnbalancedTag { found, .. } => found,
                ParseErrorKind::UnexpectedClosingTag { tag } => tag,
                _ => &e.message,
            },
            Error::UnknownTag { tag, .. } => tag,
            Error::UnknownClass { class, .. } => class,
            Error::UnsupportedAttribute { attr, .. } => attr,
            Error::UnknownThemeToken { token, .. } => token,
            Error::InvalidEventHandler { name, .. } => name,
            Error::InvalidInterpolation { raw, .. } => raw,
            Error::Codegen(e) => &e.message,
        }
    }

    pub fn hint(&self) -> Option<&str> {
        match self {
            Error::UnknownClass { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Error::Parse(e) => e.message.clone(),
            Error::UnknownTag { tag, .. } => format!("unknown tag `<{tag}>`"),
            Error::UnknownClass { class, .. } => format!("unknown class `{class}`"),
            Error::UnsupportedAttribute { attr, .. } => {
                format!("unsupported attribute `{attr}`")
            }
            Error::UnknownThemeToken { token, .. } => {
                format!("unknown theme token `{token}`")
            }
            Error::InvalidEventHandler { name, .. } => {
                format!("invalid event handler `{name}`")
            }
            Error::InvalidInterpolation { raw, .. } => {
                format!("invalid interpolation `{raw}`")
            }
            Error::Codegen(e) => e.message.clone(),
        }
    }
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Error::Parse(e)
    }
}

impl From<CodegenError> for Error {
    fn from(e: CodegenError) -> Self {
        Error::Codegen(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for Error {}
