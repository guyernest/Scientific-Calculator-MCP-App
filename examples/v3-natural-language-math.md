# V3 Example — Natural-Language Math & Interpretation Visibility

V3 takes the LLM-decomposition story from V2 and pushes it one layer up:
the user no longer types `(3 + 5)^2 / log10(1000)`. They type a
**word problem** —

> What is the hypotenuse of a right triangle with sides 5 and 12?

— and the LLM has to do two things before it can call any tools:

1. **Recognize the math.** "Hypotenuse" + "right triangle" + "two sides"
   is the Pythagorean theorem. So the LLM mentally rewrites the
   question as `c = √(a² + b²)`.
2. **Decompose into primitives.** With `a = 5` and `b = 12`, the LLM
   issues four ordered tool calls: `power(5, 2)`, `power(12, 2)`,
   `add(25, 144)`, `sqrt(169)`.

V3 makes both steps visible in the widget. The new **interpretation
panel** shows the natural-language teaching loop in four rows:

```
phrasing    What is the hypotenuse of a right triangle with sides 5 and 12?
concept     Pythagorean theorem: c = √(a² + b²)
expression  √(5² + 12²)
answer      13   (the hypotenuse is 13)
```

…sitting directly above the V2 step list, which still shows the four
ordered tool calls the LLM actually executed. Together they answer the
question the user is really asking — *how did you get from my words to
the number?*

## Teaching point

> The MCP server stays a flat collection of primitive tools. It does
> *not* parse English, it does not parse expressions, and it does not
> know what "hypotenuse" means. **Interpretation lives in the LLM**;
> the widget *displays* the interpretation it receives.

This is the same separation V2 introduced for operator precedence,
applied to natural language: the LLM owns the semantic step (phrasing
→ math), the server owns the arithmetic, the widget owns visualization
of both.

## What V3 adds

### Server: `get_constant(name)`

V3 adds one new primitive: `get_constant(name)`. It returns `π` or `e`
in the same `CalcOutput` shape as the arithmetic tools so the LLM can
compose it with `multiply`, `power`, etc. This is what unlocks the
circle-area demo without forcing the LLM to memorize `3.14159…` (and
without giving the server a magic constants table baked into every
arithmetic op).

```jsonc
// success
{ "ok": "true",  "op": "get_constant(pi)", "inputs": [],
  "result": 3.141592653589793, "display": "3.1415926535" }

// unknown name
{ "ok": "false", "op": "get_constant", "inputs": [],
  "code": "unknown_constant",
  "message": "Unknown constant 'phi'. Supported: pi, e." }
```

Only `pi` and `e` are supported — V3 deliberately does not become a
constants library. If the LLM needs `phi` or `c` (speed of light) it
should pass the literal value to the arithmetic tools.

### Widget: interpretation panel

The widget gains a panel above the existing step list with four rows:

| Row | What it shows | Filled by |
|---|---|---|
| `phrasing` | The user's original natural-language question | LLM (passes through) |
| `concept` | The math the LLM recognized (e.g. *Pythagorean theorem*) | LLM |
| `expression` | The equivalent symbolic math (e.g. `√(5² + 12²)`) | LLM |
| `answer` | The final numeric answer — `computing…` until done | LLM (on completion) |

The widget listens for a new client-side notification:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "ui/notifications/interpretation",
  "params": {
    "phrasing": "What is the hypotenuse of a right triangle with sides 5 and 12?",
    "concept":  "Pythagorean theorem: c = √(a² + b²)",
    "expression": "√(5² + 12²)"
  }
}
```

The host typically sends two of these per word problem — one before
the tool calls (phrasing/concept/expression filled, no answer yet) and
one after (`answer` filled, `complete: true`). Each envelope is
shallow-merged into widget state so partial updates don't blow away
earlier fields.

`ui/notifications/tool-result` (V2) still drives the step list. The
two streams render independently, so the user sees:

```
─────────────────────────────────────────
 Interpretation
   phrasing    What is the hypotenuse … sides 5 and 12?
   concept     Pythagorean theorem: c = √(a² + b²)
   expression  √(5² + 12²)
   answer      13
