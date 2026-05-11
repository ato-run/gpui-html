//! Static `<style>` parsing and lowering.
//!
//! Picks up the [`crate::ast::StyleNode`] entries the document parser
//! produces (PR #28 / issue #26) and turns the v0.1 CSS subset into the
//! same `MethodCall` IR `class_map.rs` already produces from utility
//! classes. The point isn't to be a CSS engine — gpuiHTML rejects
//! most of CSS by design — but to give authors *one consistent path*:
//! write Tailwind in `class=`, write per-component rules in `<style>`,
//! both compile to the same gpui builder calls.
//!
//! v0.1 subset
//! ============
//!
//! Selectors:
//!   `.foo` only. Anything else (pseudo-classes, combinators,
//!   compound, comma-lists, id, type, universal, at-rules) raises a
//!   structured `UnsupportedSelector` diagnostic.
//!
//! Declarations: see [`lower_declaration`] for the full table.
//! Highlights:
//!   - layout: display, flex-direction, align-items, justify-content
//!   - spacing: gap, padding[-side], margin[-side] (rem only)
//!   - sizing: width / height = `100%` / `auto`
//!   - color: color / background-color / border-color via
//!     `var(--theme-<token>)`
//!   - typography: font-weight (numeric or keyword)
//!   - misc: overflow[-axis], cursor: pointer, opacity
//!
//! Theme tokens use the `var(--theme-<token>)` shape rather than
//! Tailwind utility names because CSS custom-property names allow
//! hyphens freely — `var(--theme-accent-foreground)` is unambiguous,
//! whereas `text-accent-foreground` (the Tailwind shape) is the open
//! design question in #7.
//!
//! Span model
//! ==========
//!
//! Every span returned in the public IR is **absolute** — translated
//! from the CSS source's local offsets to byte positions in the
//! original HTML document via the `base_offset` argument to
//! [`parse_stylesheet`]. Diagnostics can therefore be rendered the
//! same way as parse errors from the HTML parser, with no special-
//! casing for "offset within `<style>`".

use std::collections::HashMap;

use crate::ast::Span;
use crate::class_map::{MethodCall, StyleMap};
use crate::{CssError, CssErrorKind, Error};

// ---------- IR types ---------------------------------------------------

/// Parsed `<style>` content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

/// One rule: `.<class> { <declarations> }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub class_name: String,
    pub class_name_span: Span,
    pub declarations: Vec<Declaration>,
    /// Span covering the entire rule from `.` to `}`.
    pub span: Span,
}

/// One `property: value` pair inside a rule. Trailing `;` is consumed
/// but not part of the span (matches the convention authors use when
/// underlining declarations in editors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: String,
    pub property_span: Span,
    pub value: String,
    pub value_span: Span,
    /// Span covering the property + `:` + value.
    pub span: Span,
}

// ---------- public entry points ---------------------------------------

/// Parse CSS source into the v0.1 IR. `base_offset` is the byte offset
/// in the original document where this CSS body begins (i.e. the
/// `content_start` of the corresponding `StyleNode`).
pub fn parse_stylesheet(css: &str, base_offset: usize) -> Result<Stylesheet, Error> {
    let mut p = Parser::new(css, base_offset);
    p.parse_stylesheet()
}

/// Lower a parsed stylesheet into the class -> MethodCalls map that
/// codegen consumes.
///
/// Hard fails on the first declaration that can't be lowered. Future
/// work may switch this to a collect-and-warn model so unsupported
/// rules don't block the rest of the document from compiling, but for
/// v0.1 the strict-fail behavior keeps the diagnostic story simple.
pub fn lower_stylesheet(sheet: &Stylesheet) -> Result<StyleMap, Error> {
    let mut map: StyleMap = HashMap::new();
    for rule in &sheet.rules {
        let mut calls = Vec::new();
        for decl in &rule.declarations {
            calls.extend(lower_declaration(decl)?);
        }
        map.entry(rule.class_name.clone())
            .or_default()
            .extend(calls);
    }
    Ok(map)
}

/// Convenience: parse and lower in one call.
pub fn parse_and_lower(css: &str, base_offset: usize) -> Result<StyleMap, Error> {
    let sheet = parse_stylesheet(css, base_offset)?;
    lower_stylesheet(&sheet)
}

