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
for the v0.1 spec (tag table, class table, theme tokens, codegen rules).

Status: **0.0.0 / scaffold.** Spec is locked at v0.1; parser and codegen
are stubs. See [Roadmap](#roadmap).

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

## Crates

- [`gpui-rsx`](crates/gpui-rsx) — parser + IR + codegen library.
  *(Note: name collides with [wsafight/gpui-rsx](https://crates.io/crates/gpui-rsx)
  on crates.io; will be renamed before any publish — see spec 付録 B.)*
- [`gpui-html`](crates/gpui-html) — `gpui-html <input.html>` CLI.

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
