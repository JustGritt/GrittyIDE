# GFM fixture

Deterministic Markdown covering what the preview must render. Do not edit
casually: rendering tests snapshot this file.

## Emphasis and inline code

Plain, *emphasis*, **strong**, ***both***, ~~strikethrough~~, `inline code`,
a\*literal asterisk\*, and an entity: &copy; &amp; &lt;tag&gt;.

Line one of a paragraph
continues on line two (soft break).

## Lists

- unordered one
- unordered two
  - nested
  - nested with `code`
- unordered three

1. ordered one
2. ordered two
   1. nested ordered
3. ordered three

5. starts at five
6. six

## Task lists

- [ ] open task
- [x] done task
- [X] done task, capital
- [ ]not a task (no space)
- regular item with [brackets] in text

## Table

| Name | Kind | Size |
|:-----|:----:|-----:|
| `a.ts` | file | 12 |
| **docs** | dir | 3 |
| c \| pipe | escaped | 0 |

## Code

```ts
export function hello(name: string): string {
  return `Hello, ${name}!`;
}
```

```rust
fn main() {
    println!("hi");
}
```

```json
{ "key": [1, 2, 3], "nested": { "ok": true } }
```

```
no language
```

```unknown-lang
still escaped <b>not bold</b>
```

    indented code block
    second line

## Quotes and rules

> A quote
> with two lines
>
> > nested quote

---

***

## Links and images

[Relative](docs/target.ts), [line](docs/target.ts:3), [line and column](docs/target.ts:3:5),
[anchor](#table), [external](https://example.com/path?q=1), <https://example.org>,
[reference link][ref], [mailto](mailto:someone@example.com).

Bare domain in prose: README.md and AGENTS.md must stay text; https://example.net/auto links.

![Relative image](docs/pixel.png "Local image")
![Data image](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==)

[ref]: https://example.com/reference "Reference title"

## Headings with punctuation & duplicates

### Duplicate

### Duplicate

### Ünïcödé — heading, with: punctuation!

Final paragraph with a hard break  
after two spaces, and superscript H<sup>2</sup>O stays escaped.
