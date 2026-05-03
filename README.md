# Scientific Calculator MCP App

An educational MCP App that teaches the **three paths** of an MCP App widget by
acting as a small interactive calculator.

```
┌─────────────────────────────────────────────────────────────┐
│                 MCP Host (Claude Desktop, ChatGPT)          │
│   ┌─────────────────────────────────────────────────────┐   │
│   │              Calculator Widget (HTML)               │   │
│   │   ┌──────────────────────────┐                      │   │
│   │   │ 1 + 1                    │   <- local UI path   │   │
│   │   │ ──────────────── 2       │   <- server result   │   │
│   │   └──────────────────────────┘                      │   │
│   │   [7][8][9][÷]  [+/-][C][←]                         │   │
│   │   [4][5][6][×]                                      │   │
│   │   [1][2][3][−]                                      │   │
│   │   [0][.][   =   ][+]                                │   │
│   └─────────────────────────────────────────────────────┘   │
│                          ▲ MCP Bridge                       │
│                          ▼                                  │
│   ┌─────────────────────────────────────────────────────┐   │
│   │        Calculator MCP Server (Rust, PMCP)           │   │
│   │   • add(a, b)        • subtract(a, b)               │   │
│   │   • multiply(a, b)   • divide(a, b)                 │   │
│   │   • negate(x)                                       │   │
│   └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## V1: Three paths

| Path | Trigger | What happens |
|---|---|---|
| **Local UI** | Clicking digits/operators in the keypad | The widget updates its visible expression locally. No tool calls. The badge shows `local`. |
| **Server compute** | Clicking `=` (or `+/-`) on a simple `a OP b` expression | The widget calls a primitive MCP tool (`add`, `subtract`, etc.) via `app.callServerTool` (MCP Apps SDK) and renders the structured result. The badge shows `server`. |
| **LLM reasoning** | Typing math in the chat (e.g. _"compute 1 + 1"_) | The host LLM decomposes the request, calls primitive tools, and the host pushes each `structuredContent` to the widget via `ui/notifications/tool-result`. The badge shows `llm`. |

V1 is intentionally minimal — it shows the three paths with five primitive
tools and one widget.

## V2: LLM decomposition & operator precedence

V2 (this branch) demonstrates how an MCP App handles non-trivial
expressions like `(3 + 5)^2 / log10(1000)`. The host LLM decomposes the
expression into **ordered primitive tool calls**; the server stays a flat
collection of primitives (no `evaluate_expression` parser). The widget
gains a **step list** that visualizes the LLM's decomposition in real
time, so the teaching point — *the LLM owns precedence, the server owns
arithmetic* — is visible.

Three scientific primitives are added to give the LLM enough vocabulary:

- `power(base, exponent)`
- `sqrt(x)` — `domain_error` for `x < 0`
- `log(x, base)` — explicit base; `domain_error` for `x <= 0`, `base <= 0`, `base == 1`

### V2 widget keys: `√x`, `x²`, `xʸ`, `log₁₀`, `ln`

The keypad now exposes a row of scientific function keys above the
arithmetic grid so a single number on the display can be routed directly
to one of the V2 primitives via the **server** path
(`app.callServerTool`). Each key encodes the (x, base) pair in its label
so the user can see exactly what the server is asked to compute:

| Key | Tool call | Notes |
|---|---|---|
| `√x` | `sqrt(x)` | unary; `domain_error` if `x < 0` |
| `x²` | `power(x, 2)` | shortcut for the common case |
| `xʸ` | `power(a, b)` | new infix `^` operator; `=` resolves it |
| `log₁₀` | `log(x, 10)` | base 10; explicit in the label |
| `ln` | `log(x, e)` | base e; explicit in the label |

Arbitrary-base logs and full expressions (like `(3 + 5)^2 / log10(1000)`)
still route through the **LLM** path: ask the chat, the host LLM
decomposes the expression into ordered primitive tool calls, and each
result is pushed to the widget's step list. The widget keys handle the
single-step scientific cases; chat handles the multi-step reasoning.
That split is the educational point.

The canonical chat demo: typing _"compute (3 + 5)^2 / log10(1000)"_ in
chat makes the LLM emit, in order:

1. `add(3, 5)` → `8`
2. `power(8, 2)` → `64`
3. `log(1000, 10)` → `3`
4. `divide(64, 3)` → `21.333…`

Each call's `structuredContent` is pushed to the widget via
`ui/notifications/tool-result` and appended to the step list.

See [`examples/v2-llm-decomposition.md`](examples/v2-llm-decomposition.md)
for the full walk-through. `preview.html` includes a **Run decomposition**
button that simulates the host pushing this exact sequence.

## V3: Natural-language math & interpretation visibility

V3 takes the same separation one layer up — the user types a *word
problem* in chat, and the LLM has to recognize the math before it can
call any tools. The widget gains an **interpretation panel** that
visualizes the full teaching loop:

```
user phrasing  →  interpreted math  →  executed tool calls  →  final answer
```

The canonical V3 demo:

> *What is the hypotenuse of a right triangle with sides 5 and 12?*

The host LLM emits an interpretation envelope (`Pythagorean theorem:
c = √(a² + b²)`, expression `√(5² + 12²)`), then the four ordered
primitive calls — `power(5, 2)`, `power(12, 2)`, `add(25, 144)`,
`sqrt(169)` — and finally a completion envelope with
`answer: "13"`. The widget shows the interpretation rows, the step
list grows in real time, and the answer row settles to `13`.

V3 also adds **one new primitive**: `get_constant(name)` for `pi` /
`e`. This exists so the LLM can look up π while decomposing a circle-
area question (`area of a circle with radius 3` → `get_constant("pi")`,
`power(3, 2)`, `multiply(π, 9)`) without the server needing to know
what "circle" means.

| Tool | Signature | Notes |
|---|---|---|
| `get_constant(name)` | `name: "pi"\|"e"` | New in V3. Returns the same `CalcOutput` shape as the arithmetic tools. Unknown names return `{ ok: false, code: 'unknown_constant', ... }`. |

The widget listens for a new client-side notification,
`ui/notifications/interpretation`, with a four-field envelope:

```jsonc
{
  "phrasing": "What is the hypotenuse of a right triangle with sides 5 and 12?",
  "concept":  "Pythagorean theorem: c = √(a² + b²)",
  "expression": "√(5² + 12²)"
}
```

The host typically sends two of these per word problem — one before
the tool calls and one after with the final `answer` — and each
envelope is shallow-merged into widget state.

The teaching point is unchanged: the **server stays a flat collection
of primitives**. It does not parse phrasing. Interpretation lives in
the LLM and is *displayed* by the widget. See
[`examples/v3-natural-language-math.md`](examples/v3-natural-language-math.md)
for the full walk-through. `preview.html` includes
**Hypotenuse**, **Circle area**, and **20% discount** buttons that
simulate the host pushing the interpretation + tool-result sequence
end-to-end.

### V1 server tools

Every tool returns the same discriminated-union shape:

```jsonc
// success
{ "ok": "true",  "op": "add", "inputs": [1, 1], "result": 2, "display": "2" }