// ---------- parser ----------------------------------------------------

struct Parser<'src> {
    src: &'src str,
    pos: usize,
    base_offset: usize,
}

impl<'src> Parser<'src> {
    fn new(src: &'src str, base_offset: usize) -> Self {
        Self {
            src,
            pos: 0,
            base_offset,
        }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// Translate a local (CSS-relative) byte range to an absolute span
    /// in the original document.
    fn span(&self, local_start: usize, local_end: usize) -> Span {
        Span::new(self.base_offset + local_start, self.base_offset + local_end)
    }

    /// Skip whitespace and CSS comments (`/* ... */`).
    fn skip_ws_and_comments(&mut self) -> Result<(), Error> {
        loop {
            // whitespace
            while let Some(c) = self.peek() {
                if c.is_whitespace() {
                    self.advance();
                } else {
                    break;
                }
            }
            if self.peek() == Some('/') && self.src[self.pos..].starts_with("/*") {
                let comment_start = self.pos;
                self.pos += 2;
                loop {
                    if self.at_eof() {
                        return Err(Error::Css(CssError {
                            kind: CssErrorKind::MalformedRule,
                            span: self.span(comment_start, self.pos),
                            message: "unclosed CSS comment".into(),
                        }));
                    }
                    if self.src[self.pos..].starts_with("*/") {
                        self.pos += 2;
                        break;
                    }
                    self.advance();
                }
            } else {
                return Ok(());
            }
        }
    }

    fn parse_stylesheet(&mut self) -> Result<Stylesheet, Error> {
        let mut rules = Vec::new();
        self.skip_ws_and_comments()?;
        while !self.at_eof() {
            // At-rules (`@media`, `@keyframes`, etc.) and selector forms
            // we don't support both surface as `UnsupportedSelector` so
            // diagnostics point at the same span as a normal rule head.
            if self.peek() == Some('@') {
                return Err(self.unsupported_selector_at_pos("at-rules"));
            }
            rules.push(self.parse_rule()?);
            self.skip_ws_and_comments()?;
        }
        Ok(Stylesheet { rules })
    }

    fn parse_rule(&mut self) -> Result<Rule, Error> {
        let rule_start = self.pos;
        let (class_name, class_name_span) = self.parse_class_selector()?;
        self.skip_ws_and_comments()?;
        if self.peek() != Some('{') {
            return Err(Error::Css(CssError {
                kind: CssErrorKind::MalformedRule,
                span: self.span(self.pos, self.pos.saturating_add(1)),
                message: "expected `{` to open rule body".into(),
            }));
        }
        self.advance(); // consume `{`

        let mut declarations = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            match self.peek() {
                Some('}') => {
                    self.advance();
                    break;
                }
                None => {
                    return Err(Error::Css(CssError {
                        kind: CssErrorKind::MalformedRule,
                        span: self.span(rule_start, self.pos),
                        message: "unclosed CSS rule — missing `}`".into(),
                    }));
                }
                _ => {
                    declarations.push(self.parse_declaration()?);
                }
            }
        }

        Ok(Rule {
            class_name,
            class_name_span,
            declarations,
            span: self.span(rule_start, self.pos),
        })
    }

