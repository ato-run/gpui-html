# SKILL.md — Working on gpui-html

A working guide for agents and contributors. Not a tutorial — read this before
making changes so the project's boundaries and judgment calls don't have to be
re-derived from the spec each time.

The repository's specification lives in `docs/spec.md` and is the source of
truth for what gpuiHTML accepts and lowers. This file is the operational
companion: which calls are allowed, which are forbidden, and the quality gates
every change must clear.

## gpui-html の役割

- gpui-html は **browser engine ではない**.
- DOM / CSSOM / JS runtime / browser layout は実装しない.
- full HTML を input として受け取る. HTML5 boilerplate は document
  compatibility layer として吸収する.
- 静的な UI tree と静的な `<style>` rule を GPUI builder Rust chain に
  lower する static compiler である.
- 出力は単一の dense なメソッドチェーン式 (rustfmt はユーザ側の責務).

ここを踏み外すと「軽い GPUI を捨てて小型ブラウザを書き始める」方向に
ドリフトする. issue / PR を増やす前に必ず spec の **目的 / 非目的**
セクションに照らす.

## 入力モデル

| Tag category | Behaviour |
|---|---|
| `<!DOCTYPE>` | Parsed and skipped. Case-insensitive keyword. |
| `<html>`, `<head>`, `<body>` | Transparent wrappers. Children flatten into parent. Attrs consumed and discarded. Case-insensitive tag-name match. |
| `<meta>`, `<link>`, `<title>` | Metadata. Parsed and dropped. `meta`/`link` are void-like. `title` is raw-text. |
| `<script>` | Raw-text skip. Body consumed verbatim, **JS never executed**. |
| `<style>` | Raw-text preserve on `Node::Style { css, span, content_start }`. Handed off to the CSS pipeline. |
| UI tags (currently `<div>`, `<span>`) | Strict subset. Case-sensitive. Unknown UI-shaped tags (`table`, `section`, `button`, ...) → `UnknownTag`. |

UI-tag-side rules:

- Attribute values must be double-quoted. Single-quoted values reject as
  `SingleQuotedAttrValue`; unquoted as `UnquotedAttrValue`.
- Only `class` and `id` are accepted on UI tags. Other attribute names →
  `UnsupportedAttribute`.
- Tags must balance. There is no HTML5 error recovery.

Do not casually expand the UI tag set. Adding `<button>`, `<img>`, `<input>`
etc. requires:

1. Spec amendment with the matching GPUI builder lowering and rationale.
2. Test coverage in `parse::tests` and `codegen::tests`.
3. Update to this file's table.

## CSS 方針

`<style>` content is parsed in `crates/gpui-html-core/src/css.rs` as a
**class-selector-only subset**. CSS is not a cascade engine here — it's an
alternative MethodCall source.

| | Treatment |
|---|---|
| `.foo` (single class selector) | Lowered to MethodCalls. |
| `.foo:hover`, `.foo .bar`, `.foo > .bar`, `.foo.bar`, `.foo, .bar`, `#root`, `div`, `*`, `::-webkit-*` | **Lenient skip.** Rule body consumed up to matching `}`, nothing reaches the IR. |
| `@media`, `@keyframes`, `@import`, `@charset` | **Lenient skip.** Brace-balanced for `{...}` form; `;`-terminated for the statement form. |
| Property in the spec's lowering table (see `lower_declaration`) | Lowered. |
| Property not in the table (e.g. `font-family`, vendor prefixes) | **Lenient skip per-declaration.** Other declarations in the same rule still apply. |
| Out-of-range value (e.g. `padding: 1.7rem`) | **Lenient skip per-declaration.** |
| `var(--theme-<token>)` | Lowered via `class_map::normalize_theme_token` (same helper the utility-class path uses). |
| Direct color literals (`red`, `#fff`, `rgb(...)`) | **Lenient skip.** Host theme is the source of truth. |
| Mismatched braces, missing `:` in a declaration, EOF mid-rule | **Hard fail** (`MalformedRule` / `MalformedDeclaration`). |

Integration order with utility classes:

