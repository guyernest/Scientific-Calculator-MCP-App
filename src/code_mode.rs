//! V4 — Code mode for the calculator.
//!
//! `execute_code(expression)` runs an LLM-generated math expression as one
//! tool call instead of decomposing into many primitive calls. The chat
//! experience changes from "N step rows" to "one keypad widget with the full
//! trace inside" (see widgets/keypad.html).
//!
//! Educational framing — this is the lightest possible code mode:
//!
//! * **DSL**: pure math expressions. Numbers, the constants `pi`/`e`, the
//!   nine calculator primitives as named functions, infix `+ - * / ^`, and
//!   unary `-`. No variables, no control flow, single expression. The LLM
//!   already knows this language; we just parse and dispatch.
//!
//! * **Validation**: the parser. Unknown identifiers and shape errors are
//!   rejected before evaluation.
//!
//! * **Authorization**: none. Math is pure; there is nothing to gate.
//!
//! * **Execution**: walk the AST, dispatch each operator/call to the same
//!   primitive functions the V1/V2/V3 tools already wrap. Every dispatch
//!   appends a step to the trace, so the widget sees the same shape it sees
//!   for LLM-decomposed calls — just delivered in one tool result.
//!
//! Production code mode (see `pmcp-code-mode` in the SDK) adds HMAC token
//! signing, policy evaluators, JS sandboxing, and a two-phase
//! validate→approve→execute flow. None of that earns its keep for math —
//! the calculator is a teaching surface, not a SQL/API gateway.

use crate::{add, divide, get_constant, log, multiply, negate, power, sqrt, subtract, CalcOutput};

// =============================================================================
// AST
// =============================================================================

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64),
    /// Bare identifier like `pi` or `e` — resolved via `get_constant`.
    Const(String),
    /// `name(args...)` — resolved against the primitive registry.
    Call(String, Vec<Expr>),
    /// Desugared from `+ - * / ^` to keep one evaluation path.
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    /// Desugared from unary `-`.
    Neg(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl BinOp {
    fn primitive(self) -> &'static str {
        match self {
            BinOp::Add => "add",
            BinOp::Sub => "subtract",
            BinOp::Mul => "multiply",
            BinOp::Div => "divide",
            BinOp::Pow => "power",
        }
    }
}

// =============================================================================
// Parser — recursive descent with precedence climbing.
//
//   expr   = term (('+' | '-') term)*
//   term   = factor (('*' | '/') factor)*
//   factor = unary ('^' factor)?           (right-assoc)
//   unary  = '-' unary | atom
//   atom   = number | ident | ident '(' args? ')' | '(' expr ')'
//   args   = expr (',' expr)*
// =============================================================================

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl ParseError {
    fn new(message: impl Into<String>, position: usize) -> Self {
        Self { message: message.into(), position }
    }
}

pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let mut p = Parser::new(input);
    let expr = p.parse_expr()?;
    p.skip_ws();
    if p.pos < p.src.len() {
        let trailing = String::from_utf8_lossy(&p.src[p.pos..]);
        return Err(ParseError::new(
            format!("unexpected trailing input '{}'", trailing),
            p.pos,
        ));
    }
    Ok(expr)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { src: input.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        self.skip_ws();
        if self.src.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_term()?;
        loop {
            let op = match self.peek() {
                Some(b'+') => BinOp::Add,
                Some(b'-') => BinOp::Sub,
                _ => break,
            };
            self.pos += 1; // peek already skipped ws
            let rhs = self.parse_term()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_factor()?;
        loop {
            let op = match self.peek() {
                Some(b'*') => BinOp::Mul,
                Some(b'/') => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_factor()?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.parse_unary()?;
        if self.peek() == Some(b'^') {
            self.pos += 1;
            let rhs = self.parse_factor()?;
            return Ok(Expr::BinOp(BinOp::Pow, Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
            let inner = self.parse_unary()?;
            return Ok(Expr::Neg(Box::new(inner)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let Some(b) = self.src.get(self.pos).copied() else {
            return Err(ParseError::new("unexpected end of expression", start));
        };

        if b == b'(' {
            self.pos += 1;
            let inner = self.parse_expr()?;
            if !self.eat(b')') {
                return Err(ParseError::new("expected ')'", self.pos));
            }
            return Ok(inner);
        }

        if b.is_ascii_digit() || b == b'.' {
            return self.parse_number();
        }

        if b.is_ascii_alphabetic() || b == b'_' {
            let name = self.parse_ident();
            if self.peek() == Some(b'(') {
                self.pos += 1;
                let args = self.parse_args()?;
                if !self.eat(b')') {
                    return Err(ParseError::new(format!("expected ')' after {}(...)", name), self.pos));
                }
                return Ok(Expr::Call(name, args));
            }
            return Ok(Expr::Const(name));
        }

        Err(ParseError::new(
            format!("unexpected character '{}'", b as char),
            start,
        ))
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if self.peek() == Some(b')') {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if !self.eat(b',') {
                return Ok(args);
            }
        }
    }

    fn parse_number(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b.is_ascii_digit() || b == b'.' {
                self.pos += 1;
            } else if (b == b'e' || b == b'E')
                && self.pos + 1 < self.src.len()
                && (self.src[self.pos + 1].is_ascii_digit()
                    || self.src[self.pos + 1] == b'+'
                    || self.src[self.pos + 1] == b'-')
            {
                self.pos += 2;
                while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| ParseError::new("invalid utf-8 in number", start))?;
        let n: f64 = text
            .parse()
            .map_err(|_| ParseError::new(format!("invalid number '{}'", text), start))?;
        Ok(Expr::Num(n))
    }

    fn parse_ident(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }
}

// =============================================================================
// Evaluator — walks the AST, dispatches each call to the calculator
// primitives, records every dispatch as a step. Stops at the first failure.
// =============================================================================

#[derive(Debug)]
pub enum EvalError {
    UnknownFunction(String),
    UnknownConstant(String),
    Arity { name: String, expected: usize, got: usize },
    /// A primitive returned a structured error. The widget renders the failed
    /// step in red; the trace up to that point is preserved.
    StepFailed { step: CalcOutput },
}

pub struct EvalContext {
    pub steps: Vec<CalcOutput>,
}

impl EvalContext {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    fn dispatch(&mut self, out: CalcOutput) -> Result<f64, EvalError> {
        match &out {
            CalcOutput::Ok { value } => {
                let r = value.result;
                self.steps.push(out);
                Ok(r)
            }
            CalcOutput::Err { .. } => {
                self.steps.push(out.clone());
                Err(EvalError::StepFailed { step: out })
            }
        }
    }
}

pub fn evaluate(expr: &Expr, ctx: &mut EvalContext) -> Result<f64, EvalError> {
    match expr {
        Expr::Num(n) => Ok(*n),

        Expr::Const(name) => {
            let out = get_constant(name);
            if matches!(out, CalcOutput::Err { .. }) {
                if let CalcOutput::Err { error } = &out {
                    if error.code == "unknown_constant" {
                        return Err(EvalError::UnknownConstant(name.clone()));
                    }
                }
            }
            ctx.dispatch(out)
        }

        Expr::Neg(inner) => {
            let v = evaluate(inner, ctx)?;
            ctx.dispatch(negate(v))
        }

        Expr::BinOp(op, l, r) => {
            let a = evaluate(l, ctx)?;
            let b = evaluate(r, ctx)?;
            let out = match op {
                BinOp::Add => add(a, b),
                BinOp::Sub => subtract(a, b),
                BinOp::Mul => multiply(a, b),
                BinOp::Div => divide(a, b),
                BinOp::Pow => power(a, b),
            };
            let _ = op.primitive(); // tag for documentation; actual op name comes from CalcOutput
            ctx.dispatch(out)
        }

        Expr::Call(name, args) => {
            let mut vs = Vec::with_capacity(args.len());
            for a in args {
                vs.push(evaluate(a, ctx)?);
            }
            let out = call_primitive(name, &vs)?;
            ctx.dispatch(out)
        }
    }
}

fn call_primitive(name: &str, args: &[f64]) -> Result<CalcOutput, EvalError> {
    fn arity(name: &str, expected: usize, got: usize) -> Result<(), EvalError> {
        if expected == got {
            Ok(())
        } else {
            Err(EvalError::Arity { name: name.into(), expected, got })
        }
    }

    match name {
        "add" => {
            arity(name, 2, args.len())?;
            Ok(add(args[0], args[1]))
        }
        "subtract" => {
            arity(name, 2, args.len())?;
            Ok(subtract(args[0], args[1]))
        }
        "multiply" => {
            arity(name, 2, args.len())?;
            Ok(multiply(args[0], args[1]))
        }
        "divide" => {
            arity(name, 2, args.len())?;
            Ok(divide(args[0], args[1]))
        }
        "negate" => {
            arity(name, 1, args.len())?;
            Ok(negate(args[0]))
        }
        "power" => {
            arity(name, 2, args.len())?;
            Ok(power(args[0], args[1]))
        }
        "sqrt" => {
            arity(name, 1, args.len())?;
            Ok(sqrt(args[0]))
        }
        "log" => {
            arity(name, 2, args.len())?;
            Ok(log(args[0], args[1]))
        }
        "get_constant" => Err(EvalError::UnknownFunction(
            "get_constant takes a name, not a number — use the bare identifier (e.g. `pi`) instead".into(),
        )),
        _ => Err(EvalError::UnknownFunction(name.into())),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_ok(src: &str) -> (f64, Vec<CalcOutput>) {
        let expr = parse(src).expect("parse");
        let mut ctx = EvalContext::new();
        let r = evaluate(&expr, &mut ctx).expect("eval");
        (r, ctx.steps)
    }

    fn eval_err(src: &str) -> EvalError {
        let expr = parse(src).expect("parse");
        let mut ctx = EvalContext::new();
        evaluate(&expr, &mut ctx).expect_err("expected eval error")
    }

    #[test]
    fn parses_and_evaluates_simple_arithmetic() {
        assert_eq!(eval_ok("1 + 1").0, 2.0);
        assert_eq!(eval_ok("10 - 4").0, 6.0);
        assert_eq!(eval_ok("3 * 4").0, 12.0);
        assert_eq!(eval_ok("12 / 4").0, 3.0);
    }

    #[test]
    fn precedence_and_associativity() {
        // * before +
        assert_eq!(eval_ok("1 + 2 * 3").0, 7.0);
        // parens override
        assert_eq!(eval_ok("(1 + 2) * 3").0, 9.0);
        // ^ right-associative: 2^3^2 = 2^(3^2) = 2^9 = 512
        assert_eq!(eval_ok("2 ^ 3 ^ 2").0, 512.0);
        // unary minus
        assert_eq!(eval_ok("-(3 + 5)").0, -8.0);
    }

    #[test]
    fn function_calls_and_constants() {
        assert_eq!(eval_ok("sqrt(64)").0, 8.0);
        assert_eq!(eval_ok("power(2, 10)").0, 1024.0);
        assert!((eval_ok("log(1000, 10)").0 - 3.0).abs() < 1e-9);
        let (pi_val, _) = eval_ok("pi");
        assert!((pi_val - std::f64::consts::PI).abs() < 1e-12);
    }

    #[test]
    fn pythagorean_decomposes_to_steps() {
        // sqrt(5^2 + 12^2) = 13
        let (r, steps) = eval_ok("sqrt(power(5, 2) + power(12, 2))");
        assert!((r - 13.0).abs() < 1e-9);
        // Expect: power(5,2), power(12,2), add(25,144), sqrt(169) = 4 steps
        assert_eq!(steps.len(), 4);
    }

    #[test]
    fn circle_area_with_pi_constant() {
        // pi * 34^2 ~ 3631.68
        let (r, steps) = eval_ok("pi * 34 ^ 2");
        assert!((r - std::f64::consts::PI * 1156.0).abs() < 1e-6);
        // Expect: get_constant(pi), power(34,2), multiply(pi, 1156)
        assert_eq!(steps.len(), 3);
    }

    #[test]
    fn step_failure_is_reported_with_partial_trace() {
        // sqrt(-1) should fail with domain_error after one step.
        let err = eval_err("sqrt(-1)");
        match err {
            EvalError::StepFailed { step } => match step {
                CalcOutput::Err { error } => {
                    assert_eq!(error.op, "sqrt");
                    assert_eq!(error.code, "domain_error");
                }
                _ => panic!("expected Err step"),
            },
            _ => panic!("expected StepFailed"),
        }
    }

    #[test]
    fn divide_by_zero_propagates_as_step_failure() {
        let err = eval_err("1 / 0");
        match err {
            EvalError::StepFailed { step } => match step {
                CalcOutput::Err { error } => assert_eq!(error.code, "divide_by_zero"),
                _ => panic!("expected Err"),
            },
            _ => panic!("expected StepFailed"),
        }
    }

    #[test]
    fn unknown_function_is_rejected() {
        match eval_err("foo(1, 2)") {
            EvalError::UnknownFunction(name) => assert_eq!(name, "foo"),
            other => panic!("expected UnknownFunction, got {:?}", other),
        }
    }

    #[test]
    fn unknown_constant_is_rejected() {
        match eval_err("phi") {
            EvalError::UnknownConstant(name) => assert_eq!(name, "phi"),
            other => panic!("expected UnknownConstant, got {:?}", other),
        }
    }

    #[test]
    fn arity_mismatch_is_rejected() {
        match eval_err("sqrt(1, 2)") {
            EvalError::Arity { name, expected, got } => {
                assert_eq!(name, "sqrt");
                assert_eq!(expected, 1);
                assert_eq!(got, 2);
            }
            other => panic!("expected Arity, got {:?}", other),
        }
    }

    #[test]
    fn parse_error_on_unbalanced_parens() {
        let err = parse("1 + (2 * 3").expect_err("expected parse error");
        assert!(err.message.contains(')'));
    }

    #[test]
    fn parse_error_on_trailing_garbage() {
        let err = parse("1 + 2 garbage").expect_err("expected parse error");
        assert!(err.message.contains("trailing"));
    }

    #[test]
    fn scientific_notation_numbers() {
        assert_eq!(eval_ok("1e3").0, 1000.0);
        assert_eq!(eval_ok("1.5e2").0, 150.0);
        assert_eq!(eval_ok("2e-3").0, 0.002);
    }

    #[test]
    fn canonical_decomposition_example_runs_in_one_call() {
        // The V2 canonical example: (3 + 5)^2 / log10(1000) = 64 / 3.
        // V2 needed the LLM to make 4 separate tool calls. V4 runs it as one.
        let (r, steps) = eval_ok("(3 + 5) ^ 2 / log(1000, 10)");
        assert!((r - 64.0 / 3.0).abs() < 1e-9);
        // add, power, log, divide = 4 steps
        assert_eq!(steps.len(), 4);
    }
}