// error
{ "ok": "false", "op": "divide", "inputs": [1, 0],
  "code": "divide_by_zero", "message": "Cannot divide by zero." }
```

Errors are structured (`divide_by_zero`, `invalid_input`) so the widget and
the LLM can reason about them without parsing free-form strings.

### What V1 deliberately does not have

- No `evaluate_expression` parser on the server.
- No calculator history (the chat transcript is the history).
- No plotting or code mode (V4+). Scientific primitives arrive in V2; natural-language interpretation arrives in V3 — V1 has only the five arithmetic tools above.
- No widget → LLM "send this prompt" routing — the MCP Apps SDK exposes
  `app.callServerTool` and `app.ontoolresult`, but does not expose a
  "compose a chat message on my behalf" API. See
  [SDK limitations](#sdk-limitations) below.

## Run

```bash
cargo build --release
./target/release/scientific-calculator-mcp-app
# Serves on http://0.0.0.0:3000 (override with PORT=8080)
```

Connect with Claude Code:

```bash
claude mcp add calculator --transport http http://localhost:3000
```

Or test the server directly:

```bash
curl -s -X POST http://localhost:3000 \
  -H 'Content-Type: application/json' -H 'Accept: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"add","arguments":{"a":1,"b":1}}}'
```

## Develop the widget without a host

```bash
# Open preview.html in a browser.
xdg-open preview.html  # or: open preview.html
```

`preview.html` is a **minimal mock MCP Apps host**: it answers the
widget's `ui/initialize` handshake, replies to `tools/call` JSON-RPC
requests (server path), and pushes `ui/notifications/tool-result` and
`ui/notifications/interpretation` notifications to the iframe (LLM
path). Everything flows over `postMessage` exactly as it would from
Claude Desktop or ChatGPT Apps, so the preview exercises the same SDK
plumbing as production.

## Test

```bash
cargo test
```

Tests cover all primitives (V1 arithmetic + V2 scientific + V3
`get_constant`), divide-by-zero, domain errors (`sqrt(-1)`,
`log(0, 10)`, `power(-1, 0.5)`, …), NaN/Infinity handling, the
structured-output JSON shape, and decomposition walk-throughs for
`(3 + 5)^2 / log10(1000)` (V2) and the
hypotenuse/circle-area word problems (V3).

## MCP Apps SDK

The widget uses the official
[`@modelcontextprotocol/ext-apps`](https://www.npmjs.com/package/@modelcontextprotocol/ext-apps)
SDK (loaded as an ES module from
`https://esm.sh/@modelcontextprotocol/ext-apps@1.7.1`). There is no
bundler step — `widgets/keypad.html` is the deployable artifact, and
the server `include_str!`s it as-is. The SDK provides:

