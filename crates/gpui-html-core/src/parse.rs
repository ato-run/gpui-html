//! gpuiHTML parser (stage 1).
//!
//! gpuiHTML is a strict subset of HTML5: tags must be balanced, attribute
//! values must be double-quoted, and only the elements/attributes listed in
//! the spec are recognised. There is no HTML5 error recovery — a missing
//! quote or an unbalanced close tag is a fatal compile error.
//!
//! The v0.1 vertical slice supports `<div>`, `<span>`, `class="..."`,
//! `id="..."`, text children, and `<!-- ... -->` comments. Anything else
//! is rejected with a structured [`Error`] carrying a [`Span`].

use crate::ast::{Attr, ClassToken, Element, Node, Span, TextNode};
use crate::{Error, ParseError, ParseErrorKind};

const SUPPORTED_TAGS: &[&str] = &["div", "span"];
const SUPPORTED_ATTRS: &[&str] = &["class", "id"];

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
        while !self.at_eof() {
            // A `</...` at document root has no matching open, so it's a
            // hard error rather than a "let the parent handle it" sentinel.
            // (Without this guard parse_node returns None, pos doesn't
            // advance, and the loop spins forever.)
            if self.peek_str("</") {
                return Err(self.consume_unexpected_closing_tag()?);
            }
            if let Some(node) = self.parse_node()? {
                nodes.push(node);
            }
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

    /// Returns `Ok(None)` for whitespace-only text (skipped) and for comments.
    /// Callers that want to detect `</` at the document root must check for
    /// it themselves before calling this; here `</` means "end of parent
    /// element" and is left in the buffer for `parse_element` to consume.
    fn parse_node(&mut self) -> Result<Option<Node>, Error> {
        if self.peek_str("<!--") {
            self.skip_comment()?;
            return Ok(None);
        }
        if self.peek_str("</") {
            // Reached parent's close tag — caller handles it.
            return Ok(None);
        }
        if self.peek() == Some('<') {
            let element = self.parse_element()?;
            return Ok(Some(Node::Element(element)));
        }
        Ok(self.parse_text())
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
            if let Some(node) = self.parse_node()? {
                children.push(node);
            }
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
        let err = parse("<table></table>").unwrap_err();
        match err {
            Error::UnknownTag { tag, .. } => assert_eq!(tag, "table"),
            other => panic!("expected UnknownTag, got {other:?}"),
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
}