1. **Phase 1** — for each class in `class="..."` source order, emit MethodCalls
   from the matching CSS rule (if any).
2. **Phase 2** — for each class in source order, emit utility-class
   lowering. Classes covered only by a CSS rule (no utility match) don't
   error.

Net: **utility class wins over CSS rule** when both touch the same Style
field, because gpui builder semantics make later calls override earlier
ones.

## theme token 方針

| Surface | Shape | Result |
|---|---|---|
| Utility class | `bg-<token>` / `text-<token>` / `border-<token>` | `.bg(theme.<token>)` etc. |
| CSS declaration | `color: var(--theme-<token>)` etc. | Same `theme.<token>` Rust ident |
| Hyphenated multi-segment | `text-accent-foreground`, `var(--theme-accent-foreground)` | `theme.accent_foreground` (hyphen → underscore) |
| Palette pattern (numeric last segment) | `bg-red-500`, `text-blue-300` | **Reject** with palette hint. Same rule on both surfaces. |
| `bg-<token>/<alpha>` (slash alpha) | `bg-accent/10` | **Not lowered.** Reject with hint pointing at #23. Don't packed-RGBA. |
| `bg-transparent` | literal | `.bg(gpui::transparent_black())`. Not a theme.transparent lookup. |

The `class_map::normalize_theme_token` helper is the single source of truth
for the disambiguation rule (palette vs theme token, hyphen-to-snake
normalization). Utility and CSS both call it.

Do not introduce host-app-specific color knowledge into `gpui-html-core`.
The compiler knows token names; rustc validates the field exists when the
generated code is compiled against the host's `Theme` struct. No packed
RGBA, no font-family stacks baked in, no Tailwind palette.

## class lowering 方針

- `crates/gpui-html-core/src/class_map.rs` is the central lowering table.
- `lower_classes_with_styles` is the public entry: utility + CSS rules
  merged in one pass.
- `lower_one(tok) -> Result<Option<MethodCall>, Error>`. `Ok(None)` is a
  **recognized no-op** — accepted for Tailwind preview compatibility but
  emits no builder call. v0.1 uses this for `font-sans` only.
- Source order of MethodCalls is preserved. Later builder calls override
  earlier ones (gpui builder semantics).
- App-shell compatibility tokens (`max-w-128` etc.) are documented
  exemptions, not a general extension mechanism. New ones require a spec
  amendment.

Currently lowered (growing toward the full spec table at `docs/spec.md` §
class 対応範囲):

- Flex direction / sizing / alignment / justification
- Spacing (p/m/gap with axis + side variants, scale `{0..=12, 16, 20, 24,
  32}`)
- Size (w/h/size + min/max, fractions, viewport keywords, `w-full`, `w-auto`)
- Typography (text-{xs..3xl}, font-{thin..black}, italic, line-through,
  truncate, leading-\*, line-clamp-N)
- Border (numeric + directional widths, dashed, theme color)
- Overflow / Cursor / Opacity
- Radius (named suffixes, sides, corners) + Shadow
- Color via theme tokens (`bg-<token>`, `text-<token>`, `border-<token>`)
  and the `bg-transparent` literal

Out of v0.1 scope (deferred to v0.2 manifest or follow-up):

- Theme-token alpha (`bg-<token>/<n>`) — issue #23
- Custom-scale sizing manifest beyond `max-w-128` — issue #19 follow-up
- Per-element font-family beyond `font-sans` no-op

## diagnostics 方針

- Every error variant carries a `Span` covering the offending source
  range.
- CSS errors carry **absolute** spans (translated from the CSS body's
  `content_start` to byte offsets in the original HTML document) so
  consumers don't special-case "offset within `<style>`".
- The wire schema (`crate::diagnostic::Diagnostic`) is stable across v0.1
  patches. Editors and CI consume it via `gpui-html check --format json`
  (NDJSON to stderr).
- Lenient skip (CSS) does not currently surface as a warning channel —
  the user reads `docs/spec.md` to know what's supported. A future
  collect-and-warn upgrade is a clean follow-up (see "残り follow-up"
  below).