─────────────────────────────────────────
 Tool calls (LLM decomposition)
   1. power(5, 2)    → 25
   2. power(12, 2)   → 144
   3. add(25, 144)   → 169
   4. sqrt(169)      → 13
─────────────────────────────────────────
```

The result-line badge still shows `llm` for the headline, just as in V2.

## Demos

Open `preview.html` and use the **V3 — natural-language word problems**
buttons in the side panel. Each demo simulates a host LLM by:

1. Pushing an interpretation envelope (phrasing/concept/expression).
2. Pushing the primitive tool-result notifications in order.
3. Pushing a completion envelope with the final `answer`.

### 1. Hypotenuse — `sides 5 and 12`

> *What is the hypotenuse of a right triangle with sides 5 and 12?*

| # | Call | Result |
|---|---|---|
| 1 | `power(5, 2)` | `25` |
| 2 | `power(12, 2)` | `144` |
| 3 | `add(25, 144)` | `169` |
| 4 | `sqrt(169)` | `13` |

Concept: *Pythagorean theorem: c = √(a² + b²)*. The widget renders the
interpretation rows first, then animates each tool call into the step
list, then fills in the `answer` row with `13`.

### 2. Circle area — `r = 3`

> *What is the area of a circle with radius 3?*

| # | Call | Result |
|---|---|---|
| 1 | `get_constant("pi")` | `3.1415926535…` |
| 2 | `power(3, 2)` | `9` |
| 3 | `multiply(π, 9)` | `28.2743…` |

Concept: *Area of a circle: A = π · r²*. This demo is the reason
`get_constant` exists — the LLM looks up π as a primitive value and
composes it through the existing arithmetic tools, with no special
"area-of-circle" server tool needed.

### 3. 20% discount on 80

> *If a 20% discount is applied to 80, what is the final price?*

| # | Call | Result |
|---|---|---|
| 1 | `multiply(80, 0.2)` | `16` |
| 2 | `subtract(80, 16)` | `64` |

Concept: *Final price = original − (original × discount rate)*. Pure V1
arithmetic — what's V3 about this one is that the LLM had to translate
"20% discount applied to 80" into the formula at all. The widget shows
that translation, then the two primitive calls.

## Why no `evaluate_word_problem` tool?

The same reason V2 didn't add an `evaluate_expression` parser — pushing
interpretation into the server would short-circuit the teaching point.
LLMs are already good at recognizing word problems and decomposing
them. The interesting question is "how do you make that work
*visible* to the user?", and that's a UI problem, not a server one.

The server stays five primitive arithmetic tools, three scientific
primitives, and one constants lookup. The widget displays the LLM's
work. The chat owns the conversation. Every time we're tempted to add
"smart" behavior to the server, we ask: would this take a step out of
the user's view?

## Try it locally

```bash
cargo test
cargo build --release
./target/release/scientific-calculator-mcp-app
```

Then in a host chat:

```
What is the hypotenuse of a right triangle with sides 5 and 12?
```

Or open `preview.html` in a browser and click **Hypotenuse (sides 5,
12)** in the V3 side panel — it simulates the full teaching loop
(interpretation → tool calls → final answer) so you can see the
widget without running a host LLM.

## SDK note

The `ui/notifications/interpretation` envelope is a widget-side
convention V3 introduces. The MCP Apps bridge in pmcp 2.6 does not
expose a typed API for the host to send custom widget notifications,
so in production the host would need to post the envelope via the same
`postMessage` mechanism it uses for `ui/notifications/tool-result`.
The preview harness already does this. If a future pmcp release adds a
typed "push interpretation envelope" API, the widget contract above
(four optional rows, shallow-merge semantics) is what it should
target.

## Out of scope for V3

- **Code mode (`validate_code` / `execute_code`).** The full design for
  letting the LLM hand off a snippet of code to the server (instead of
  ordered tool calls) needs more work — it raises sandboxing,
  language-choice, and trust questions V3 doesn't try to answer.
- **Plotting.** Word problems with a graphical answer (e.g. "plot
  y = x²") want a different widget shape, deferred to a later version.
- **Calculator history.** The chat transcript and stacked widget runs
  remain the history. We deliberately don't add an in-widget log.
