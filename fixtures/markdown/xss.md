# XSS fixture

Every construct below must render as inert text or be removed. The sanitizer
test asserts on the output; do not "fix" the Markdown.

<script>alert('raw html script')</script>

<img src=x onerror="alert('onerror')">

<a href="javascript:alert('js')">javascript link in raw html</a>

[javascript link](javascript:alert('md'))
[vbscript link](vbscript:msgbox)
[data html link](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)
[file link](file:///etc/passwd)
[entity scheme](&#106;avascript:alert(1))
[percent scheme](%6Aavascript:alert(1))

![svg data image](data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIG9ubG9hZD0iYWxlcnQoMSkiLz4=)
![onerror in title](docs/pixel.png "x\" onerror=\"alert(1)")

<iframe src="https://example.com"></iframe>
<object data="x"></object>
<embed src="x">
<style>body { display: none }</style>
<svg onload="alert(1)"></svg>
<math><mi xlink:href="javascript:alert(1)">x</mi></math>
<form action="https://evil.example"><input type="text" name="q"><button>go</button></form>
<a id="root" href="#">clobber the app root</a>
<div style="position:fixed;inset:0;background:red">overlay</div>
<a href="https://example.com" target="_blank" rel="opener">target</a>
<input type="checkbox" onclick="alert(1)">

`<script>` in inline code and a fenced block:

```html
<script>alert('fenced')</script>
```

Benign content survives: **bold**, a [safe link](https://example.com), a
[local link](docs/target.ts), and a table:

| a | b |
|---|---|
| 1 | 2 |
