//! gpuiHTML parser (stage 1).
//!
//! gpuiHTML is a strict subset of HTML5 *for UI tags* (`<div>`, `<span>`)
//! plus a thin **document compatibility layer** so a complete HTML
//! document can be fed to the compiler verbatim. Wrappers and metadata
//! are recognised but produce no GPUI elements; only `<body>`'s UI
//! subtree reaches codegen.
//!
//! Tag categories:
//!
//! - **UI tags** (`div`, `span`): strict subset, attribute values must be
//!   double-quoted, only `class` and `id` accepted, balanced tags
//!   required. Unknown UI-shaped tags (`table`, `section`, ...) still
//!   surface as `UnknownTag`.
//!
//! - **Document wrappers** (`html`, `head`, `body`): parsed but
//!   transparent — their children flatten into the parent node list, so
//!   `<html><body><div/></body></html>` collapses to the same AST as a
//!   bare `<div/>`. Wrapper attrs are consumed and discarded.
//!
//! - **Metadata** (`meta`, `link`, `title`): parsed and dropped. `meta`
//!   and `link` are void-like (no closing tag required). `title` is a
//!   raw-text tag — its content is consumed verbatim until `</title>`.
//!
//! - **Raw-text skip** (`script`): content is consumed as raw text (so
//!   `<` inside JS doesn't try to start a tag) and discarded. gpuiHTML
//!   never executes the script.
//!
//! - **Raw-text preserve** (`style`): content is consumed as raw text
//!   and stored on a `Node::Style { css, span }` for the static-CSS
//!   lowering pipeline (#27 wires it through; this stage only captures).
//!
//! Tag-name comparisons for the document-compat categories are
//! case-insensitive (`<HTML>` and `<html>` both work). UI-tag matching
//! stays case-sensitive: `<DIV>` is `UnknownTag`, write lowercase.
//!
//! There is no HTML5 error recovery: a missing quote, an unbalanced
//! close tag, or an unclosed `<script>` is a fatal compile error with
//! a span pointing at the offending source.

use crate::ast::{Attr, ClassToken, Element, Node, Span, StyleNode, TextNode};
use crate::{Error, ParseError, ParseErrorKind};

/// UI tags accepted by the parser. Anything outside this list (and the
/// document-compat tags below) surfaces as `UnknownTag`.
///
/// `div` and `span` lower to their literal gpui constructors. The
/// remaining tags (`p`, `h1`, `h2`, `h3`, `button`) all lower to
/// `div()` with tag-specific default method calls emitted before the
/// user's class chain — see `codegen::tag_constructor_and_defaults`.
/// `img` / `icon` / `slot` from the spec table are still deferred:
/// they need asset / component-registry / runtime support that v0.1
/// doesn't yet model.
const SUPPORTED_TAGS: &[&str] = &["div", "span", "p", "h1", "h2", "h3", "button"];
const SUPPORTED_ATTRS: &[&str] = &["class", "id"];

const WRAPPER_TAGS: &[&str] = &["html", "head", "body"];
const VOID_METADATA_TAGS: &[&str] = &["meta", "link"];
const RAW_TEXT_SKIP_TAGS: &[&str] = &["script", "title"];
const RAW_TEXT_PRESERVE_TAG: &str = "style";

pub fn parse(src: &str) -> Result<Vec<Node>, Error> {
    let mut p = Parser::new(src);
    p.parse_document()
}

struct Parser<'src> {
    src: &'src str,
    pos: usize,
}

