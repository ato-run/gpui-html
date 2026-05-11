# gpuiHTML v0.1 — surface draft

**gpuiHTML は、HTML-like syntax と Tailwind-like class によって GPUI の
element tree / style tree / component tree を静的に生成する、GPUI 専用の
constrained markup language である。**

実体は HTML 互換レンダラーではなく、GPUI の `Styled` trait と `Style`
struct に落とせる範囲だけを文法化した、Tailwind 風 UI 記述言語である。
([trait.Styled][styled], [struct.Style][style])

このドキュメントは v0.1 の **surface draft** である。tag / class / event
の表は確定に近いが、theme manifest 形式・component prop 型・slot
semantics・`<span>` の inline-display 実装などは v0.2 で確定させる
（[付録 B](#付録-b-未解決事項) 参照）。

## 例

```html
<div class="flex flex-col gap-3 p-4 rounded-xl bg-surface border border-border">
  <h2 class="text-lg font-semibold text-primary">Execution Plan</h2>

  <p class="text-sm text-muted">
    This capsule requests permission to execute commands.
  </p>

  <button id="approve" class="h-9 px-4 rounded-md bg-accent text-accent-foreground">
    Approve
  </button>
</div>
```

これはブラウザ DOM ではなく、ビルド時または実行時に次のような GPUI
コードへ変換される。

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
    .child(/* h2 */)
    .child(/* p  */)
    .child(/* button */)
```

GPUI の公式例でも、`div().flex().flex_col().gap_3().bg(...).justify_center()
.items_center().shadow_lg().border_1().text_xl()` のような Tailwind-ish な
builder chain が使われている。([gpui.rs][gpuirs])

## 目的

1. GPUI を直接書くよりデザイン管理しやすくする。
2. HTML/Tailwind に慣れた人が Ato Desktop の UI を編集できるようにする。
3. Electron/DOM/CSSOM/JS runtime を持ち込まず、最終的には GPUI native
   element にコンパイルする。

つまり、これは **HTML で書ける GPUI** であって、**GPUI で動くブラウザ**
ではない。

## 非目的

```text
DOM API
JavaScript runtime
CSS cascade の完全再現
CSS selector engine
media query
pseudo selector
web component
iframe
canvas
arbitrary SVG
browser-compatible form behavior
HTML parsing error recovery
```

ここをサポートし始めると、GPUI の軽さを捨てて小型ブラウザエンジンを
作ることになる。

## 基本ルール

```text
すべての class は GPUI の Styled trait method もしくは
Style field 書き換えに静的変換できなければならない。
変換できない class は compile error。
```

通常の Tailwind とは違い、任意の CSS は許可しない。許可するのは
**gpui-tailwind subset**（このドキュメントの class 表に列挙されたもの）
だけ。

## 対応タグ

### Document compatibility layer

gpuiHTML accepts a **full HTML document** as input — the same source you
would paste into a browser. The compiler is not a browser engine, so most
of HTML is parsed-and-discarded, but the boilerplate doesn't have to be
hand-stripped before invoking the compiler.

```text
<!DOCTYPE html>          → parsed, skipped (case-insensitive keyword)

<html>, <head>, <body>   → transparent wrappers; their children flatten
                           into the parent's node list, so wrappers don't
                           reach codegen. Wrapper attrs are consumed and
                           discarded. Tag-name match is case-insensitive.

<meta>, <link>           → void metadata; parsed (loose attr handling)
                           and dropped. No closing tag required.
<title>...</title>       → raw-text metadata; body consumed and dropped.

<script>...</script>     → raw-text skip. Body is consumed verbatim
                           (so `<` inside JS doesn't try to start tags)
                           and dropped. gpuiHTML never executes JS.

<style>...</style>       → raw-text preserve. Body is consumed verbatim
                           and stored on a `Node::Style` AST node for
                           the static-CSS lowering pipeline. v0.1 does
                           not parse the CSS itself; that lands in a
                           follow-up.
```

The "compatibility layer" stays narrow on purpose: gpuiHTML still does
not implement DOM, JavaScript runtime, CSSOM, or browser layout. The
layer's job is just to let the same source serve as both a Tailwind
preview (in a browser) and a gpuiHTML input (to the compiler).

### UI tags

v0.1 lowers exactly these. Anything else (e.g. `<table>`, `<section>`,
`<button>`) is `UnknownTag` at parse time.

```html
<div>     → gpui::div()
<span>    → div() / inline-style text container
<p>       → div() + paragraph default text style
<h1>      → div() + heading-1 text style (text_2xl + font_weight bold)
<h2>      → div() + heading-2 text style (text_xl  + font_weight semibold)
<h3>      → div() + heading-3 text style (text_lg  + font_weight semibold)
<button>  → clickable element / Button component
<img>     → GPUI image element
<icon>    → registered icon component
<slot>    → Rust 側から差し込む child placeholder
```

Currently lowered: `<div>`, `<span>`, `<p>`, `<h1>`, `<h2>`, `<h3>`,
`<button>`. `<img>`, `<icon>`, `<slot>` from the table remain
deferred — they need asset / component-registry / runtime support
that's outside the static-lowering surface.

The semantic tags (`<p>`, `<h1..h3>`, `<button>`) all lower to
`gpui::div()` (gpui has no dedicated constructor for them). For the
headings the spec's explicit "text style" defaults are emitted
*before* the user's class chain, so authors can override via the
usual utility shorthand:

```html
<h1>Default heading</h1>
<!-- → div().text_2xl().font_weight(FontWeight::BOLD).child("Default heading") -->

<h1 class="text-sm font-normal">Overridden</h1>
<!-- → div().text_2xl().font_weight(FontWeight::BOLD).text_sm().font_weight(FontWeight::NORMAL).child("Overridden") -->
<!--   ^^ later builder calls override earlier ones (gpui builder semantics) -->
```

UI-tag matching is **case-sensitive** (unlike the document-compat tags
above): write `<div>`, not `<DIV>`.

v0.2 以降に延期するタグ。

```text
<input>    → TextInput component
             state / focus / selection / IME / on_input の整合が
             v0.1 のスコープに対して重く、最小縦スライスから外す。
<table>, <form>, <select>, <textarea>, <video>, <audio>,
<iframe>, <canvas>, <svg>
           → 範囲外（v0.1 でも v0.2 でも非対応の方向）。
```

`<span>` は GPUI に native な inline element が無いため、内部的には
`div()` に降りる。`display: inline-flex` 相当の `Style.display` 書き換え
shim をコード生成側で挿入する。

## class 対応範囲

class 名は Tailwind の表記を使うが、意味は GPUI への変換規則で決まる。
以下の表に無い class は **compile error**。

数値スケールは `rems(n × 0.25)`（Tailwind と同じ rem スケール、デフォルト
1rem = 16px）。`gpui::Styled` のスケールは `0..12, 16, 20, 24, 32, 40, 48,
56, 64, 72, 80, 96, 112, 128` および `0p5, 1p5, 2p5, 3p5` の半段（**13, 14,
15 は存在しない**）。Tailwind と完全一致しないので、未定義の整数値は
コンパイルエラーにする。([trait.Styled][styled])

### Layout

許可。

```text
flex            → .flex()
grid            → .grid()
hidden          → .hidden()
invisible       → .invisible()
flex-row        → .flex_row()
flex-col        → .flex_col()
flex-wrap       → .flex_wrap()
flex-nowrap     → .flex_nowrap()
flex-1          → .flex_1()
flex-auto       → .flex_auto()
flex-none       → .flex_none()
grow            → .flex_grow()
shrink          → .flex_shrink()
shrink-0        → .flex_shrink_0()

items-start     → .items_start()
items-center    → .items_center()
items-end       → .items_end()
items-baseline  → .items_baseline()

justify-start   → .justify_start()
justify-center  → .justify_center()
justify-end     → .justify_end()
justify-between → .justify_between()
justify-around  → .justify_around()
justify-evenly  → .justify_evenly()

content-start   → .content_start()
content-center  → .content_center()
content-between → .content_between()
```

禁止。

```text
inline-flex     → 対応する Styled shorthand が無い。
                  必要になったら Style.display 直書きで実装する。
items-stretch   → 同上。Style.align_items を直書きする shim が必要。
```

### Size

許可。

```text
w-<n>           → .w_<n>()
h-<n>           → .h_<n>()
size-<n>        → .size_<n>()
min-w-<n>       → .min_w_<n>()
min-h-<n>       → .min_h_<n>()
max-w-<n>       → .max_w_<n>()
max-h-<n>       → .max_h_<n>()
w-full / h-full / size-full   → .w_full() / .h_full() / .size_full()
w-auto / h-auto               → .w_auto() / .h_auto()
w-screen / h-screen / size-screen
                              → .w_screen() / .h_screen() / .size_screen()
w-1/2, w-1/3, w-2/3, w-3/4    → .w_1_2() / .w_1_3() / .w_2_3() / .w_3_4()
```

`<n>` は spacing scale と同じ。`13`, `14`, `15` は禁止。

#### App-shell compatibility tokens (#19)

Tailwind config-extended scales are not generally supported in v0.1
(custom-token manifest is v0.2 territory). One exemption ships in v0.1
to keep the Ato Desktop preview fixture compileable end-to-end:

```text
max-w-128       → .max_w(rems(32.0))     (v0.1 compat)
```

This single token mirrors `maxWidth: { '128': '32rem' }` in the
preview's Tailwind config. Anything else under `max-w-/max-h-/min-w-/
min-h-` that doesn't match the v0.1 spacing scale (e.g. `max-w-200`,
`max-w-card`) reports `UnsupportedClass` with a hint pointing at the
v0.2 manifest direction.

The v0.2 manifest will replace this single-token exemption with a
proper host-side declaration: the host app declares custom sizing
tokens in a manifest file, and the compiler reads it. Until then,
`max-w-128` is the only token we admit.

### Spacing

許可（全方向 + 軸別 + 個別側）。

```text
p-<n>           → .p_<n>()
px-<n> / py-<n> → .px_<n>() / .py_<n>()
pt-<n> / pr-<n> / pb-<n> / pl-<n>

m-<n>           → .m_<n>()
mx-<n> / my-<n> / mt-<n> / mr-<n> / mb-<n> / ml-<n>

gap-<n>         → .gap_<n>()
gap-x-<n> / gap-y-<n>
```

negative margin（`-m-2` 等）は v0.1 では禁止。理由は layout の
読みやすさと Ato Desktop での運用上、必要性が低いため（API 上は
`m_neg_2()` 等が存在するので将来開放可能）。

### Position

限定対応。

```text
relative   → .relative()
absolute   → .absolute()
top-<n> / right-<n> / bottom-<n> / left-<n>
inset-<n>
```

禁止。

```text
fixed   → gpui::Position に該当値が無い。
sticky  → 同上。
z-*     → Style に z-index 相当のフィールドが無い。
```

### Border / Radius / Shadow

許可。

```text
border          → .border_1()             ← bare 'border' は 1px に解決
border-<n>      → .border_<n>()           ← n ∈ {0..12, 16, 20, 24, 32}
border-t/r/b/l-<n> → .border_t_<n>() etc.
border-dashed   → .border_dashed()
border-<token>  → .border_color(theme.<token>)

rounded         → .rounded_md()           ← bare 'rounded' は md に解決
rounded-none / sm / md / lg / xl / 2xl / 3xl / full
                → .rounded_<suffix>()
rounded-t / b / l / r-<suffix>
rounded-tl / tr / bl / br-<suffix>

shadow          → .shadow_md()            ← bare 'shadow' は md に解決
shadow-sm / md / lg / xl / 2xl / none
                → .shadow_<suffix>()
```

### Color

色は任意 Tailwind palette ではなく **design token only**。

```text
bg-<token>      → .bg(theme.<token>)
text-<token>    → .text_color(theme.<token>)
border-<token>  → .border_color(theme.<token>)
bg-transparent  → .bg(gpui::transparent_black())
```

`<token>` は **hyphen-separated multi-segment ident** を許可する
(issue #7)。各 segment は `[A-Za-z_][A-Za-z0-9_]*` の形でなければなら
ず、hyphen は lowering 時に underscore に正規化される:

```text
bg-accent             → .bg(theme.accent)
bg-accent-foreground  → .bg(theme.accent_foreground)
text-muted-foreground → .text_color(theme.muted_foreground)
border-primary-hover  → .border_color(theme.primary_hover)
```

直接色指定（`bg-red-500`, `text-[#ff0000]`, `border-blue-300`）は
**すべて禁止**。判別ルールは「最終 segment が purely numeric かどうか」:

```text
bg-red-500            → palette utility (numeric last segment) → reject
bg-accent-foreground  → theme token (no numeric segment) → accept
```

palette と theme token の disambiguation は CSS 側 `var(--theme-X)`
(issue #27) と utility class 側 `bg-X` (issue #7) で同じ規則を使う。
共通 helper `class_map::normalize_theme_token` が両者を駆動するため、
`var(--theme-accent-foreground)` と `bg-accent-foreground` は同じ Rust
ident `theme.accent_foreground` を生成する。

理由: Ato Desktop の theme、dark mode、accessibility を壊さないため。

`Styled::bg`, `text_color`, `border_color` は `impl Into<Hsla>` を取る。
theme tokens は `Hsla` 定数として解決される。([trait.Styled][styled])

### Typography

許可。

```text
text-xs / sm / base / lg / xl / 2xl / 3xl
                → .text_xs() ... .text_3xl()

font-thin       → .font_weight(FontWeight::THIN)
font-light      → .font_weight(FontWeight::LIGHT)
font-normal     → .font_weight(FontWeight::NORMAL)
font-medium     → .font_weight(FontWeight::MEDIUM)
font-semibold   → .font_weight(FontWeight::SEMIBOLD)
font-bold       → .font_weight(FontWeight::BOLD)
font-extrabold  → .font_weight(FontWeight::EXTRA_BOLD)
font-black      → .font_weight(FontWeight::BLACK)

italic          → .italic()
not-italic      → .not_italic()
line-through    → .line_through()

leading-none    → .line_height(rems(1.0))
leading-tight   → .line_height(rems(1.25))
leading-snug    → .line_height(rems(1.375))
leading-normal  → .line_height(rems(1.5))
leading-relaxed → .line_height(rems(1.625))
leading-loose   → .line_height(rems(2.0))

line-clamp-<n>  → .line_clamp(<n>)
truncate        → .truncate()
```

`font-*` と `leading-*` は `Styled` 上の shorthand method が **存在しない**
ので、codegen 側で `font_weight(FontWeight::*)` と `line_height(rems(...))`
へ展開する。これは v0.1 spec の暗黙のコード生成ルールである。

#### `font-sans` no-op (#19)

Font-family is a host-app responsibility, not a per-element style:
gpui apps configure their font stack on the `Theme` or app shell once,
not on every `<div>`. v0.1 therefore does **not** lower font-family
utilities to GPUI builder calls.

For Tailwind preview compatibility, `font-sans` is accepted as a
**recognized no-op**: the class doesn't error (so the Ato Desktop
preview fixture compiles end-to-end), but no MethodCall is emitted.
Source order of other classes is preserved; the no-op simply
disappears from the lowered output.

```text
font-sans       → (no-op; recognized for Tailwind compat, no GPUI call)
font-mono       → UnsupportedClass + hint
font-serif      → UnsupportedClass + hint
```

`font-mono` and `font-serif` reject because in practice each gpui
app's `Theme` declares one font-family at a time; admitting them as
no-ops too would create the false impression that switching is
per-element.

禁止。

```text
whitespace-nowrap / whitespace-normal
                → Styled に shorthand が無く、Style.text への直書きが必要。
                  単行 truncation が目的なら truncate を使う。
text-ellipsis   → 単独の method が無い。truncate に統合される。
float / multi-column / arbitrary line clamp without n
                → 範囲外。
```

### Overflow / Scroll

許可。

```text
overflow-hidden   → .overflow_hidden()
overflow-visible  → .overflow_visible()
overflow-scroll   → .overflow_scroll()
overflow-x-hidden → .overflow_x_hidden()
overflow-y-hidden → .overflow_y_hidden()
overflow-x-scroll → .overflow_x_scroll()
overflow-y-scroll → .overflow_y_scroll()
```

禁止。

```text
overflow-auto     → gpui の Overflow enum は {Visible, Hidden, Scroll} のみ。
                    Auto 値は存在しない。
overflow-x-auto   → 同上。
overflow-y-auto   → 同上。
```

スクロール領域を作りたい場合は `overflow-y-scroll` を使う。GPUI の
スクロール挙動は web と完全一致しない（mouse wheel handling 等）点に
注意。([struct.Style][style])

### Opacity / Cursor

```text
opacity-<n>     → .opacity(<n> / 100.0)    ← n ∈ 0..100, e.g. opacity-50 → .opacity(0.5)

cursor-default  → .cursor_default()
cursor-pointer  → .cursor_pointer()
cursor-text     → .cursor_text()
cursor-move     → .cursor_move()
cursor-grab / cursor-grabbing
cursor-not-allowed
cursor-col-resize / cursor-row-resize
cursor-ew-resize / cursor-ns-resize
cursor-nesw-resize / cursor-nwse-resize
cursor-crosshair / cursor-help / cursor-none
```

`Styled` の cursor 系は 24 種類あり、Tailwind cursor utility と概ね一対一
で対応する。([trait.Styled][styled])

### Interactivity (Events)

イベントは HTML/JS ではなく、Rust handler への参照にする。

```html
<button on:click="approveExecutionPlan">
  Approve
</button>
```

これは次に変換される。

```rust
.on_click(cx.listener(Self::approve_execution_plan))
```

許可するイベント。

```text
on:click       → on_click
on:input       → on_input
on:change      → on_change
on:focus       → on_focus
on:blur        → on_blur
on:keydown     → on_key_down
on:mouseenter  → on_mouse_enter
on:mouseleave  → on_mouse_leave
```

禁止。

```text
onclick="javascript:..."
<script>...
style="..."
```

ハンドラ名は camelCase で参照し、codegen は snake_case の Rust method
名にマッピングする。

## Component 呼び出し

大文字タグは Rust component に対応させる。

```html
<ExecutionGraphView graph="$graph" class="h-full w-full" />

<SecretInput
  name="PG_PASSWORD"
  value="$secrets.PG_PASSWORD"
  on:change="setSecret"
/>
```

```text
小文字タグ → gpuiHTML builtin primitive（上記タグ表）
大文字タグ → Rust component（呼び出し側で構築済みの型）
$<expr>    → Rust view state / props 参照（`$sessions`, `$secrets.PG_PASSWORD`）
```

component の prop は基本的に文字列値か `$<expr>` の二択。配列・オブジェクト
リテラルは v0.1 では未サポート。

## `<style>` インライン CSS のサブセット対応

v0.1 spec は full HTML document をそのまま受け取る (issue #26)。HTML
boilerplate は parse-and-skip し、`<body>` 内の UI subtree だけが
codegen 対象になる。`<head>` 等に置かれた `<style>...</style>` は
parse pipeline が AST 上で `Node::Style` として保持し、CSS pipeline
(issue #27) が以下のサブセットを class lowering に統合する。

外部 CSS ファイル (`<link rel="stylesheet">`) は v0.1 でも非対応。
fetch しないし、href も無視する。

### v0.1 で受け取る CSS subset

Selector:

```text
.foo                   → 対応 (class selector のみ)
.foo-bar / .foo_1      → 対応 (Tailwind 風 hyphenated / underscored)
```

非対応 (すべて `UnsupportedSelector` 構造化エラー):

```text
.foo:hover             → pseudo-class
.foo .bar              → descendant combinator
.foo > .bar            → child combinator
.foo, .bar             → selector list
.foo.bar               → compound selector
#root                  → id selector
div, *                 → type / universal selector
@media, @keyframes     → at-rule
```

Declaration (property → 既存 `MethodCall` lowering へ流す):

```text
display: flex / grid / block / none
flex-direction: row / column
flex-wrap: wrap / nowrap
align-items: center / start / end / baseline / flex-start / flex-end
justify-content: center / start / end / space-between / space-around / space-evenly
gap / padding[-side] / margin[-side]: <n>rem または <n>px
                       → spacing scale (#10 と同じ {0..=12, 16, 20, 24, 32})
width / height: 100% / auto
                       → .w_full() / .h_full() / .w_auto() / .h_auto()
color / background-color / border-color: var(--theme-<token>)
                       → .text_color(theme.<token>) など
font-weight: thin / light / normal / medium / semibold / bold / extrabold / black
            または 100 / 300 / 400 / 500 / 600 / 700 / 800 / 900
overflow / overflow-x / overflow-y: hidden / visible / scroll
cursor: pointer / default / text
opacity: 0..=1 (0.5 等)、または 0%..=100%
```

非対応 declaration は `UnsupportedDeclaration` 構造化エラー、対応
property の不正値 (e.g. `padding: 1.7rem`) は `UnsupportedValue` で
spec 上のスケールから外れていることを通知する。

### CSS theme token 参照

CSS 内では theme token を `var(--theme-<token>)` の形で参照する。
custom property 名は CSS 規格上 hyphen を含めるので、Tailwind utility
の `text-accent-foreground` (#7 で議論中) のような曖昧さが発生しない。
hyphen は Rust ident として lowering 時に underscore に正規化される。

```text
color: var(--theme-primary)              → .text_color(theme.primary)
color: var(--theme-accent-foreground)    → .text_color(theme.accent_foreground)
background-color: var(--theme-surface)   → .bg(theme.surface)
```

直接色指定 (`color: red`, `color: #fff`, `color: rgb(...)`) は v0.1 では
すべて `UnsupportedValue` として reject される。理由は Ato Desktop の
theme / dark mode / accessibility を壊さないため。

### 統合順序: utility class が CSS rule を上書きする

`class="foo bar"` で要素が両方の class を持ち、`<style>.foo { ... }`
で `.foo` のルールも定義されているとき、emitted method order は次のとおり:

1. **Phase 1** (CSS rule): class 出現順に、CSS rule の `MethodCall` 列を
   出力する。
2. **Phase 2** (utility class): 同じく class 出現順に、utility class
   lowering を出力する。Phase 1 で CSS rule にカバーされた class が
   utility table に存在しなくても `UnknownClass` にしない。

意味的には GPUI builder semantics により後勝ちになるため、**utility
class が CSS rule を override する**。`class="..."` は局所オーバーライド
として読みやすい、というユーザ視点での原則。

### 統合の Definition of Done

- `<style>...</style>` 内の class rule が、対応 element の `class=`
  属性と突き合わされて lowering される。
- 同じ class が複数 stylesheet または同一 stylesheet で複数回現れた
  場合は source order で MethodCall 列を結合する。
- 非対応 selector / declaration / value は span 付き構造化エラーとして
  通知される (silent drop しない)。
- 元 HTML 全体に対する absolute span として出力される (parser 内部の
  局所 offset は外に漏らさない)。

## 旧 "CSS ファイルは許可しない" セクション (補足)

通常の外部 CSS ファイルは使わない。spacing / radius / shadow scale は
spec で固定済み (前述の class 表) であり、定義ファイルを必要としない。

色だけは固定スケールに乗らないため、**theme token** という概念で扱う。

### v0.1 の theme token 責務

```text
- token 名は symbolic（任意の識別子文字列）。
- codegen は bg-<token> を `.bg(theme.<token>)` に展開するだけ。
- token 名の妥当性は v0.1 では検証しない。
- `theme` 値の実体は呼び出し側（Ato Desktop 等）が所有する Rust struct。
- gpuiHTML compiler は theme struct の定義も読まないし、生成もしない。
```

つまり v0.1 の compiler は `bg-surface` を見て、機械的に
`.bg(theme.surface)` を出力するだけ。`surface` というフィールドが
実在するかは Rust compile 時に判定される（存在しなければ rustc が
通常のエラーを出す）。

直接色指定（`bg-red-500`, `text-[#ff0000]`, `border-blue-300`）は
**すべて禁止**。理由は Ato Desktop の theme、dark mode、accessibility
を壊さないため。

### Theme manifest (host-side validation)

オプションで TOML 形式の host theme manifest を読み込み、compiler
側で token 名を validate できる。CLI:

```sh
gpui-html compile <input> --manifest <path>
gpui-html check   <input> --manifest <path>
```

Library:

```rust
gpui_html_core::compile_with_manifest(src, Some(&manifest))
```

Manifest schema (TOML):

```toml
[colors]
base    = "#09090b"
surface = "#09090b"
accent  = "#6366f1"
"accent-foreground" = "#ffffff"

[max-width]
"128" = "32rem"

[max-height]
# (オプション)

[min-width]
# (オプション)

[min-height]
# (オプション)
```

挙動:

- `[colors]` の key 集合が `bg-X` / `text-X` / `border-X` および CSS
  `var(--theme-X)` の validate に使われる。manifest にない token は
  `UnknownThemeToken` で span 付き reject。
- `[max-width]` / `[max-height]` / `[min-width]` / `[min-height]` の
  entry は対応する custom-scale utility (`max-w-128` 等) を
  `.max_w(rems(N))` 形式に解決する。値は `<n>rem` または `<n>px` の
  みを受け取る (`16px = 1rem`)。それ以外 (`%`, `vh`, `calc(...)`) は
  manifest load 時の hard fail。
- `[colors]` の VALUE (hex / rgba 文字列) は v0.1 では検証しないし、
  Rust 出力にも埋め込まない (host の `Theme` struct がそれぞれの
  Rust 定義を保持する前提)。これは #23 (theme-token alpha) が
  packed-RGBA lowering に拡張する余地のために残してある。
- 不明な section (`[shadow]`, `[font]` など) は forward-compat の
  ために silently 無視される。

manifest 無しの場合 (back-compat):

- theme token は symbolic に pass-through (`bg-foo` → `theme.foo`)。
  実体の存在は rustc が後段で検証する。
- `max-w-128` のみ単独 app-shell compatibility exemption として解決
  される。それ以外の custom-scale token は `UnknownClass` + v0.2-
  manifest hint。

color value を用いた lowering (例: `bg-accent/10` → packed RGBA) は
v0.1 でも未実装で、#23 に残っている。manifest が色値を保持する
インフラ自体は既にあるので、#23 を完了するための spec 拡張は最小
で済む。

### v0.2 で予定する拡張

```text
- dark / light variant の切替を spec で記述する。
- spacing / radius / shadow を含む token 化を検討する。
- color VALUE を読み込んで packed-RGBA / alpha lowering を可能にする
  (#23)。
- font-family / font stack manifest entry の検討。
```

## Diagnostics

gpuiHTML は LLM・デザイナー・人間が並行して書く前提なので、validation
error は単なる「コンパイルできない」では弱い。compiler は **常に source
span を伴う構造化エラー**を返さなければならない。これは v0.1 spec の
要件である。

### 必須エラーカテゴリ

```text
ParseError              — 構文エラー（タグ閉じ忘れ等、UnbalancedTag を含む）
UnknownTag              — 対応タグ表に無い小文字タグ
UnknownComponent        — 大文字タグだが Rust 側の component manifest にない
UnknownClass            — class 表に無い utility class
UnsupportedAttribute    — 構文上は valid だが v0.1 で対応しない属性
UnknownThemeToken       — bg-<token> 等で <token> がスキーマと不整合（v0.2+）
InvalidEventHandler     — on:* の handler 名が Rust identifier として不正
InvalidInterpolation    — `$<expr>` の expr が解析不能
```

### 各 diagnostic に必須のフィールド

```text
- ファイルパス
- 行 / 列（または byte span）
- 該当 token / 該当 class / 該当 tag の literal 文字列
- 短い correction hint（提示できるとき）
```

correction hint の例:

```text
unknown class `overflow-auto`
  -- in design/session-pane.gpui.html:14:23
  hint: gpui has no `Overflow::Auto`. Use `overflow-y-scroll` instead.

unknown tag `<input>`
  -- in design/secrets-modal.gpui.html:8:3
  hint: <input> is deferred to v0.2. For now, render via a Rust
        component (e.g. <SecretInput .../>).
```

### 出力形式

CLI は人間向け（color + caret）と機械向け（JSON one-error-per-line）の
両方を出す。LLM がエラーを読んで自己修正できることを優先するため、JSON
は安定 schema として spec に含める（v0.1 で固定）。

```json
{
  "code": "UnknownClass",
  "file": "design/session-pane.gpui.html",
  "line": 14,
  "column": 23,
  "span": [422, 436],
  "literal": "overflow-auto",
  "hint": "gpui has no `Overflow::Auto`. Use `overflow-y-scroll` instead."
}
```

実装は Span-aware AST（[付録 B](#付録-b-未解決事項) の AST 拡張）を
前提とする。span のない AST では上記要件を満たせないため、AST 設計と
diagnostic 設計は同じマイルストーンで進める。

## 変換パイプライン

```text
.gpui.html
  ↓ parse           (gpuiHTML AST)
gpuiHTML AST
  ↓ class lower     (Style IR + theme token resolution)
GPUI Style IR
  ↓ component lower (capitalized tag → Rust path lookup)
Rust GPUI code
  ↓ rustc
native desktop UI
```

開発時は hot reload のために runtime parse してもよいが、正式ビルドでは
codegen する。

```text
design/session-pane.gpui.html
→ generated/session_pane.rs
```

prior art として、wsafight 氏の [`gpui-rsx`][gpuirsx-crate] crate が JSX-like
syntax から GPUI method chain を生成する Rust proc-macro として既に
公開されている（v0.3.2, 2026-02-22 時点）。gpuiHTML はこれと近いが、
Rust macro ではなく外部デザインファイルをコンパイルする設計に寄せる
点で異なる。

## Ato Desktop での適用範囲

gpuiHTML を使うべき場所。

```text
sidebar
session list
logs panel
execution graph viewer shell
consent modal
secrets modal
settings panel
error envelope viewer
stop/retry controls
```

使わない場所。

```text
capsule app の HTML/CSS/JS 本体
iframe / WebView 内のアプリ
外部 Web ページ
```

つまり、Ato Desktop の shell は gpuiHTML、capsule の中身は WebView の
まま分離する。

## 最小サンプル

```html
<div class="flex flex-col h-full bg-surface text-primary">
  <header class="flex items-center justify-between h-12 px-4 border-b border-border">
    <h1 class="text-sm font-semibold">Ato Desktop</h1>
    <button class="h-8 px-3 rounded-md bg-accent text-accent-foreground"
            on:click="newSession">
      New Session
    </button>
  </header>

  <main class="grid grid-cols-12 flex-1 overflow-hidden">
    <aside class="col-span-3 border-r border-border overflow-y-scroll">
      <SessionList sessions="$sessions" />
    </aside>

    <section class="col-span-9 flex flex-col overflow-hidden">
      <LogViewer logs="$activeSession.logs"
                 class="flex-1 overflow-y-scroll p-4" />
    </section>
  </main>
</div>
```

`overflow-y-auto` ではなく `overflow-y-scroll` を使っている点に注意
（前述の通り gpui に `Auto` overflow は存在しない）。

## 仕様の一文定義

> **gpuiHTML は、HTML-like syntax と Tailwind-like class によって GPUI の
> element tree / style tree / component tree を静的に生成する、GPUI 専用の
> constrained markup language である。**

## 付録 A: 初期ドラフトからの修正点

GPUI の `Styled` trait と `Style` struct を docs.rs で確認した上で、
v0.1 への確定にあたり次の修正を入れた。

1. **`overflow-auto` 系を削除。** GPUI の `Overflow` enum は
   `{Visible, Hidden, Scroll}` のみで `Auto` を持たないため、`overflow-auto`,
   `overflow-x-auto`, `overflow-y-auto` は spec から落とした。意図が
   「スクロール領域」なら `overflow-y-scroll` を使う。
2. **`inline-flex`, `items-stretch` を禁止に移動。** いずれも `Styled`
   に shorthand method が無く、`Style` field 直書きを必要とするため、
   v0.1 では除外。
3. **`font-*`（weight）と `leading-*` の codegen を明示。** これらは
   `Styled` に直接の method が存在せず、`font_weight(FontWeight::*)` /
   `line_height(rems(...))` への展開が必要。spec に展開規則を記載した。
4. **`whitespace-nowrap` / `text-ellipsis` を禁止に移動。** 単独 shorthand
   が無いため。単行 truncation が目的なら `truncate` を使う。
5. **bare `rounded` / `border` / `shadow` のデフォルト解決を明記。**
   これらは Tailwind では値付きだが GPUI では `rounded()` / `shadow()`
   bare method が無いので、それぞれ `rounded_md()` / `border_1()` /
   `shadow_md()` に解決する規則を追加。
6. **`flex-1`, `flex-auto`, `flex-none`, `grow`, `shrink`, `shrink-0`,
   `justify-evenly`, `items-baseline`, `border-dashed`, `cursor-*`,
   `opacity-*`, `italic`, `line-through`, `line-clamp-<n>`, `invisible`
   を追加。** いずれも `Styled` に対応 method があり、shell UI で必要に
   なる頻度が高い。
7. **spacing scale の有効値を明示。** `0..12, 16, 20, 24, 32, 40, 48, 56,
   64, 72, 80, 96, 112, 128` および `0p5, 1p5, 2p5, 3p5` のみ。`13, 14,
   15` は存在しない（GPUI 側の scale が sparse なため）。
8. **prior art への参照を更新。** `gpui-rsx` crate (wsafight) は実際に
   crates.io 公開済み（v0.3.2）。
9. **`<input>` を v0.2 送りに。** state / focus / selection / IME /
   on_input の整合が必要で、最小縦スライスから外した方が安全。v0.1 の
   対応タグ表からは外し、「v0.2 以降に延期するタグ」へ移動。
10. **theme token の責務を v0.1 で狭く定義。** v0.1 の compiler は
    `bg-<token>` を機械的に `.bg(theme.<token>)` へ展開するだけにし、
    token 名の妥当性検証や manifest 読み込みは v0.2 以降に延期。
    `theme` 値の実体は呼び出し側が所有する Rust struct とする。
11. **diagnostics 要件を spec に追加。** unknown class / tag / attr /
    theme token / event handler は span 付き構造化エラーで返す。LLM
    self-correction を product feature として最初から扱う。
12. **crate 命名衝突を解消。** workspace の `gpui-rsx` を
    `gpui-html-core` に rename。これで crates.io 公開時の衝突は無し。

## 付録 B: 未解決事項

- **theme manifest 形式。** v0.2 で TOML / JSON / Rust const のどれを
  source of truth にするか、dark/light variant の切替をどう書くかを
  決める。v0.1 では token 名は symbolic（spec で検証しない）。
- **component prop の型解決。** 大文字タグの prop が Rust 側で `&str`
  なのか `Hsla` なのか `Vec<T>` なのかは現状 codegen 時点で型を知る
  手段がない。v0.2 で component manifest 形式（外部ファイルで prop 型
  を宣言）を導入する想定。
- **slot semantics.** `<slot name="...">` の name 解決規則は v0.2。
- **`<span>` の inline-display 実装。** Style.display を直書きする shim
  をどこに置くか、`<span>` をどこまで `<div>` と区別するかは v0.2 で
  確定。
- **AST 拡張 (Span-aware).** [Diagnostics](#diagnostics) を満たすには
  AST に `Span { start, end }` と `ClassToken { raw, span }` を持たせる
  必要がある。実装マイルストーンとして diagnostic 要件と同時に進める。
- **`<input>` の v0.2 設計.** state / focus / IME / on_input をどう
  シリアライズするか、TextInput component への lowering をどう定義
  するかは v0.2 で確定。

[styled]: https://docs.rs/gpui/latest/gpui/trait.Styled.html
[style]: https://docs.rs/gpui/latest/gpui/struct.Style.html
[gpuirs]: https://www.gpui.rs/
[gpuirsx-crate]: https://crates.io/crates/gpui-rsx
