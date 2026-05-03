# V2 Example — LLM Decomposition & Operator Precedence

V2 adds three scientific primitives (`power`, `sqrt`, `log`) and a widget
**step list** that visualizes the ordered tool calls an LLM makes when
decomposing a non-trivial expression.

> **Teaching point.** Operator precedence is *not* solved by a server-side
> expression parser. The MCP host LLM is responsible for reading the user's
> expression, choosing the correct decomposition into primitive tool calls,
> and ordering them. The server only ever computes one primitive at a time.
> The widget makes that ordering visible.

## V2 server tools

| Tool | Signature | Notes |
|---|---|---|
| `add(a, b)` | binary | V1 |
| `subtract(a, b)` | binary | V1 |
| `multiply(a, b)` | binary | V1 |
| `divide(a, b)` | binary | V1, structured `divide_by_zero` error |
| `negate(x)` | unary | V1 |
| `power(base, exponent)` | binary | New. `base^exponent`. Domain errors for non-finite results (e.g. `0^-1`, `(-1)^0.5`). |
| `sqrt(x)` | unary | New. Domain error for `x < 0`. |
| `log(x, base)` | binary | New. Explicit base. Domain error if `x <= 0`, `base <= 0`, or `base == 1`. Use `base = 10` for log10, `base = e` for ln. |

All tools return the same discriminated-union shape introduced in V1:

```jsonc
// success
{ "ok": "true",  "op": "log", "inputs": [1000, 10], "result": 3, "display": "3" }

// error
{ "ok": "false", "op": "sqrt", "inputs": [-1],
  "code": "domain_error", "message": "sqrt is undefined for negative numbers in the reals." }
```

## Canonical decomposition: `(3 + 5)^2 / log10(1000)`

The user types in chat:

> Compute (3 + 5)^2 / log10(1000)

The host LLM decomposes the request — applying parentheses, exponentiation
priority, and the log10 ↦ `log(_, 10)` rewrite — and emits **four ordered
tool calls**:

| # | Call | Result | Why it's this tool |
|---|---|---|---|
| 1 | `add(3, 5)` | `8` | innermost parentheses |
| 2 | `power(8, 2)` | `64` | exponent on the parenthesized result |
| 3 | `log(1000, 10)` | `3` | `log10(1000)` ⇒ `log(_, base=10)` |
| 4 | `divide(64, 3)` | `21.333…` | the top-level division |

For each call, the MCP host pushes the `structuredContent` to the widget
via `ui/notifications/tool-result`. The widget appends each one to its
**LLM decomposition** step list:

```
1.  add(3, 5)         → 8
2.  power(8, 2)       → 64
3.  log(1000, 10)     → 3
4.  divide(64, 3)     → 21.3333333333
```

The headline result line shows the *latest* step (`divide(64, 3) → 21.333…`,
badged `llm`), and the step list shows the full ordered decomposition. The
chat transcript still holds the natural-language history; the widget owns
the "what did the LLM ask the server to do, in order".

## Why no `evaluate_expression` tool?

We deliberately don't add a single server tool that takes a string like
`"(3 + 5)^2 / log10(1000)"` and parses it. That would short-circuit the
teaching point:

- LLMs already know operator precedence, parentheses, and standard
  notations like `^`, `log10`, `sqrt`, etc. Reasoning about decomposition
  is the LLM's strength.
- A server-side parser would push a real grammar (and edge-case handling
  for unicode minus signs, exponent notation, implicit multiplication,
  etc.) into the server. That's complexity that doesn't belong in a tool
  that's supposed to demonstrate primitive composition.
- The wire-shape we *want* the user to see is "the LLM made a sequence of
  small, reviewable tool calls", not "the LLM handed off a string". The
  step list literally renders that sequence.

V3 (deferred) explores `execute_code` / code-mode for cases where a
sequence of explicit tool calls is too coarse-grained — that needs more
design and is intentionally out of scope here.

## Domain errors are structured, not strings

Each new tool returns a structured error so the LLM can recover (e.g.
"the user wrote `sqrt(-1)`; explain it's not real, ask if they want a
complex result") and the widget can display the failure inline in the
step list:

```jsonc
{ "ok": "false", "op": "sqrt", "inputs": [-1],
  "code": "domain_error", "message": "sqrt is undefined for negative numbers in the reals." }
```

Codes used in V2:
- `invalid_input` — non-finite (NaN, ±∞) numeric input
- `domain_error` — input outside the function's real domain (e.g. `sqrt(-1)`,
  `log(0, _)`, `log(_, 1)`, `power(-1, 0.5)`)
- `divide_by_zero` — V1, kept for parity

## Try it locally

```bash
cargo test
cargo build --release
./target/release/scientific-calculator-mcp-app
```

Then in a host chat:

```
Compute (3 + 5)^2 / log10(1000) using the calculator tools.
```

Or open `preview.html` in a browser and click **Run decomposition** in the
side panel — it simulates the LLM pushing the four tool results in order
and you'll see the step list animate in.