impl<'src> Parser<'src> {
    fn new(src: &'src str) -> Self {
        Self { src, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_str(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_ws_and_comments(&mut self) -> Result<(), Error> {
        loop {
            self.skip_whitespace();
            if self.peek_str("<!--") {
                self.skip_comment()?;
            } else {
                return Ok(());
            }
        }
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn parse_document(&mut self) -> Result<Vec<Node>, Error> {
        let mut nodes = Vec::new();
        self.skip_ws_and_comments()?;
        // `<!DOCTYPE ...>` is only meaningful at the document level; consume
        // and discard it before the main loop so it doesn't try to parse
        // as a normal tag.
        if self.peek_str_ci("<!doctype") {
            self.skip_doctype()?;
            self.skip_ws_and_comments()?;
        }
        while !self.at_eof() {
            // A `</...` at document root has no matching open, so it's a
            // hard error rather than a "let the parent handle it" sentinel.
            // (Without this guard parse_one returns empty, pos doesn't
            // advance, and the loop spins forever.)
            if self.peek_str("</") {
                return Err(self.consume_unexpected_closing_tag()?);
            }
            nodes.extend(self.parse_one()?);
            self.skip_ws_and_comments()?;
        }
        Ok(nodes)
    }

    /// Caller has confirmed `</` is at `self.pos`. Consume the tag name and
    /// (if present) trailing `>` so the position advances past the offending
    /// run, then return the structured error.
    fn consume_unexpected_closing_tag(&mut self) -> Result<Error, Error> {
        let start = self.pos;
        self.pos += 2;
        let (tag, ident_span) = self.parse_identifier("expected close tag name")?;
        // Best-effort: also consume up to and including '>' if it's right
        // there, so the span covers the full `</foo>` literal.
        self.skip_whitespace();
        if self.peek() == Some('>') {
            self.advance();
        }
        let span = ident_span.merge(Span::new(start, self.pos));
        Ok(Error::Parse(ParseError {
            kind: ParseErrorKind::UnexpectedClosingTag { tag: tag.clone() },
            span,
            message: format!("unexpected closing tag </{tag}> with no matching open"),
        }))
    }

    /// Parse one logical position in the source and return the nodes it
    /// produces (zero, one, or many).
    ///
    /// The return is `Vec<Node>` rather than `Option<Node>` because
    /// document wrappers (`<html>`, `<head>`, `<body>`) flatten — their
    /// children replace the wrapper itself in the parent's node list.
    /// Comments, metadata tags, and `<script>` produce empty vectors.
    /// Normal elements and text produce single-node vectors. `<style>`
    /// produces a single `Node::Style`.
    ///
    /// `</...` is left untouched: callers either expect it (an
    /// element's children loop, a wrapper's children loop) or treat it
    /// as a top-level error (`parse_document`).
    fn parse_one(&mut self) -> Result<Vec<Node>, Error> {
        if self.peek_str("<!--") {
            self.skip_comment()?;
            return Ok(Vec::new());
        }
        if self.peek_str_ci("<!doctype") {
            // Stray DOCTYPE inside an element body is unusual but cheap
            // to tolerate: skip it the same way `parse_document` would.
            self.skip_doctype()?;
            return Ok(Vec::new());
        }
        if self.peek_str("</") {
            // Caller's close tag — leave it.
            return Ok(Vec::new());
        }
        if self.peek() != Some('<') {
            return Ok(self.parse_text().into_iter().collect());
        }

        // We're at `<` followed by something that should be a tag name.
        // Peek the name (case-preserving) and dispatch on a lowercased
        // copy so HTML-typed `<HTML>`/`<Body>` work on the document-
        // compatibility side. UI tags stay case-sensitive — that's
        // enforced inside `parse_element` itself.
        let Some(name) = self.peek_tag_name() else {
            // `<` not followed by a name — let `parse_element` produce
            // the existing structured error.
            let element = self.parse_element()?;
            return Ok(vec![Node::Element(element)]);
        };
        let name_lower = name.to_ascii_lowercase();

        if WRAPPER_TAGS.contains(&name_lower.as_str()) {
            return self.parse_wrapper(&name_lower);
        }
        if VOID_METADATA_TAGS.contains(&name_lower.as_str()) {
            self.parse_void_metadata()?;
            return Ok(Vec::new());
        }
        if RAW_TEXT_SKIP_TAGS.contains(&name_lower.as_str()) {
            self.skip_raw_text_tag(&name_lower)?;
            return Ok(Vec::new());
        }
        if name_lower == RAW_TEXT_PRESERVE_TAG {
            let style = self.parse_style_tag()?;
            return Ok(vec![Node::Style(style)]);
        }

        // Normal UI element (or unknown UI tag — `parse_element`
        // produces `UnknownTag` for anything outside SUPPORTED_TAGS).
        let element = self.parse_element()?;
        Ok(vec![Node::Element(element)])
    }

    fn parse_element(&mut self) -> Result<Element, Error> {
        let start = self.pos;
        debug_assert_eq!(self.peek(), Some('<'));
        self.advance(); // consume '<'

        let (tag, tag_span) = self.parse_identifier("expected tag name")?;
        if !SUPPORTED_TAGS.contains(&tag.as_str()) {
            return Err(Error::UnknownTag {
                tag,
                span: tag_span,
            });
        }

        let mut classes = Vec::new();
        let mut attrs = Vec::new();

        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('>') => {
                    self.advance();
                    break;
                }
                Some('/') => {
                    if !self.peek_str("/>") {
                        return Err(self.err(
                            ParseErrorKind::InvalidCharacter('/'),
                            self.pos,
                            "expected '/>' for self-closing tag",
                        ));
                    }
                    self.pos += 2;
                    let span = Span::new(start, self.pos);
                    return Ok(Element {
                        tag,
                        tag_span,
                        classes,
                        attrs,
                        children: Vec::new(),
                        span,
                    });
                }
                Some(_) => {
                    let attr = self.parse_attr()?;
                    if attr.name == "class" {
                        classes.extend(split_classes(&attr.value, attr.value_span));
                    } else if SUPPORTED_ATTRS.contains(&attr.name.as_str()) {
                        attrs.push(attr);
                    } else {
                        return Err(Error::UnsupportedAttribute {
                            attr: attr.name,
                            span: attr.name_span,
                        });
                    }
                }
                None => {
                    return Err(self.err(
                        ParseErrorKind::EofInTag,
                        self.pos,
                        "unexpected EOF inside open tag",
                    ));
                }
            }
        }

        let mut children = Vec::new();
        loop {
            if self.peek_str("</") {
                break;
            }
            if self.at_eof() {
                return Err(Error::Parse(ParseError {
                    kind: ParseErrorKind::UnclosedTag,
                    span: tag_span,
                    message: format!("unclosed tag <{tag}>"),
                }));
            }
            children.extend(self.parse_one()?);
        }

        // close tag
        debug_assert!(self.peek_str("</"));
        self.pos += 2;
        let (close_name, close_span) = self.parse_identifier("expected close tag name")?;
        if close_name != tag {
            return Err(Error::Parse(ParseError {
                kind: ParseErrorKind::UnbalancedTag {
                    expected: tag.clone(),
                    found: close_name.clone(),
                },
                span: close_span,
                message: format!("expected </{tag}>, found </{close_name}>"),
            }));
        }
        self.skip_whitespace();
        if self.peek() != Some('>') {
            return Err(self.err(
                ParseErrorKind::InvalidCharacter('>'),
                self.pos,
                "expected '>' to close end tag",
            ));
        }
        self.advance();

        Ok(Element {
            tag,
            tag_span,
            classes,
            attrs,
            children,
            span: Span::new(start, self.pos),
        })
    }

    fn parse_identifier(&mut self, ctx: &str) -> Result<(String, Span), Error> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' {
                self.advance();
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.err(
                ParseErrorKind::InvalidCharacter(self.peek().unwrap_or('\0')),
                start,
                ctx,
            ));
        }
        let s = self.src[start..self.pos].to_string();
        Ok((s, Span::new(start, self.pos)))
    }

    fn parse_attr(&mut self) -> Result<Attr, Error> {
        let (name, name_span) = self.parse_identifier("expected attribute name")?;
        self.skip_whitespace();
        if self.peek() != Some('=') {
            return Err(self.err(
                ParseErrorKind::InvalidCharacter('='),
                self.pos,
                "expected '=' after attribute name",
            ));
        }
        self.advance();
        self.skip_whitespace();
        let quote_pos = self.pos;
        match self.peek() {
            Some('"') => {} // happy path, fall through
            Some('\'') => {
                return Err(self.err(
                    ParseErrorKind::SingleQuotedAttrValue,
                    quote_pos,
                    "attribute values must be double-quoted (gpuiHTML rejects single quotes)",
                ));
            }
            Some(_) => {
                return Err(self.err(
                    ParseErrorKind::UnquotedAttrValue,
                    quote_pos,
                    "attribute values must be double-quoted",
                ));
            }
            None => {
                return Err(self.err(
                    ParseErrorKind::EofInTag,
                    quote_pos,
                    "unexpected EOF after `=` — expected a double-quoted value",
                ));
            }
        }
        self.advance(); // consume opening "
        let value_start = self.pos;
        loop {
            match self.peek() {
                Some('"') => {
                    let value_span = Span::new(value_start, self.pos);
                    let value = self.src[value_start..self.pos].to_string();
                    self.advance(); // consume closing "
                    return Ok(Attr {
                        name,
                        name_span,
                        value,
                        value_span,
                    });
                }
                Some(_) => {
                    self.advance();
                }
                None => {
                    return Err(self.err(
                        ParseErrorKind::UnclosedAttribute,
                        quote_pos,
                        "unclosed attribute value (missing closing `\"`)",
                    ));
                }
            }
        }
    }

    /// v0.1 simplification: trim leading/trailing whitespace; drop if empty.
    /// Inline elements with significant inter-element whitespace are out of
    /// scope for the minimal slice — see spec Roadmap.
    fn parse_text(&mut self) -> Option<Node> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '<' {
                break;
            }
            self.advance();
        }
        let raw = &self.src[start..self.pos];
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Node::Text(TextNode {
            text: trimmed.to_string(),
            span: Span::new(start, self.pos),
        }))
    }

    fn skip_comment(&mut self) -> Result<(), Error> {
        debug_assert!(self.peek_str("<!--"));
        let start = self.pos;
        self.pos += 4;
        loop {
            if self.peek_str("-->") {
                self.pos += 3;
                return Ok(());
            }
            if self.at_eof() {
                return Err(Error::Parse(ParseError {
                    kind: ParseErrorKind::UnexpectedEof,
                    span: Span::new(start, self.pos),
                    message: "unclosed comment".into(),
                }));
            }
            self.advance();
        }
    }

    fn err(&self, kind: ParseErrorKind, pos: usize, msg: &str) -> Error {
        Error::Parse(ParseError {
            kind,
            span: Span::new(pos, (pos + 1).min(self.src.len())),
            message: msg.to_string(),
        })
    }

    // ---------- document-compatibility helpers ---------------------------

    /// ASCII-case-insensitive variant of `peek_str`. Used for matching
    /// HTML5-style tag-name prefixes (e.g. `<!DOCTYPE` vs `<!doctype`)
    /// without copying the buffer.
    fn peek_str_ci(&self, s: &str) -> bool {
        let remaining = &self.src.as_bytes()[self.pos..];
        if remaining.len() < s.len() {
            return false;
        }
        remaining[..s.len()].eq_ignore_ascii_case(s.as_bytes())
    }

    /// Peek the tag name after `<` without consuming any input. Returns
    /// the name preserving case, or `None` if the next char isn't `<`
    /// or no identifier character follows.
    fn peek_tag_name(&self) -> Option<String> {
        if self.peek() != Some('<') {
            return None;
        }
        let after_lt = self.pos + 1;
        let bytes = self.src.as_bytes();
        let mut end = after_lt;
        while end < bytes.len() {
            let b = bytes[end];
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':' {
                end += 1;
            } else {
                break;
            }
        }
        if end == after_lt {
            return None;
        }
        Some(self.src[after_lt..end].to_string())
    }

    /// Consume `<!DOCTYPE ...>` (case-insensitive). Caller has confirmed
    /// `<!doctype` at the current position. Anything between the
    /// keyword and `>` is discarded — the v0.1 compiler doesn't care
    /// about doctype variants beyond "is this an HTML5 document".
    fn skip_doctype(&mut self) -> Result<(), Error> {
        let start = self.pos;
        // Consume `<!`, the DOCTYPE keyword, and everything up to `>`.
        // Rather than re-validate the keyword shape (caller already
        // matched it case-insensitively), just scan forward.
        while let Some(c) = self.peek() {
            if c == '>' {
                self.advance();
                return Ok(());
            }
            self.advance();
        }
        Err(Error::Parse(ParseError {
            kind: ParseErrorKind::UnexpectedEof,
            span: Span::new(start, self.pos),
            message: "unclosed `<!DOCTYPE` declaration".into(),
        }))
    }

    /// Parse `<wrapper [attrs]> children </wrapper>` and return the
    /// flattened children. The wrapper itself doesn't make it into the
    /// AST. Wrapper attrs are consumed and discarded — gpuiHTML's
    /// codegen has no place to attach `<html lang="ja">` and friends.
    fn parse_wrapper(&mut self, name_lower: &str) -> Result<Vec<Node>, Error> {
        let open_start = self.pos;
        debug_assert_eq!(self.peek(), Some('<'));
        self.advance(); // consume `<`
        let (_actual_name, _name_span) = self.parse_identifier("expected wrapper tag name")?;
        // Caller's `peek_tag_name` already confirmed this matches one of
        // WRAPPER_TAGS case-insensitively; no need to re-check.
        self.consume_attrs_until_close()?;

        let mut children: Vec<Node> = Vec::new();
        loop {
            if self.peek_str("</") {
                break;
            }
            if self.at_eof() {
                return Err(Error::Parse(ParseError {
                    kind: ParseErrorKind::UnclosedTag,
                    span: Span::new(open_start, self.pos),
                    message: format!("unclosed wrapper <{name_lower}>"),
                }));
            }
            children.extend(self.parse_one()?);
        }

        // Close tag: case-insensitive name match against the wrapper.
        debug_assert!(self.peek_str("</"));
        self.pos += 2;
        let (close_name, close_span) = self.parse_identifier("expected close tag name")?;
        if !close_name.eq_ignore_ascii_case(name_lower) {
            return Err(Error::Parse(ParseError {
                kind: ParseErrorKind::UnbalancedTag {
                    expected: name_lower.to_string(),
                    found: close_name.clone(),
                },
                span: close_span,
                message: format!("expected </{name_lower}>, found </{close_name}>"),
            }));
        }
        self.skip_whitespace();
        if self.peek() != Some('>') {
            return Err(self.err(
                ParseErrorKind::InvalidCharacter('>'),
                self.pos,
                "expected '>' to close wrapper end tag",
            ));
        }
        self.advance();
        Ok(children)
    }

    /// Parse `<meta ...>` or `<link ...>` — both are void in HTML5 and
    /// take no closing tag. Attrs are parsed loosely (any `name`,
    /// `name="value"`, or `name='value'` pair) and discarded.
    fn parse_void_metadata(&mut self) -> Result<(), Error> {
        debug_assert_eq!(self.peek(), Some('<'));
        self.advance();
        let (_name, _span) = self.parse_identifier("expected void metadata tag name")?;
        self.consume_attrs_until_close()
    }

    /// Parse `<script ...>...</script>` or `<title ...>...</title>`,
    /// consuming the body as raw text and discarding it. The body may
    /// contain `<` characters that don't start tags (e.g. JS expressions
    /// using `<` for comparison). Termination is the first
    /// case-insensitive `</name` followed by `>` or whitespace.
    fn skip_raw_text_tag(&mut self, name_lower: &str) -> Result<(), Error> {
        let open_start = self.pos;
        debug_assert_eq!(self.peek(), Some('<'));
        self.advance();
        let (_name, _span) = self.parse_identifier("expected raw-text tag name")?;
        self.consume_attrs_until_close()?;
        self.consume_raw_text_until_close(name_lower, open_start)
            .map(|_| ())
    }

    /// Parse `<style ...>...</style>` and capture the raw CSS body for
    /// the static-CSS lowering pipeline (#27 reads the `Node::Style`
    /// AST node downstream; this stage doesn't parse the CSS itself).
    fn parse_style_tag(&mut self) -> Result<StyleNode, Error> {
        let open_start = self.pos;
        debug_assert_eq!(self.peek(), Some('<'));
        self.advance();
        let (_name, _span) = self.parse_identifier("expected <style> tag name")?;
        self.consume_attrs_until_close()?;
        // `consume_attrs_until_close` consumes past the `>`, so the
        // current position is the first byte of CSS content.
        let content_start = self.pos;
        let (css, end_pos) =
            self.consume_raw_text_until_close(RAW_TEXT_PRESERVE_TAG, open_start)?;
        Ok(StyleNode {
            css,
            span: Span::new(open_start, end_pos),
            content_start,
        })
    }

    /// Shared raw-text body consumer. Reads from the current position
    /// (just past the open tag's `>`) up to and including the matching
    /// `</name>` close tag (case-insensitive on the name).
    /// Returns the raw body text and the byte offset just after `>`.
    fn consume_raw_text_until_close(
        &mut self,
        name_lower: &str,
        open_start: usize,
    ) -> Result<(String, usize), Error> {
        let body_start = self.pos;
        let close_marker = format!("</{name_lower}");
        loop {
            if self.at_eof() {
                return Err(Error::Parse(ParseError {
                    kind: ParseErrorKind::UnclosedTag,
                    span: Span::new(open_start, self.pos),
                    message: format!("unclosed <{name_lower}> — missing </{name_lower}>"),
                }));
            }
            if self.peek_str_ci(&close_marker) {
                let after_marker = self.pos + close_marker.len();
                // Verify what's right after `</name` is `>` or
                // whitespace — otherwise it's just text that happens to
                // start with `</nameSOMETHING_ELSE`.
                let next_byte = self.src.as_bytes().get(after_marker).copied();
                let is_close = matches!(
                    next_byte,
                    Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
                );
                if is_close {
                    let body = self.src[body_start..self.pos].to_string();
                    self.pos = after_marker;
                    self.skip_whitespace();
                    if self.peek() != Some('>') {
                        return Err(self.err(
                            ParseErrorKind::InvalidCharacter('>'),
                            self.pos,
                            "expected '>' to close raw-text end tag",
                        ));
                    }
                    self.advance();
                    return Ok((body, self.pos));
                }
            }
            self.advance();
        }
    }

    /// Loose attribute consumer for document-compat tags. Walks
    /// `name="value"` / `name='value'` / bare-`name` pairs without
    /// validating against `SUPPORTED_ATTRS`. Stops at `>` or `/>` and
    /// consumes the closer. Used by wrappers, void metadata, and
    /// raw-text tags — UI elements still go through the strict
    /// `parse_attr` path.
    fn consume_attrs_until_close(&mut self) -> Result<(), Error> {
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('>') => {
                    self.advance();
                    return Ok(());
                }
                Some('/') if self.peek_str("/>") => {
                    self.pos += 2;
                    return Ok(());
                }
                Some('/') => {
                    return Err(self.err(
                        ParseErrorKind::InvalidCharacter('/'),
                        self.pos,
                        "expected '/>' to close self-closing tag",
                    ));
                }
                Some(_) => {
                    self.consume_loose_attr()?;
                }
                None => {
                    return Err(self.err(
                        ParseErrorKind::EofInTag,
                        self.pos,
                        "unexpected EOF inside open tag",
                    ));
                }
            }
        }
    }

    /// One-shot loose attribute consumer. Accepts:
    ///   `name`              (HTML5 boolean attribute)
    ///   `name="value"`      (standard form)
    ///   `name='value'`      (single-quoted — accepted for document-
    ///                       compat tags only, unlike the strict parser)
    fn consume_loose_attr(&mut self) -> Result<(), Error> {
        let _ = self.parse_identifier("expected attribute name")?;
        self.skip_whitespace();
        if self.peek() != Some('=') {
            // Bare attribute — HTML5 allows boolean attrs without values
            // (e.g. `<script async>`, `<input disabled>`).
            return Ok(());
        }
        self.advance();
        self.skip_whitespace();
        let quote = match self.peek() {
            Some('"') => '"',
            Some('\'') => '\'',
            Some(_) => {
                return Err(self.err(
                    ParseErrorKind::UnquotedAttrValue,
                    self.pos,
                    "attribute value must be quoted",
                ));
            }
            None => {
                return Err(self.err(
                    ParseErrorKind::EofInTag,
                    self.pos,
                    "unexpected EOF after `=`",
                ));
            }
        };
        let quote_pos = self.pos;
        self.advance();
        loop {
            match self.peek() {
                Some(c) if c == quote => {
                    self.advance();
                    return Ok(());
                }
                Some(_) => {
                    self.advance();
                }
                None => {
                    return Err(self.err(
                        ParseErrorKind::UnclosedAttribute,
                        quote_pos,
                        "unclosed attribute value",
                    ));
                }
            }
        }
    }
}