    /// Parse a single class selector (`.foo` / `.foo-bar` / `.foo_1`).
    /// Anything more elaborate (pseudo, combinator, compound, list,
    /// id, type, universal) raises `UnsupportedSelector`.
    fn parse_class_selector(&mut self) -> Result<(String, Span), Error> {
        let start = self.pos;
        if self.peek() != Some('.') {
            return Err(
                self.unsupported_selector_at_pos("expected a class selector starting with `.`")
            );
        }
        self.advance(); // `.`
        let name_start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                self.advance();
            } else {
                break;
            }
        }
        if self.pos == name_start {
            return Err(
                self.unsupported_selector_at_pos("class selector must have a name after `.`")
            );
        }
        let class_name = self.src[name_start..self.pos].to_string();

        // Anything but whitespace or `{` after the class name means we
        // hit a feature the v0.1 subset doesn't allow — pseudo-classes
        // (`:hover`), compound (`.foo.bar`), descendant combinator (` .bar`
        // — but the lookahead check happens before we eat whitespace, so
        // descendants get caught here too).
        let after_name = self.pos;
        let bad_char = self.peek().filter(|c| !c.is_whitespace() && *c != '{');
        if let Some(c) = bad_char {
            // Read ahead until whitespace / `{` so the diagnostic shows
            // the full offending suffix rather than just one char.
            let mut tail_end = after_name;
            for ch in self.src[after_name..].chars() {
                if ch.is_whitespace() || ch == '{' {
                    break;
                }
                tail_end += ch.len_utf8();
            }
            let full_selector = &self.src[start..tail_end];
            let kind_msg = match c {
                ':' => "pseudo-class / pseudo-element",
                '.' => "compound selector (`.foo.bar`)",
                '#' => "id selector",
                ',' => "selector list (`.foo, .bar`)",
                '>' | '+' | '~' => "combinator",
                _ => "selector form",
            };
            return Err(Error::Css(CssError {
                kind: CssErrorKind::UnsupportedSelector {
                    selector: full_selector.to_string(),
                },
                span: self.span(start, tail_end),
                message: format!(
                    "v0.1 supports only single class selectors (`.foo`); \
                     {kind_msg} is out of scope"
                ),
            }));
        }

        // Even if the next non-whitespace char is `{`, we have to make
        // sure there's no descendant combinator hiding in the
        // whitespace — `\.foo .bar { ... }` is descendant.
        let probe_start = self.pos;
        let mut probe = self.pos;
        while probe < self.src.len() {
            let ch = self.src[probe..].chars().next().unwrap();
            if ch == '{' {
                break;
            }
            if !ch.is_whitespace() {
                // Found another selector token after whitespace — that's a
                // descendant combinator (or worse).
                let mut tail = probe;
                for c in self.src[probe..].chars() {
                    if c.is_whitespace() || c == '{' {
                        break;
                    }
                    tail += c.len_utf8();
                }
                let full = &self.src[start..tail];
                return Err(Error::Css(CssError {
                    kind: CssErrorKind::UnsupportedSelector {
                        selector: full.to_string(),
                    },
                    span: self.span(start, tail),
                    message: "v0.1 supports only single class selectors (`.foo`); \
                              descendant combinator is out of scope"
                        .into(),
                }));
            }
            probe += ch.len_utf8();
        }
        // No bad char found; whitespace-only between selector and `{`.
        let _ = probe_start; // silence unused warning when feature gates change.

        Ok((class_name, self.span(start, after_name)))
    }

    fn parse_declaration(&mut self) -> Result<Declaration, Error> {
        let decl_start = self.pos;
        let prop_start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '-' {
                self.advance();
            } else {
                break;
            }
        }
        if self.pos == prop_start {
            return Err(Error::Css(CssError {
                kind: CssErrorKind::MalformedDeclaration,
                span: self.span(self.pos, self.pos.saturating_add(1)),
                message: "expected property name".into(),
            }));
        }
        let property = self.src[prop_start..self.pos].to_string();
        let property_span = self.span(prop_start, self.pos);

        self.skip_ws_and_comments()?;
        if self.peek() != Some(':') {
            return Err(Error::Css(CssError {
                kind: CssErrorKind::MalformedDeclaration,
                span: self.span(decl_start, self.pos),
                message: format!("expected `:` after property `{property}`"),
            }));
        }
        self.advance(); // `:`
        self.skip_ws_and_comments()?;

        // Value runs until `;` or `}`. We tolerate `var(...)` and other
        // function-shaped values by tracking paren depth so a `;`
        // inside `var(...)` doesn't terminate the value early.
        let value_start = self.pos;
        let mut depth = 0u32;
        loop {
            match self.peek() {
                None => {
                    return Err(Error::Css(CssError {
                        kind: CssErrorKind::MalformedDeclaration,
                        span: self.span(decl_start, self.pos),
                        message: format!(
                            "unclosed declaration `{property}: ...` — missing `;` or `}}`"
                        ),
                    }));
                }
                Some('(') => {
                    depth += 1;
                    self.advance();
                }
                Some(')') => {
                    depth = depth.saturating_sub(1);
                    self.advance();
                }
                Some(';') if depth == 0 => {
                    let value_end = self.pos;
                    self.advance(); // consume `;`
                    return Ok(Declaration {
                        property,
                        property_span,
                        value: self.src[value_start..value_end].trim().to_string(),
                        value_span: self.span(value_start, value_end),
                        span: self.span(decl_start, value_end),
                    });
                }
                Some('}') if depth == 0 => {
                    // Allow the last declaration in a rule to omit `;`.
                    let value_end = self.pos;
                    return Ok(Declaration {
                        property,
                        property_span,
                        value: self.src[value_start..value_end].trim().to_string(),
                        value_span: self.span(value_start, value_end),
                        span: self.span(decl_start, value_end),
                    });
                }
                Some(_) => {
                    self.advance();
                }
            }
        }
    }

    fn unsupported_selector_at_pos(&self, what: &str) -> Error {
        // Read forward to the first `{` or end of source so the
        // diagnostic span covers the whole offending selector head.
        let start = self.pos;
        let mut end = start;
        for c in self.src[start..].chars() {
            if c == '{' {
                break;
            }
            end += c.len_utf8();
        }
        let selector = self.src[start..end].trim_end().to_string();
        Error::Css(CssError {
            kind: CssErrorKind::UnsupportedSelector {
                selector: selector.clone(),
            },
            span: self.span(start, end),
            message: format!("unsupported selector — {what}"),
        })
    }
}

