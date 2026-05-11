# gpui-html

Write [gpui](https://www.gpui.rs/) UIs as HTML.

`gpui` is a fast, expressive Rust UI framework, but its builder API
(`div().flex().flex_col().gap_2().child(...)`) is verbose to write by hand
and hard for non-Rust tooling (designers, LLMs, codegen pipelines) to
produce. **gpuiHTML** is a constrained HTML-shaped markup language that
compiles down to that builder code via a small intermediate representation.

```
  .gpui.html  ──parse──▶  AST  ──class lower──▶  Style IR  ──codegen──▶  gpui Rust
```

Not a browser, not a DOM — gpuiHTML is "HTML for gpui," and only what
maps statically to `Styled` / `Style` is allowed. See **[docs/spec.md](docs/spec.md)**
for the v0.1 surface draft (tag table, class table, theme tokens,
diagnostics, codegen rules).

Status: **0.0.0 / scaffold.** The v0.1 surface is drafted and
intentionally constrained; parser and codegen are stubs. See
[Roadmap](#roadmap).

## Why three layers, not one

Splitting parser ↔ IR ↔ codegen lets each replace independently:

- A future proc-macro frontend can reuse the IR + codegen and skip the
  HTML parser entirely (taking Rust tokens instead).
- A future LSP / formatter can reuse the parser without pulling in codegen.
- The codegen mapping table (HTML tag + class → gpui method/field) is the
  single source of truth for "what gpuiHTML supports today" and is easy to
  diff against new gpui releases.

## Spec at a glance

```html
<div class="flex flex-col gap-3 p-4 rounded-xl bg-surface border border-border">
  <h2 class="text-lg font-semibold text-primary">Execution Plan</h2>
  <p  class="text-sm text-muted">
    This capsule requests permission to execute commands.
  </p>
  <button id="approve"
          class="h-9 px-4 rounded-md bg-accent text-accent-foreground"
          on:click="approveExecutionPlan">
    Approve
  </button>
</div>
```

compiles to:

```rust
div()
    .flex()
    .flex_col()
    .gap_3()
    .p_4()
    .rounded_xl()
    .bg(theme.surface)
    .border_1()
    .border_color(theme.border)
    .child(
        div()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.primary)
            .child("Execution Plan"),
    )
    .child(/* p */)
    .child(/* button */)
```

Three rules to keep in mind:

1. **Every class must lower to a `Styled` method or a `Style` field write.**
   No arbitrary CSS, no escape hatch. Unknown class → compile error.
2. **Colors are theme tokens only.** `bg-surface` ✅, `bg-red-500` ❌. The
   theme is supplied by the caller, not baked into the spec.
3. **`overflow-auto` does not exist** in gpui — use `overflow-y-scroll`.
   See the spec's "付録 A" for other Tailwind-isms that don't carry over.

## Examples

```sh
gpui-html compile examples/hello.html
gpui-html compile examples/ato-desktop-preview.gpui.html
```

- [`examples/hello.html`](examples/hello.html) — minimal smoke
  (vertical-slice contract). The pinned compile output is the v0.1
  vertical slice regression target.
- [`examples/ato-desktop-preview.gpui.html`](examples/ato-desktop-preview.gpui.html) —
  **issue #9 acceptance fixture**. The original mock Ato Desktop preview,
  byte-identical to the input the v0.1 surface was designed around:
  full HTML document with `<!DOCTYPE>`, `<html>`, `<head>` containing
  `<meta>` / `<link>` / `<title>` / `<script>` / `<style>`, and the
  `<body>` UI tree. **Not** Ato Desktop's production UI — a static
  acceptance target that exercises every category of the v0.1 lowering
  surface (document compatibility layer, lenient CSS, full utility
  table, hyphenated theme tokens, app-shell exemptions, no-op classes)
  in one input. The compiled output is pinned by an integration test
  (`crates/gpui-html-core/tests/ato_desktop_preview_fixture.rs`)
  against the snapshot in `examples/ato-desktop-preview.expected.gpui.txt`.

  Browser-only behavior the fixture relies on:
  - `<script>` content is raw-text-skipped (gpuiHTML never executes JS).
  - The `<style>` block's browser CSS resets (`*, *::before, *::after`,
    `html, body { ... }`, `::-webkit-scrollbar`) are silently skipped
    under lenient mode — none of them have gpui-side equivalents.
  - Generated output is a single dense expression on one line; rustfmt
    is the caller's responsibility (see `docs/spec.md`).

## Crates

- [`gpui-html-core`](crates/gpui-html-core) — parser + IR + codegen library.
- [`gpui-html`](crates/gpui-html) — `gpui-html <input.html>` CLI.

Prior art: wsafight's [`gpui-rsx`](https://crates.io/crates/gpui-rsx) is a
proc-macro that compiles JSX-like syntax inline in Rust files. gpui-html
targets the same lowering but for external `.gpui.html` design files
authored by humans or LLMs — a different ergonomic, not a competitor.

## Roadmap

- [ ] **0.1.0** — minimal vertical slice: parse `<div>` + class list, emit
      `div()....child(...)` for `examples/hello.html`. End-to-end CLI.
- [ ] **0.2.0** — full v0.1 spec coverage: every element + class token in
      [docs/spec.md](docs/spec.md), with snapshot tests against the
      generated Rust.
- [ ] **0.3.0** — `$<expr>` interpolation and event handler resolution.
- [ ] **0.4.0** — proc-macro frontend (write the same syntax inline in
      `.rs` files).
- [ ] **0.5.0** — components: `<MyButton .../>` resolves to user types via
      a component manifest.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