/// Split a `class="..."` value into individual [`ClassToken`]s, each with
/// its own span pointing into the original source.
fn split_classes(value: &str, value_span: Span) -> Vec<ClassToken> {
    let mut out = Vec::new();
    let mut byte_offset = 0usize;
    while byte_offset < value.len() {
        // Skip whitespace
        let trimmed_left = &value[byte_offset..];
        let leading_ws = trimmed_left.len() - trimmed_left.trim_start().len();
        byte_offset += leading_ws;
        if byte_offset >= value.len() {
            break;
        }
        let rest = &value[byte_offset..];
        let tok_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let raw = &rest[..tok_len];
        let start = value_span.start + byte_offset;
        let end = start + tok_len;
        out.push(ClassToken {
            raw: raw.to_string(),
            span: Span::new(start, end),
        });
        byte_offset += tok_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Vec<Node> {
        parse(src).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
    }

    #[test]
    fn empty_input_is_empty_tree() {
        assert!(parse_ok("").is_empty());
        assert!(parse_ok("   \n  ").is_empty());
    }

    #[test]
    fn comment_only() {
        assert!(parse_ok("<!-- hello -->").is_empty());
    }

    #[test]
    fn single_div() {
        let nodes = parse_ok("<div></div>");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            Node::Element(e) => {
                assert_eq!(e.tag, "div");
                assert!(e.classes.is_empty());
                assert!(e.children.is_empty());
            }
            _ => panic!("expected element"),
        }
    }

    #[test]
    fn div_with_class_list() {
        let nodes = parse_ok(r#"<div class="flex flex-col gap-2"></div>"#);
        let Node::Element(e) = &nodes[0] else {
            panic!("element");
        };
        assert_eq!(
            e.classes.iter().map(|c| c.raw.as_str()).collect::<Vec<_>>(),
            vec!["flex", "flex-col", "gap-2"]
        );
        // Spans are populated and disjoint.
        for window in e.classes.windows(2) {
            assert!(window[0].span.end <= window[1].span.start);
        }
        // Verify span content matches the original source.
        let src = r#"<div class="flex flex-col gap-2"></div>"#;
        for c in &e.classes {
            assert_eq!(&src[c.span.start..c.span.end], c.raw);
        }
    }

    #[test]
    fn nested_tree_with_text() {
        let src = r#"<div class="flex flex-col">
  <span>Hello</span>
  <div class="text-muted">World</div>
</div>"#;
        let nodes = parse_ok(src);
        assert_eq!(nodes.len(), 1);
        let Node::Element(root) = &nodes[0] else {
            panic!()
        };
        assert_eq!(root.tag, "div");
        assert_eq!(root.classes.len(), 2);
        assert_eq!(root.children.len(), 2);

        let Node::Element(span) = &root.children[0] else {
            panic!()
        };
        assert_eq!(span.tag, "span");
        assert_eq!(span.children.len(), 1);
        let Node::Text(t) = &span.children[0] else {
            panic!()
        };
        assert_eq!(t.text, "Hello");

        let Node::Element(inner) = &root.children[1] else {
            panic!()
        };
        assert_eq!(inner.tag, "div");
        assert_eq!(inner.classes.len(), 1);
        assert_eq!(inner.classes[0].raw, "text-muted");
    }

    #[test]
    fn id_attribute_kept_as_attr() {
        let nodes = parse_ok(r#"<div id="root"></div>"#);
        let Node::Element(e) = &nodes[0] else {
            panic!()
        };
        assert_eq!(e.attrs.len(), 1);
        assert_eq!(e.attrs[0].name, "id");
        assert_eq!(e.attrs[0].value, "root");
    }

    #[test]
    fn unbalanced_tag_is_an_error() {
        let src = r#"<div class="flex"><span>oops</div>"#;
        let err = parse(src).unwrap_err();
        match err {
            Error::Parse(pe) => match pe.kind {
                ParseErrorKind::UnbalancedTag { expected, found } => {
                    assert_eq!(expected, "span");
                    assert_eq!(found, "div");
                }
                other => panic!("expected UnbalancedTag, got {other:?}"),
            },
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tag_is_caught_at_parse_time() {
        // Anything outside the supported UI tag set + document-compat
        // wrapper set rejects. `<table>` is in the spec but explicitly
        // v0.2 territory; `<section>` / `<article>` etc. aren't in the
        // spec at all.
        for raw in [
            "<table></table>",
            "<section></section>",
            "<article></article>",
        ] {
            let err = parse(raw).unwrap_err();
            match err {
                Error::UnknownTag { .. } => {}
                other => panic!("expected UnknownTag for {raw:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn semantic_ui_tags_parse_with_class_and_children() {
        // p / h1..h3 / button parse as elements with the usual class
        // and child semantics. The codegen-side default-method
        // emission is exercised in codegen::tests.
        for raw in [
            "<p>text</p>",
            "<h1>Title</h1>",
            "<h2 class=\"text-muted\">Sub</h2>",
            "<h3>Three</h3>",
            "<button class=\"bg-accent text-accent-foreground\">Click</button>",
        ] {
            let nodes = parse_ok(raw);
            assert_eq!(nodes.len(), 1, "for input {raw:?}");
            let Node::Element(e) = &nodes[0] else {
                panic!("expected element for {raw:?}");
            };
            // Element tags survive in the AST verbatim; codegen looks
            // them up to pick the constructor + defaults.
            assert!(
                matches!(e.tag.as_str(), "p" | "h1" | "h2" | "h3" | "button"),
                "unexpected tag {:?}",
                e.tag
            );
        }
    }

    #[test]
    fn unsupported_attribute_is_rejected() {
        let err = parse(r#"<div style="color: red"></div>"#).unwrap_err();
        match err {
            Error::UnsupportedAttribute { attr, .. } => assert_eq!(attr, "style"),
            other => panic!("expected UnsupportedAttribute, got {other:?}"),
        }
    }

    #[test]
    fn unquoted_attribute_value_is_an_error() {
        let err = parse(r#"<div class=flex></div>"#).unwrap_err();
        match err {
            Error::Parse(pe) => assert!(
                matches!(pe.kind, ParseErrorKind::UnquotedAttrValue),
                "expected UnquotedAttrValue, got {:?}",
                pe.kind
            ),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn single_quoted_attribute_value_is_a_distinct_error() {
        let err = parse(r#"<div class='flex'></div>"#).unwrap_err();
        match err {
            Error::Parse(pe) => assert!(
                matches!(pe.kind, ParseErrorKind::SingleQuotedAttrValue),
                "expected SingleQuotedAttrValue, got {:?}",
                pe.kind
            ),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_double_quoted_attribute_is_unclosed_attribute() {
        let err = parse(r#"<div class="flex"#).unwrap_err();
        match err {
            Error::Parse(pe) => assert!(
                matches!(pe.kind, ParseErrorKind::UnclosedAttribute),
                "expected UnclosedAttribute, got {:?}",
                pe.kind
            ),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn eof_inside_open_tag_is_eof_in_tag() {
        let err = parse("<div ").unwrap_err();
        match err {
            Error::Parse(pe) => assert!(
                matches!(pe.kind, ParseErrorKind::EofInTag),
                "expected EofInTag, got {:?}",
                pe.kind
            ),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn top_level_closing_tag_is_unexpected_closing_tag() {
        let err = parse("</div>").unwrap_err();
        match err {
            Error::Parse(pe) => match pe.kind {
                ParseErrorKind::UnexpectedClosingTag { tag } => assert_eq!(tag, "div"),
                other => panic!("expected UnexpectedClosingTag, got {other:?}"),
            },
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn closing_tag_before_open_in_sibling_position_is_unexpected_closing() {
        // The first thing the parser sees is `</span>`, which has no
        // matching open. This must surface as UnexpectedClosingTag, not
        // as an infinite loop or a misleading UnbalancedTag.
        let err = parse("</span><div></div>").unwrap_err();
        match err {
            Error::Parse(pe) => match pe.kind {
                ParseErrorKind::UnexpectedClosingTag { tag } => assert_eq!(tag, "span"),
                other => panic!("expected UnexpectedClosingTag, got {other:?}"),
            },
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn unexpected_closing_tag_carries_source_span() {
        let src = "</div>";
        let err = parse(src).unwrap_err();
        let span = err.span();
        assert_eq!(&src[span.start..span.end], "</div>");
    }

    #[test]
    fn self_closing_element() {
        let nodes = parse_ok(r#"<div class="x"/>"#);
        let Node::Element(e) = &nodes[0] else {
            panic!()
        };
        assert!(e.children.is_empty());
        assert_eq!(e.classes.len(), 1);
    }

    // ---------- issue #26: full HTML document ingestion -------------------

    #[test]
    fn parse_skips_doctype() {
        // Both spellings the spec accepts.
        for src in [
            "<!DOCTYPE html><div></div>",
            "<!doctype html><div></div>",
            "<!DOCTYPE HTML><div></div>",
            "<!DOCTYPE html SYSTEM \"about:legacy-compat\"><div></div>",
        ] {
            let nodes = parse_ok(src);
            assert_eq!(
                nodes.len(),
                1,
                "doctype should be skipped, leaving one element: {src:?}"
            );
            let Node::Element(e) = &nodes[0] else {
                panic!("expected element after doctype in {src:?}");
            };
            assert_eq!(e.tag, "div");
        }
    }

    #[test]
    fn malformed_unclosed_doctype_errors() {
        // Missing `>` — the parser should reach EOF and produce a
        // structured error rather than silently consuming the rest of
        // the document.
        let err = parse("<!DOCTYPE html").unwrap_err();
        assert!(
            matches!(err, Error::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn parse_accepts_html_head_body_wrappers() {
        // Wrappers flatten transparently — this should produce the same
        // single-element AST as a bare `<div></div>`.
        let nodes = parse_ok("<!DOCTYPE html><html><head></head><body><div></div></body></html>");
        assert_eq!(nodes.len(), 1, "wrappers should flatten to body's children");
        let Node::Element(e) = &nodes[0] else {
            panic!("expected single element");
        };
        assert_eq!(e.tag, "div");
    }

    #[test]
    fn wrapper_attrs_are_consumed_and_discarded() {
        // `<html lang="ja">` and `<body class="...">` etc. — wrappers
        // accept arbitrary attrs in HTML5; the parser must consume them
        // without choking. Wrapper attrs don't reach the AST.
        let nodes = parse_ok(r#"<html lang="ja"><body class="page"><div></div></body></html>"#);
        assert_eq!(nodes.len(), 1);
        let Node::Element(e) = &nodes[0] else {
            panic!()
        };
        assert_eq!(e.tag, "div");
        // The body's `class="page"` does not reach the inner div.
        assert!(
            e.classes.is_empty(),
            "wrapper attrs must not leak onto children"
        );
    }

    #[test]
    fn wrapper_tag_names_match_case_insensitively() {
        // HTML-typed `<HTML>` / `<Body>` should also work.
        let nodes = parse_ok("<HTML><BODY><div></div></BODY></HTML>");
        assert_eq!(nodes.len(), 1);
        let Node::Element(e) = &nodes[0] else {
            panic!()
        };
        assert_eq!(e.tag, "div");
    }

    #[test]
    fn parse_skips_meta_link_title() {
        // <meta>/<link> are void; <title> is raw-text. None reach the AST.
        let nodes = parse_ok(
            r#"<html><head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width">
                <link rel="preconnect" href="https://example.com" />
                <title>ato Desktop</title>
            </head><body><div></div></body></html>"#,
        );
        assert_eq!(nodes.len(), 1, "metadata tags must not produce nodes");
        let Node::Element(e) = &nodes[0] else {
            panic!()
        };
        assert_eq!(e.tag, "div");
    }

    #[test]
    fn parse_skips_script_raw_text() {
        // Script body contains `<` characters that a non-raw-text
        // parser would treat as tag starts. The parser must consume
        // the body as raw text. (Note: a literal `</script>` *inside*
        // a string would still terminate the script per the HTML5
        // raw-text rules — that's the well-known footgun and not
        // something v0.1 fixes; users escape `<\/script>` in JS.)
        let nodes = parse_ok(
            r#"<html><head><script>if (a < b) { alert("hi"); }</script></head><body><div></div></body></html>"#,
        );
        assert_eq!(nodes.len(), 1, "script must not produce nodes");
        let Node::Element(e) = &nodes[0] else {
            panic!()
        };
        assert_eq!(e.tag, "div");
    }

    #[test]
    fn script_body_is_not_parsed_as_gpui_html() {
        // Even pathological JS — broken HTML inside a string, broken
        // syntax — should pass through raw-text scanning. The first
        // matching `</script>` ends it (HTML5-standard footgun).
        let src = r#"<script>var x = "</div><span>oops"; var y = 5 < 3;</script>"#;
        let nodes = parse_ok(src);
        assert!(
            nodes.is_empty(),
            "script must produce no nodes; got {nodes:?}"
        );
    }

    #[test]
    fn malformed_unclosed_script_errors() {
        let err = parse("<script>console.log('hi')").unwrap_err();
        assert!(
            matches!(err, Error::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn parse_preserves_style_raw_text_node() {
        // Style content reaches the AST as Node::Style with the verbatim
        // CSS source. #27 will read this; this PR only captures.
        let src = r#"<html><head><style>.foo { color: red; }</style></head><body><div></div></body></html>"#;
        let nodes = parse_ok(src);
        // After flattening: [Style, Element(div)] (style sits between
        // head's children and body's children in source order).
        assert_eq!(nodes.len(), 2);
        let Node::Style(style) = &nodes[0] else {
            panic!("expected Style node first, got {nodes:?}");
        };
        assert_eq!(style.css, ".foo { color: red; }");
        // The span covers the whole `<style>...</style>` block in source.
        let span_text = &src[style.span.start..style.span.end];
        assert!(span_text.starts_with("<style"));
        assert!(span_text.ends_with("</style>"));

        let Node::Element(e) = &nodes[1] else {
            panic!()
        };
        assert_eq!(e.tag, "div");
    }

    #[test]
    fn style_body_is_not_parsed_as_gpui_html() {
        // Style content can contain `<` (e.g. attribute selectors,
        // `:not()`, or even just stray characters in declarations).
        // Verify the parser doesn't try to recurse into them.
        let src = r#"<style>.a > .b { content: "<div>"; }</style>"#;
        let nodes = parse_ok(src);
        assert_eq!(nodes.len(), 1);
        let Node::Style(style) = &nodes[0] else {
            panic!("expected Style node");
        };
        assert_eq!(style.css, r#".a > .b { content: "<div>"; }"#);
    }

    #[test]
    fn malformed_unclosed_style_errors() {
        let err = parse("<style>.foo { color: red; ").unwrap_err();
        assert!(
            matches!(err, Error::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn raw_text_close_marker_must_be_a_tag_not_a_substring() {
        // `</scripts>` (note the trailing 's') and `</scripted>` are
        // not the close tag for `<script>`. Verify the close-marker
        // detection demands `>` or whitespace after the name, so these
        // false positives don't terminate the raw text early.
        let nodes = parse_ok("<script>let s = '</scripts>'; let t = '</scripted>';</script>");
        assert!(
            nodes.is_empty(),
            "script must absorb the false-positive markers"
        );
    }
}