// ---------- declaration lowering --------------------------------------

/// Lower one CSS declaration to zero-or-more `MethodCall`s. Returns
/// an error if the property is unknown to the v0.1 subset, or if the
/// property is known but the value isn't on the supported scale.
pub fn lower_declaration(decl: &Declaration) -> Result<Vec<MethodCall>, Error> {
    let prop = decl.property.as_str();
    let val = decl.value.trim();

    let calls: Vec<MethodCall> = match prop {
        "display" => vec![match val {
            "flex" => MethodCall::nullary("flex"),
            "grid" => MethodCall::nullary("grid"),
            "block" => MethodCall::nullary("block"),
            "none" => MethodCall::nullary("hidden"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "flex-direction" => vec![match val {
            "row" => MethodCall::nullary("flex_row"),
            "column" => MethodCall::nullary("flex_col"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "flex-wrap" => vec![match val {
            "wrap" => MethodCall::nullary("flex_wrap"),
            "nowrap" => MethodCall::nullary("flex_nowrap"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "align-items" => vec![match val {
            "center" => MethodCall::nullary("items_center"),
            "flex-start" | "start" => MethodCall::nullary("items_start"),
            "flex-end" | "end" => MethodCall::nullary("items_end"),
            "baseline" => MethodCall::nullary("items_baseline"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "justify-content" => vec![match val {
            "center" => MethodCall::nullary("justify_center"),
            "flex-start" | "start" => MethodCall::nullary("justify_start"),
            "flex-end" | "end" => MethodCall::nullary("justify_end"),
            "space-between" => MethodCall::nullary("justify_between"),
            "space-around" => MethodCall::nullary("justify_around"),
            "space-evenly" => MethodCall::nullary("justify_evenly"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "gap" => vec![spacing_call("gap", val, decl.value_span)?],
        "padding" => vec![spacing_call("p", val, decl.value_span)?],
        "padding-top" => vec![spacing_call("pt", val, decl.value_span)?],
        "padding-right" => vec![spacing_call("pr", val, decl.value_span)?],
        "padding-bottom" => vec![spacing_call("pb", val, decl.value_span)?],
        "padding-left" => vec![spacing_call("pl", val, decl.value_span)?],
        "margin" => vec![spacing_call("m", val, decl.value_span)?],
        "margin-top" => vec![spacing_call("mt", val, decl.value_span)?],
        "margin-right" => vec![spacing_call("mr", val, decl.value_span)?],
        "margin-bottom" => vec![spacing_call("mb", val, decl.value_span)?],
        "margin-left" => vec![spacing_call("ml", val, decl.value_span)?],
        "width" => vec![sizing_keyword_call("w", val, decl.value_span)?],
        "height" => vec![sizing_keyword_call("h", val, decl.value_span)?],
        "color" => vec![theme_color_call("text_color", val, decl.value_span)?],
        "background-color" => vec![theme_color_call("bg", val, decl.value_span)?],
        "border-color" => vec![theme_color_call("border_color", val, decl.value_span)?],
        "font-weight" => vec![font_weight_call(val, decl.value_span)?],
        "overflow" => vec![match val {
            "hidden" => MethodCall::nullary("overflow_hidden"),
            "visible" => MethodCall::nullary("overflow_visible"),
            "scroll" => MethodCall::nullary("overflow_scroll"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "overflow-x" => vec![match val {
            "hidden" => MethodCall::nullary("overflow_x_hidden"),
            "scroll" => MethodCall::nullary("overflow_x_scroll"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "overflow-y" => vec![match val {
            "hidden" => MethodCall::nullary("overflow_y_hidden"),
            "scroll" => MethodCall::nullary("overflow_y_scroll"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "cursor" => vec![match val {
            "pointer" => MethodCall::nullary("cursor_pointer"),
            "default" => MethodCall::nullary("cursor_default"),
            "text" => MethodCall::nullary("cursor_text"),
            _ => return Err(unsupported_value(prop, val, decl.value_span)),
        }],
        "opacity" => vec![opacity_call(val, decl.value_span)?],
        _ => {
            return Err(Error::Css(CssError {
                kind: CssErrorKind::UnsupportedDeclaration {
                    property: prop.to_string(),
                },
                span: decl.property_span,
                message: format!("CSS property `{prop}` is not lowered in v0.1"),
            }));
        }
    };

    Ok(calls)
}

// ---------- value parsing helpers -------------------------------------

/// Resolve a CSS spacing value (rem or px) to a v0.1 spacing-scale step.
/// Accepts `0`, `<n>rem`, `<n>px` (16px = 1rem). Each step is 0.25rem.
fn parse_spacing_value(val: &str) -> Option<u32> {
    let v = val.trim();
    if v == "0" {
        return Some(0);
    }
    let (number_str, scale): (&str, f64) = if let Some(stripped) = v.strip_suffix("rem") {
        (stripped, 1.0)
    } else if let Some(stripped) = v.strip_suffix("px") {
        (stripped, 1.0 / 16.0)
    } else {
        return None;
    };
    let n: f64 = number_str.trim().parse().ok()?;
    if n < 0.0 {
        return None;
    }
    let raw = n * scale * 4.0; // each spacing step = 0.25rem
    let step = raw.round();
    if (raw - step).abs() > 0.01 {
        return None;
    }
    let step = step as u32;
    matches!(step, 0..=12 | 16 | 20 | 24 | 32).then_some(step)
}

fn spacing_call(prefix: &str, value: &str, span: Span) -> Result<MethodCall, Error> {
    let step = parse_spacing_value(value)
        .ok_or_else(|| unsupported_value_named(prefix_to_property(prefix), value, span))?;
    Ok(MethodCall::nullary(&format!("{prefix}_{step}")))
}

fn prefix_to_property(prefix: &str) -> &'static str {
    match prefix {
        "p" => "padding",
        "pt" => "padding-top",
        "pr" => "padding-right",
        "pb" => "padding-bottom",
        "pl" => "padding-left",
        "m" => "margin",
        "mt" => "margin-top",
        "mr" => "margin-right",
        "mb" => "margin-bottom",
        "ml" => "margin-left",
        "gap" => "gap",
        _ => "spacing",
    }
}

fn sizing_keyword_call(axis: &str, value: &str, span: Span) -> Result<MethodCall, Error> {
    let v = value.trim();
    let suffix = match v {
        "100%" | "full" => "full",
        "auto" => "auto",
        _ => {
            return Err(unsupported_value_named(
                if axis == "w" { "width" } else { "height" },
                v,
                span,
            ));
        }
    };
    Ok(MethodCall::nullary(&format!("{axis}_{suffix}")))
}

/// Resolve a `var(--theme-<token>)` reference to `theme.<rust_ident>`.
/// Hyphens in the token name become underscores so e.g.
/// `var(--theme-accent-foreground)` lowers to `theme.accent_foreground`.
/// This is the path that sidesteps the open question in #7 — CSS
/// custom-property names always allow hyphens, so the disambiguation
/// problem doesn't surface here.
fn parse_theme_var(value: &str) -> Option<String> {
    let v = value.trim();
    let inner = v.strip_prefix("var(")?.strip_suffix(')')?.trim();
    let token = inner.strip_prefix("--theme-")?;
    if token.is_empty() {
        return None;
    }
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(token.replace('-', "_"))
}

fn theme_color_call(method: &str, value: &str, span: Span) -> Result<MethodCall, Error> {
    let token = parse_theme_var(value).ok_or_else(|| {
        let prop = match method {
            "bg" => "background-color",
            "border_color" => "border-color",
            _ => "color",
        };
        Error::Css(CssError {
            kind: CssErrorKind::UnsupportedValue {
                property: prop.to_string(),
                value: value.to_string(),
            },
            span,
            message: format!(
                "color values must use `var(--theme-<token>)`; got `{value}`. \
                 Direct color literals (`#fff`, `red`, `rgb(...)`) are out of \
                 v0.1 scope so the host theme stays the source of truth."
            ),
        })
    })?;
    Ok(MethodCall::unary(method, format!("theme.{token}")))
}

fn font_weight_call(value: &str, span: Span) -> Result<MethodCall, Error> {
    let v = value.trim();
    let variant = match v {
        "100" | "thin" => "THIN",
        "300" | "light" => "LIGHT",
        "400" | "normal" => "NORMAL",
        "500" | "medium" => "MEDIUM",
        "600" | "semibold" => "SEMIBOLD",
        "700" | "bold" => "BOLD",
        "800" | "extrabold" | "extra-bold" => "EXTRA_BOLD",
        "900" | "black" => "BLACK",
        _ => return Err(unsupported_value_named("font-weight", v, span)),
    };
    Ok(MethodCall::unary(
        "font_weight",
        format!("FontWeight::{variant}"),
    ))
}

fn opacity_call(value: &str, span: Span) -> Result<MethodCall, Error> {
    let v = value.trim();
    // CSS opacity is 0..=1 (or a percentage). Tailwind/gpui's scale is
    // 0..=100. Map between them, rejecting out-of-range values.
    let pct: u32 = if let Some(p) = v.strip_suffix('%') {
        let n: u32 = p
            .trim()
            .parse()
            .ok()
            .ok_or_else(|| unsupported_value_named("opacity", v, span))?;
        if n > 100 {
            return Err(unsupported_value_named("opacity", v, span));
        }
        n
    } else {
        let n: f64 = v
            .parse()
            .ok()
            .ok_or_else(|| unsupported_value_named("opacity", v, span))?;
        if !(0.0..=1.0).contains(&n) {
            return Err(unsupported_value_named("opacity", v, span));
        }
        (n * 100.0).round() as u32
    };
    Ok(MethodCall::unary("opacity", format!("{pct}.0 / 100.0")))
}

fn unsupported_value(property: &str, value: &str, span: Span) -> Error {
    unsupported_value_named(property, value, span)
}

fn unsupported_value_named(property: &str, value: &str, span: Span) -> Error {
    Error::Css(CssError {
        kind: CssErrorKind::UnsupportedValue {
            property: property.to_string(),
            value: value.to_string(),
        },
        span,
        message: format!("CSS value `{value}` is not lowered for property `{property}` in v0.1"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(css: &str) -> Stylesheet {
        parse_stylesheet(css, 0).unwrap_or_else(|e| panic!("CSS parse failed: {e:?}"))
    }

    fn lower_ok(css: &str) -> StyleMap {
        parse_and_lower(css, 0).unwrap_or_else(|e| panic!("CSS lowering failed: {e:?}"))
    }

    fn methods_for(map: &StyleMap, class: &str) -> Vec<String> {
        map.get(class)
            .unwrap_or_else(|| panic!("class `.{class}` not in style map"))
            .iter()
            .map(|m| {
                if m.args.is_empty() {
                    m.name.clone()
                } else {
                    format!("{}({})", m.name, m.args.join(", "))
                }
            })
            .collect()
    }

    // ---------- parser tests ----------

    #[test]
    fn parse_class_selector_rule() {
        let sheet = parse_ok(".foo { display: flex; }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].class_name, "foo");
        assert_eq!(sheet.rules[0].declarations.len(), 1);
        assert_eq!(sheet.rules[0].declarations[0].property, "display");
        assert_eq!(sheet.rules[0].declarations[0].value, "flex");
    }

    #[test]
    fn parse_multiple_class_rules_in_source_order() {
        let sheet = parse_ok(
            ".foo { display: flex; }\n\
             .bar { color: var(--theme-primary); }",
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].class_name, "foo");
        assert_eq!(sheet.rules[1].class_name, "bar");
    }

    #[test]
    fn parse_handles_comments_and_missing_trailing_semicolon() {
        let sheet =
            parse_ok("/* leading */ .foo {\n  display: flex; /* inline */\n  gap: 0.5rem\n}");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(sheet.rules[0].declarations[1].property, "gap");
        assert_eq!(sheet.rules[0].declarations[1].value, "0.5rem");
    }

    #[test]
    fn reject_descendant_selector_with_hint() {
        let err = parse_stylesheet(".foo .bar { display: flex; }", 0).unwrap_err();
        match err {
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedSelector { selector },
                ..
            }) => {
                assert!(
                    selector.contains(".foo"),
                    "selector should include `.foo`, got {selector:?}"
                );
            }
            other => panic!("expected UnsupportedSelector, got {other:?}"),
        }
    }

    #[test]
    fn reject_pseudo_selector_with_hint() {
        let err = parse_stylesheet(".foo:hover { color: red; }", 0).unwrap_err();
        assert!(matches!(
            err,
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedSelector { .. },
                ..
            })
        ));
    }

    #[test]
    fn reject_compound_selector() {
        let err = parse_stylesheet(".foo.bar { color: red; }", 0).unwrap_err();
        assert!(matches!(
            err,
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedSelector { .. },
                ..
            })
        ));
    }

    #[test]
    fn reject_id_and_type_selectors() {
        for css in [
            "#root { color: red; }",
            "div { color: red; }",
            "* { color: red; }",
        ] {
            let err = parse_stylesheet(css, 0).unwrap_err();
            assert!(matches!(
                err,
                Error::Css(CssError {
                    kind: CssErrorKind::UnsupportedSelector { .. },
                    ..
                }),
            ));
        }
    }

    #[test]
    fn reject_at_rule_with_hint() {
        let err =
            parse_stylesheet("@media (min-width: 100px) { .foo { color: red; } }", 0).unwrap_err();
        assert!(matches!(
            err,
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedSelector { .. },
                ..
            })
        ));
    }

    #[test]
    fn reject_malformed_declaration() {
        // Missing `:` between property and value.
        let err = parse_stylesheet(".foo { color red }", 0).unwrap_err();
        assert!(matches!(
            err,
            Error::Css(CssError {
                kind: CssErrorKind::MalformedDeclaration,
                ..
            })
        ));
    }

    #[test]
    fn unsupported_selector_span_is_absolute() {
        // base_offset = 100 means CSS source starts 100 bytes into a
        // hypothetical HTML document. Diagnostic spans must be in
        // document coordinates, not CSS-local coordinates.
        let err = parse_stylesheet(".foo:hover { color: red; }", 100).unwrap_err();
        let span = err.span();
        assert!(
            span.start >= 100 && span.end > span.start,
            "expected span past base_offset 100, got {span:?}"
        );
    }

    // ---------- lowering tests ----------

    #[test]
    fn lower_display_flex_from_style_rule() {
        let map = lower_ok(".shell { display: flex; flex-direction: column; }");
        assert_eq!(methods_for(&map, "shell"), vec!["flex", "flex_col"]);
    }

    #[test]
    fn lower_flex_alignment_from_style_rule() {
        let map = lower_ok(".row { align-items: center; justify-content: space-between; }");
        assert_eq!(
            methods_for(&map, "row"),
            vec!["items_center", "justify_between"]
        );
    }

    #[test]
    fn lower_theme_var_color_from_style_rule() {
        let map = lower_ok(
            ".bar { \
               color: var(--theme-primary); \
               background-color: var(--theme-surface); \
               border-color: var(--theme-accent-foreground); \
             }",
        );
        // hyphenated theme tokens normalize to snake_case at the CSS
        // boundary (the design that #7 is debating for the utility
        // class side).
        assert_eq!(
            methods_for(&map, "bar"),
            vec![
                "text_color(theme.primary)",
                "bg(theme.surface)",
                "border_color(theme.accent_foreground)",
            ]
        );
    }

    #[test]
    fn lower_spacing_from_style_rule() {
        // 1rem == p_4 (each spacing step = 0.25rem). 16px == 1rem == p_4.
        let map = lower_ok(
            ".pad { padding: 1rem; padding-left: 0.5rem; gap: 0.5rem; margin-top: 16px; }",
        );
        assert_eq!(
            methods_for(&map, "pad"),
            vec!["p_4", "pl_2", "gap_2", "mt_4"]
        );
    }

    #[test]
    fn lower_overflow_cursor_opacity_from_style_rule() {
        let map = lower_ok(".util { overflow: hidden; cursor: pointer; opacity: 0.5; }");
        assert_eq!(
            methods_for(&map, "util"),
            vec!["overflow_hidden", "cursor_pointer", "opacity(50.0 / 100.0)"]
        );
    }

    #[test]
    fn lower_font_weight_keyword_or_numeric() {
        // Both Tailwind keywords and CSS numeric weights accepted.
        let map = lower_ok(".a { font-weight: bold; } .b { font-weight: 500; }");
        assert_eq!(
            methods_for(&map, "a"),
            vec!["font_weight(FontWeight::BOLD)"]
        );
        assert_eq!(
            methods_for(&map, "b"),
            vec!["font_weight(FontWeight::MEDIUM)"]
        );
    }

    #[test]
    fn lower_width_height_keywords() {
        let map = lower_ok(".full { width: 100%; height: auto; }");
        assert_eq!(methods_for(&map, "full"), vec!["w_full", "h_auto"]);
    }

    #[test]
    fn unsupported_css_declaration_reports_span() {
        // `font-family` isn't lowered. Span should point at the
        // property name for editor highlighting.
        let css = ".x { font-family: Inter; }";
        let err = parse_and_lower(css, 0).unwrap_err();
        match err {
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedDeclaration { property },
                span,
                ..
            }) => {
                assert_eq!(property, "font-family");
                assert_eq!(&css[span.start..span.end], "font-family");
            }
            other => panic!("expected UnsupportedDeclaration, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_css_value_reports_span() {
        // `padding: 1.7rem` is a valid property, invalid scale step.
        let css = ".x { padding: 1.7rem; }";
        let err = parse_and_lower(css, 0).unwrap_err();
        match err {
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedValue { property, value },
                ..
            }) => {
                assert_eq!(property, "padding");
                assert_eq!(value, "1.7rem");
            }
            other => panic!("expected UnsupportedValue, got {other:?}"),
        }
    }

    #[test]
    fn direct_color_literal_is_unsupported_value() {
        // `color: #fff` is rejected — host theme is the source of truth.
        let err = parse_and_lower(".x { color: #fff; }", 0).unwrap_err();
        assert!(matches!(
            err,
            Error::Css(CssError {
                kind: CssErrorKind::UnsupportedValue { .. },
                ..
            })
        ));
    }

    #[test]
    fn declarations_within_one_rule_preserve_source_order() {
        let map = lower_ok(".o { padding: 1rem; gap: 0.5rem; color: var(--theme-primary); }");
        assert_eq!(
            methods_for(&map, "o"),
            vec!["p_4", "gap_2", "text_color(theme.primary)"]
        );
    }

    #[test]
    fn same_class_in_multiple_rules_appends_in_source_order() {
        // A user could write the same class twice (not idiomatic but
        // legal). The lowering must concatenate, not replace.
        let map = lower_ok(".foo { padding: 1rem; }\n.foo { gap: 0.5rem; }");
        assert_eq!(methods_for(&map, "foo"), vec!["p_4", "gap_2"]);
    }
}
