# V1 Example — Basic Arithmetic

Goal: see all three paths fire in a single conversation.

## Path 1 — Local UI updates

The user clicks `1`, `+`, `1` in the keypad. The widget shows:

```
1 + 1
        —          <- result placeholder
        local      <- badge
```

No tool was called. No network traffic.

## Path 3 — Widget calls a primitive tool

The user clicks `=`. The widget recognizes `1 + 1` as the simple
`number OP number` shape, so it calls the `add` primitive directly:

```jsonc
app.callServerTool({ name: "add", arguments: { "a": 1, "b": 1 } })
// -> { structuredContent: { "ok": "true", "op": "add", "inputs": [1, 1], "result": 2, "display": "2" }, ... }
```

The widget renders:

```
1 + 1
        2          <- result, green
        server     <- badge
```

## Path 2 — LLM reasons, server computes

The user types into the chat:

> Compute (3 + 5) * 2

V1 has no `evaluate_expression` server tool, so the LLM must decompose
the request itself:

1. `add(3, 5)` → `8`
2. `multiply(8, 2)` → `16`

For each call, the host pushes the `structuredContent` to the widget via
`ui/notifications/tool-result`. The widget displays the latest call:

```
multiply(8, 2)
        16
        llm        <- badge: the LLM drove this call
```

The chat transcript holds the full sequence — the widget only shows the
"current moment". This is the spec's main teaching point: precedence,
decomposition, and natural language are the LLM's job; arithmetic
correctness is the server's job; visualization and direct manipulation
are the widget's job.

## Divide-by-zero produces a structured error

```jsonc
app.callServerTool({ name: "divide", arguments: { "a": 1, "b": 0 } })
// -> { structuredContent: { "ok": "false", "op": "divide", "inputs": [1, 0],
//                           "code": "divide_by_zero", "message": "Cannot divide by zero." }, ... }
```

The widget renders the message in red with the `error` badge.
