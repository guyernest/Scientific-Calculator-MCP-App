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
| **Server compute** | Clicking `=` (or `+/-`) on a simple `a OP b` expression | The widget calls a primitive MCP tool (`add`, `subtract`, etc.) via `mcpBridge.callTool` and renders the structured result. The badge shows `server`. |
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

The canonical demo: typing _"compute (3 + 5)^2 / log10(1000)"_ in chat
makes the LLM emit, in order:

1. `add(3, 5)` → `8`
2. `power(8, 2)` → `64`
3. `log(1000, 10)` → `3`
4. `divide(64, 3)` → `21.333…`

Each call's `structuredContent` is pushed to the widget via
`ui/notifications/tool-result` and appended to the step list.

See [`examples/v2-llm-decomposition.md`](examples/v2-llm-decomposition.md)
for the full walk-through. `preview.html` includes a **Run decomposition**
button that simulates the host pushing this exact sequence.

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
- No plotting or code mode (V3+). Scientific primitives arrive in V2 — V1 has only the five arithmetic tools above.
- No widget → LLM "send this prompt" routing — the MCP Apps SDK exposes
  `mcpBridge.callTool` / `getState` / `setState` and pushes
  `ui/notifications/tool-result`, but does not expose a "compose a chat
  message on my behalf" API. See [SDK limitations](#sdk-limitations) below.

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

`preview.html` mocks `window.mcpBridge` and adds buttons that simulate the
host pushing `ui/notifications/tool-result` to the widget — so you can see
both the **server** path (clicking `=` in the keypad) and the **LLM** path
(clicking the simulator buttons in the side panel) without running an MCP
host.

## Test

```bash
cargo test
```

Tests cover all primitives (V1 arithmetic + V2 scientific), divide-by-zero,
domain errors (`sqrt(-1)`, `log(0, 10)`, `power(-1, 0.5)`, …),
NaN/Infinity handling, the structured-output JSON shape, and a
decomposition walk-through for `(3 + 5)^2 / log10(1000)`.

## SDK limitations

The educational point of V1 is that a widget click should be able to
"hand off" to the LLM for reasoning. The MCP Apps spec (SEP-1865) and the
PMCP `McpAppsAdapter` reference example expose the following bridge
surface:

- `mcpBridge.callTool(name, args)` — widget → server tool call
- `mcpBridge.getState()` / `setState(s)` — widget-local persistence
- Inbound `ui/notifications/tool-result` messages with `structuredContent`

There is no `mcpBridge.sendUserMessage()` or equivalent in the reference
SDK. So when the user clicks `=` on an expression V1 can't evaluate (e.g.
`1 + 2 * 3`, where precedence matters), the widget does the closest
supported thing: it shows a hint pointing the user to ask the chat. When
the user does, the LLM-driven path lights up automatically.

V2 sidesteps this by leaning into the supported direction: the user types
the expression in the chat, the LLM decomposes it into ordered primitive
tool calls, and the host pushes each `structuredContent` back to the
widget. The widget renders the ordered list in its **LLM decomposition**
panel, so the educational point ("the LLM owns precedence, the server
owns arithmetic") is visible without inventing a new bridge API. A
proper widget → LLM "send this prompt" handoff still depends on the SDK
exposing such a surface, and is left for a future iteration.

## File map

```
.
├── Cargo.toml
├── src/
│   ├── lib.rs            # PMCP server: primitive tools (V1) + scientific (V2)
│   └── main.rs           # Local HTTP binary
├── scientific-calculator-mcp-app-lambda/   # AWS Lambda wrapper
├── widgets/
│   └── keypad.html       # Interactive keypad widget + V2 step-list view
├── preview.html          # Mock-bridge harness with V2 decomposition demo
├── examples/
│   ├── v1-basic-arithmetic.md
│   └── v2-llm-decomposition.md
└── README.md
```
