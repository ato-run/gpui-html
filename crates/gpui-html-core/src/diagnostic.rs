//! Diagnostic JSON schema (stable for v0.1) — see [docs/spec.md § Diagnostics].
//!
//! The schema is stable across patch versions of the v0.1 line so LLM-driven
//! consumers can rely on it for self-correction. The CLI emits one
//! [`Diagnostic`] per error, one JSON object per line, when invoked with
//! `--format json`.

use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub file: Option<String>,
    pub line: usize,
    pub column: usize,
    pub span: (usize, usize),
    pub literal: String,
    pub hint: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn from_error(err: &Error, source: &str, file: Option<&str>) -> Self {
        let span = err.span();
        let (line, column) = line_column(source, span.start);
        Diagnostic {
            code: err.code().to_string(),
            file: file.map(str::to_string),
            line,
            column,
            span: (span.start, span.end),
            literal: err.literal().to_string(),
            hint: err.hint().map(str::to_string),
            message: err.message(),
        }
    }
}

/// 1-based line and column for the byte offset.
pub fn line_column(src: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in src.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_column_basics() {
        let src = "abc\ndef\nghij";
        assert_eq!(line_column(src, 0), (1, 1));
        assert_eq!(line_column(src, 2), (1, 3));
        assert_eq!(line_column(src, 4), (2, 1));
        assert_eq!(line_column(src, 8), (3, 1));
        assert_eq!(line_column(src, 11), (3, 4));
    }

    #[test]
    fn diagnostic_round_trip_unknown_class() {
        let err = Error::UnknownClass {
            class: "overflow-auto".into(),
            span: Span::new(10, 23),
            hint: Some("gpui has no `Overflow::Auto`. Use `overflow-y-scroll`.".into()),
        };
        let diag = Diagnostic::from_error(&err, "<div class=\"overflow-auto\">", Some("x.gpui.html"));
        let json = serde_json::to_string(&diag).unwrap();
        let back: Diagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(diag, back);
        assert_eq!(diag.code, "UnknownClass");
        assert_eq!(diag.literal, "overflow-auto");
        assert_eq!(diag.span, (10, 23));
        assert!(diag.hint.is_some());
    }

    #[test]
    fn diagnostic_for_each_error_variant_serializes() {
        let span = Span::new(0, 1);
        let cases = [
            Error::Parse(crate::ParseError {
                kind: crate::ParseErrorKind::UnbalancedTag {
                    expected: "div".into(),
                    found: "span".into(),
                },
                span,
                message: "unbalanced tag".into(),
            }),
            Error::UnknownTag {
                tag: "table".into(),
                span,
            },
            Error::UnknownClass {
                class: "wat".into(),
                span,
                hint: None,
            },
            Error::UnsupportedAttribute {
                attr: "onclick".into(),
                span,
            },
            Error::UnknownThemeToken {
                token: "magenta".into(),
                span,
            },
            Error::InvalidEventHandler {
                name: "1bad".into(),
                span,
            },
            Error::InvalidInterpolation {
                raw: "$".into(),
                span,
            },
            Error::Codegen(crate::CodegenError {
                span,
                message: "oops".into(),
            }),
        ];
        for err in cases {
            let diag = Diagnostic::from_error(&err, "x", None);
            let json = serde_json::to_string(&diag).unwrap();
            let back: Diagnostic = serde_json::from_str(&json).unwrap();
            assert_eq!(diag, back, "round trip failed for {}", err.code());
        }
    }
}
