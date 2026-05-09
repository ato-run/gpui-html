# gpui-html

Write [gpui](https://www.gpui.rs/) UIs as HTML.

`gpui` is a fast, expressive Rust UI framework, but its builder API
(`div().flex().flex_col().gap_2().child(...)`) is verbose to write by hand
and hard for non-Rust tooling (designers, LLMs, codegen pipelines) to
produce. **gpuiHTML** is a strict HTML subset that compiles down to that
builder code via a small intermediate representation we call **gpui-rsx**.

```
  gpuiHTML  ──parse──▶  gpui-rsx (IR)  ──codegen──▶  gpui Rust
   *.html                ast::Node tree                  *.rs
```

Status: **0.0.0 / scaffold.** The pipeline shape is fixed, the parser and
codegen are stubs. See [Roadmap](#roadmap).

## Why three layers, not one

Splitting parser ↔ IR ↔ codegen lets each replace independently:

- A future `gpui-rsx-macro` proc-macro frontend can reuse the IR + codegen
  and skip the HTML parser entirely (taking Rust tokens instead).
- A future LSP / formatter can reuse the parser without pulling in codegen.
- The codegen mapping table (HTML tag + class -> gpui method) is the
  single source of truth for "what gpuiHTML supports today" and is easy to
  diff against new gpui releases.

## Spec (v0 sketch)

The full spec lives in code — `crates/gpui-rsx/src/codegen.rs` is
authoritative. The shape:

**Elements** map to gpui constructors:

| HTML tag    | gpui call         |
|-------------|-------------------|
| `<div>`     | `div()`           |
| `<span>`    | `div()` (inline)  |
| `<button>`  | `div()` + click handler attrs |
| `<img>`     | `img()`           |

**Classes** are tailwind-style utility tokens, mapped one-to-one to gpui
builder methods:

| class token   | gpui call          |
|---------------|--------------------|
| `flex`        | `.flex()`          |
| `flex-col`    | `.flex_col()`      |
| `gap-2`       | `.gap_2()`         |
| `p-4`         | `.p_4()`           |
| `text-white`  | `.text_color(rgb(0xffffff))` |

**Children** become chained `.child(...)` calls. Bare text nodes become
`.child("...")`.

Anything outside the table is a hard error — gpuiHTML doesn't pass through
unknown HTML. The point of the spec is determinism: every input has exactly
one valid Rust output, or it fails to compile.

## Example

`examples/hello.html`:

```html
<div class="flex flex-col gap-2 p-4">
  <span>Hello, gpui!</span>
  <div class="text-white">Compiled from HTML.</div>
</div>
```

Target output:

```rust
div()
    .flex()
    .flex_col()
    .gap_2()
    .p_4()
    .child("Hello, gpui!")
    .child(
        div()
            .text_color(rgb(0xffffff))
            .child("Compiled from HTML."),
    )
```

## Crates

- [`gpui-rsx`](crates/gpui-rsx) — parser + IR + codegen library.
- [`gpui-html`](crates/gpui-html) — `gpui-html <input.html>` CLI.

## Roadmap

- [ ] **0.1.0** — minimal vertical slice: parse `<div>` + class list, emit
      `div()....child(...)` for `examples/hello.html`. End-to-end CLI.
- [ ] **0.2.0** — full v0 spec coverage: every element + class token in the
      tables above, with snapshot tests against generated Rust.
- [ ] **0.3.0** — interpolation: `{state.count}` and `{|cx| ...}` bridges
      into Rust expressions.
- [ ] **0.4.0** — `gpui-rsx-macro` proc-macro frontend (write rsx inline in
      `.rs` files instead of separate `.html`).
- [ ] **0.5.0** — components: `<MyButton .../>` resolves to user types.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