- Malformed structure (parse errors, mismatched braces) always **hard
  fails**. Lenient mode is only for unsupported but well-formed input.

## 実装時の禁止事項

- **Do not** hand-write "compiled output" and present it as tool output.
  Run `cargo run -p gpui-html -- compile <file>` and use the actual result.
- **Do not** require callers to narrow their input to a stripped subtree.
  Full HTML is the canonical input. The document compatibility layer
  handles `<!DOCTYPE>` / `<html>` / `<head>` / `<body>` / `<script>` /
  `<style>`.
- **Do not** embed host-app-specific color values, font stacks, or other
  theme particulars in `gpui-html-core`. The compiler is intentionally
  ignorant of the host's `Theme` struct contents.
- **Do not** assume the compiler can resolve a theme token to an RGBA
  value. It can't. Packed-RGBA lowering requires a v0.2 manifest design.
- **Do not** attempt to convert JS behavior to Rust. `<script>` content
  is raw-text-skip and never reaches the IR.
- **Do not** add arbitrary Tailwind utilities without a spec entry. The
  spec table is the gate; lowering follows the spec, not the other way
  around.
- **Do not** silently expand the UI tag set. Anything beyond `<div>` /
  `<span>` needs spec + tests + this file's table updated.

## 変更時の quality gate

Every change must pass:

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p gpui-html -- check examples/hello.html
cargo run -p gpui-html -- compile examples/hello.html
```

If `examples/ato-desktop-preview.gpui.html` is present (issue #9 fixture):

```sh
cargo run -p gpui-html -- check examples/ato-desktop-preview.gpui.html
cargo run -p gpui-html -- compile examples/ato-desktop-preview.gpui.html
```

`hello.html` output must remain **byte-identical** unless the change
explicitly updates its snapshot test. The byte-identical contract is
the regression guard for the v0.1 vertical slice.

## acceptance fixture

`examples/ato-desktop-preview.gpui.html` is the **#9 acceptance fixture**.
It is the original full HTML Ato Desktop preview, byte-identical to the
input the project was designed around. Rules:

- It stays full HTML — DOCTYPE, html, head with meta/link/script/style,
  body, UI tree. **No stripping. No manual substitutions.**
- A snapshot test in `crates/gpui-html-core/src/codegen.rs` pins the
  compiled output. Any change that affects the output must update the
  snapshot in the same PR.
- The browser-only `<style>` rules (resets, `::-webkit-scrollbar`) are
  expected to silently skip under the lenient CSS mode. That behavior is
  load-bearing for the fixture — don't tighten it back to strict-fail
  without a plan for what replaces the silent skip.

When the fixture is in the repo, `cargo run -p gpui-html -- check
examples/ato-desktop-preview.gpui.html` must exit 0 on every commit to
main.

## 残り follow-up

These are real gaps the project has knowingly left open. Pick them up
intentionally, not by accident.

- **Collect-and-warn diagnostic channel** for lenient CSS skips. Today
  the silent skip is documented; a `--lint` mode that emits one
  `Diagnostic` per skipped rule would close the visibility gap without
  reverting to strict-fail.
- **Pretty-printer / multi-line output mode** for the emitted gpui code.
  v0.1 emits one dense line; `gpui-html compile --pretty` could rustfmt
  with aggressive chain-wrap settings.
- **Theme manifest / custom tokens**. Host apps declare their full token
  set; the compiler reads it and validates names. Subsumes the
  `max-w-128` exemption, theme-token alpha (#23), and font-family
  per-element decisions.
- **Broader UI tag support**. `<button>`, `<p>`, `<h1..3>`, `<img>`,
  `<slot>` are all in the spec but not yet lowered. Component tags
  (`<UpperCaseTag />`) and `$expr` interpolation are spec'd as v0.1
  surface but deferred.
- **CI workflow**. Issue #8 still open. Adding GitHub Actions running
  `cargo test --workspace` + fmt + clippy on PRs is mechanical and
  unblocks contributor PRs.
- **`<input>` and form-shaped elements** — explicitly v0.2 in the spec
  due to state / focus / IME / on_input integration weight.