- `new App({ name, version })` — constructor
- `app.callServerTool({ name, arguments })` — widget → server tool call
- `app.ontoolresult` / `app.ontoolinput` / `app.ontoolcancelled` /
  `app.onteardown` / `app.onerror` — required handlers, registered
  before `app.connect()`
- `app.onhostcontextchanged` — react to host theme changes
- `app.connect()` — handshake; with the default `autoResize: true` this
  also installs a `ResizeObserver` so the widget no longer needs a
  manual `notifyIntrinsicHeight` call
- `app.getHostContext()` — read theme/locale once connect resolves

## SDK limitations

The educational point of V1 is that a widget click should be able to
"hand off" to the LLM for reasoning. The MCP Apps SDK exposes
`app.callServerTool` for widget → server tool calls and
`app.ontoolresult` for inbound results, but it does **not** expose a
`sendUserMessage()` or equivalent. So when the user clicks `=` on an
expression V1 can't evaluate (e.g. `1 + 2 * 3`, where precedence
matters), the widget does the closest supported thing: it shows a hint
pointing the user to ask the chat. When the user does, the LLM-driven
path lights up automatically.

V2 sidesteps this by leaning into the supported direction: the user types
the expression in the chat, the LLM decomposes it into ordered primitive
tool calls, and the host pushes each `structuredContent` back to the
widget. The widget renders the ordered list in its **LLM decomposition**
panel, so the educational point ("the LLM owns precedence, the server
owns arithmetic") is visible without inventing a new bridge API. A
proper widget → LLM "send this prompt" handoff still depends on the SDK
exposing such a surface, and is left for a future iteration.

V3 introduces a widget-side notification convention,
`ui/notifications/interpretation`, that carries the LLM's
phrasing/concept/expression/answer envelope into the widget alongside
the V2 tool-result stream. `@modelcontextprotocol/ext-apps@1.7.1` does
not expose a typed handler for arbitrary host-side notifications
(`app.on*` covers tool input/result/cancellation, teardown, host-
context changes, errors — and that's it), so the widget keeps a
narrowly scoped raw `window.addEventListener('message', ...)` listener
that ONLY accepts the `ui/notifications/interpretation` method.
Everything else (tool results, lifecycle, theme) flows through the
SDK. If a future SDK release adds a custom-notifications handler, the
fallback listener can be deleted and the four-field shallow-merge
contract above is what it should target.

State persistence (`mcpBridge.setState` / `getState`) was used in
earlier revisions of the widget; the SDK does not provide a direct
equivalent and the keypad now renders entirely from the live tool
result stream. The chat transcript is the history.

## File map

```
.
├── Cargo.toml
├── src/
│   ├── lib.rs            # PMCP server: V1 arithmetic + V2 scientific + V3 get_constant
│   └── main.rs           # Local HTTP binary
├── scientific-calculator-mcp-app-lambda/   # AWS Lambda wrapper
├── widgets/
│   └── keypad.html       # Keypad + V2 step list + V3 interpretation panel (uses @modelcontextprotocol/ext-apps SDK)
├── preview.html          # Mock MCP Apps host — V2 decomposition + V3 word-problem demos
├── examples/
│   ├── v1-basic-arithmetic.md
│   ├── v2-llm-decomposition.md
│   └── v3-natural-language-math.md
└── README.md
```
