//! Scientific Calculator MCP App — library (V1 + V2 tools)
//!
//! Library exposing `build_server()` so both the local HTTP binary
//! (`src/main.rs`) and the AWS Lambda wrapper
//! (`scientific-calculator-mcp-app-lambda/src/main.rs`) can construct the
//! same MCP server.
//!
//! V1: primitive arithmetic (`add`, `subtract`, `multiply`, `divide`,
//! `negate`).
//!
//! V2: adds scientific primitives (`power`, `sqrt`, `log` with explicit
//! base) so the host LLM can decompose expressions like
//! `(3 + 5)^2 / log10(1000)` into ordered primitive tool calls. The
//! server intentionally remains a flat collection of primitives — there is
//! no `evaluate_expression` parser. Operator precedence is the LLM's job;
//! the widget visualizes the ordered decomposition in a step list.
//!
//! V3: adds `get_constant(name)` so the LLM can look up `pi` / `e` while
//! decomposing word problems (e.g. circle area) into primitive arithmetic.
//! The widget gains an *interpretation* panel that captures the natural-
//! language teaching loop:
//! `user phrasing → interpreted math → executed tool calls → final answer`.
//! The server still does not parse phrasing; interpretation lives in the
//! LLM and is *displayed* by the widget.
//!
//! See `src/main.rs` for the design narrative and `examples/` for usage.

use pmcp::server::simple_resources::ResourceCollection;
use pmcp::server::typed_tool::TypedToolWithOutput;
use pmcp::server::{Server, ServerBuilder};
use pmcp::types::mcp_apps::HostType;
use pmcp::types::ui::{UIResource, UIResourceContents};
use pmcp::{RequestHandlerExtra, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// =============================================================================
// Embedded widgets — bundled at compile time so both local and Lambda runtimes
// have everything they need without disk access.
// =============================================================================

const KEYPAD_HTML: &str = include_str!("../widgets/keypad.html");
const KEYPAD_URI: &str = "ui://app/keypad";

// =============================================================================
// Tool Inputs
// =============================================================================

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BinaryInput {
    /// Left-hand operand.
    pub a: f64,
    /// Right-hand operand.
    pub b: f64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UnaryInput {
    /// Operand.
    pub x: f64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PowerInput {
    /// Base.
    pub base: f64,
    /// Exponent.
    pub exponent: f64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LogInput {
    /// Argument (must be > 0).
    pub x: f64,
    /// Logarithm base (must be > 0 and != 1).
    pub base: f64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConstantInput {
    /// Name of the mathematical constant to look up. Currently supports
    /// `"pi"` and `"e"` (case-insensitive).
    pub name: String,
}

// =============================================================================
// Tool Outputs
// =============================================================================

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CalcResult {
    pub op: String,
    pub inputs: Vec<f64>,
    pub result: f64,
    pub display: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CalcError {
    pub op: String,
    pub inputs: Vec<f64>,
    pub code: String,
    pub message: String,
}

/// MCP `outputSchema` requires top-level `"type": "object"` so hosts can
/// validate tool results as objects. `serde(tag = ...)` on an enum makes
/// schemars emit a bare `oneOf` without that hint, which trips the
/// MCP-conformance "outputSchema missing type: object" warning. We use
/// `#[schemars(extend(...))]` to inject the field while keeping the
/// discriminated-union JSON shape (`{"ok": "true", ...}` /
/// `{"ok": "false", ...}`) the widget contract relies on.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "ok")]
#[schemars(extend("type" = "object"))]
pub enum CalcOutput {
    #[serde(rename = "true")]
    Ok {
        #[serde(flatten)]
        value: CalcResult,
    },
    #[serde(rename = "false")]
    Err {
        #[serde(flatten)]
        error: CalcError,
    },
}

impl CalcOutput {
    fn ok(op: &str, inputs: Vec<f64>, result: f64) -> Self {
        CalcOutput::Ok {
            value: CalcResult {
                op: op.to_string(),
                inputs,
                result,
                display: format_number(result),
            },
        }
    }

    fn err(op: &str, inputs: Vec<f64>, code: &str, message: impl Into<String>) -> Self {
        CalcOutput::Err {
            error: CalcError {
                op: op.to_string(),
                inputs,
                code: code.to_string(),
                message: message.into(),
            },
        }
    }
}

pub fn format_number(x: f64) -> String {
    if !x.is_finite() {
        if x.is_nan() {
            return "NaN".to_string();
        }
        return if x > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() };
    }
    if x == 0.0 {
        return "0".to_string();
    }
    if x.fract() == 0.0 && x.abs() < 1e16 {
        return format!("{}", x as i64);
    }
    let s = format!("{:.10}", x);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

// =============================================================================
// Tool Handlers
// =============================================================================

fn validate_binary(op: &str, a: f64, b: f64) -> Option<CalcOutput> {
    if !a.is_finite() || !b.is_finite() {
        return Some(CalcOutput::err(
            op,
            vec![a, b],
            "invalid_input",
            format!("{} requires finite numeric inputs.", op),
        ));
    }
    None
}

fn validate_unary(op: &str, x: f64) -> Option<CalcOutput> {
    if !x.is_finite() {
        return Some(CalcOutput::err(
            op,
            vec![x],
            "invalid_input",
            format!("{} requires a finite numeric input.", op),
        ));
    }
    None
}

pub fn add(a: f64, b: f64) -> CalcOutput {
    if let Some(e) = validate_binary("add", a, b) {
        return e;
    }
    CalcOutput::ok("add", vec![a, b], a + b)
}

pub fn subtract(a: f64, b: f64) -> CalcOutput {
    if let Some(e) = validate_binary("subtract", a, b) {
        return e;
    }
    CalcOutput::ok("subtract", vec![a, b], a - b)
}

pub fn multiply(a: f64, b: f64) -> CalcOutput {
    if let Some(e) = validate_binary("multiply", a, b) {
        return e;
    }
    CalcOutput::ok("multiply", vec![a, b], a * b)
}

pub fn divide(a: f64, b: f64) -> CalcOutput {
    if let Some(e) = validate_binary("divide", a, b) {
        return e;
    }
    if b == 0.0 {
        return CalcOutput::err(
            "divide",
            vec![a, b],
            "divide_by_zero",
            "Cannot divide by zero.",
        );
    }
    CalcOutput::ok("divide", vec![a, b], a / b)
}

pub fn negate(x: f64) -> CalcOutput {
    if let Some(e) = validate_unary("negate", x) {
        return e;
    }
    CalcOutput::ok("negate", vec![x], -x)
}

// =============================================================================
// V2 — Scientific primitives
//
// These exist so the host LLM can decompose expressions like
// `(3 + 5)^2 / log10(1000)` into ordered primitive tool calls. The server is
// still NOT an expression parser — precedence and ordering live in the LLM.
// =============================================================================

pub fn power(base: f64, exponent: f64) -> CalcOutput {
    if !base.is_finite() || !exponent.is_finite() {
        return CalcOutput::err(
            "power",
            vec![base, exponent],
            "invalid_input",
            "power requires finite numeric inputs.",
        );
    }
    let r = base.powf(exponent);
    if !r.is_finite() {
        return CalcOutput::err(
            "power",
            vec![base, exponent],
            "domain_error",
            "power produced a non-finite result (overflow or undefined).",
        );
    }
    CalcOutput::ok("power", vec![base, exponent], r)
}

pub fn sqrt(x: f64) -> CalcOutput {
    if let Some(e) = validate_unary("sqrt", x) {
        return e;
    }
    if x < 0.0 {
        return CalcOutput::err(
            "sqrt",
            vec![x],
            "domain_error",
            "sqrt is undefined for negative numbers in the reals.",
        );
    }
    CalcOutput::ok("sqrt", vec![x], x.sqrt())
}

pub fn log(x: f64, base: f64) -> CalcOutput {
    if !x.is_finite() || !base.is_finite() {
        return CalcOutput::err(
            "log",
            vec![x, base],
            "invalid_input",
            "log requires finite numeric inputs.",
        );
    }
    if x <= 0.0 {
        return CalcOutput::err(
            "log",
            vec![x, base],
            "domain_error",
            "log argument must be positive.",
        );
    }
    if base <= 0.0 || base == 1.0 {
        return CalcOutput::err(
            "log",
            vec![x, base],
            "domain_error",
            "log base must be positive and not equal to 1.",
        );
    }
    CalcOutput::ok("log", vec![x, base], x.log(base))
}

// =============================================================================
// V3 — Mathematical constants
//
// `get_constant(name)` lets the LLM look up π and e while decomposing word
// problems (e.g. "area of a circle with radius 3" → `power(3, 2)` then
// `multiply(pi, 9)`). It returns the same CalcOutput shape as the arithmetic
// tools so the widget's step list and interpretation panel can render it
// uniformly. The server still doesn't parse phrasing — it just hands the LLM
// a primitive lookup it can compose with the arithmetic primitives.
// =============================================================================

pub fn get_constant(name: &str) -> CalcOutput {
    let key = name.trim().to_lowercase();
    let (canonical, value) = match key.as_str() {
        "pi" | "π" => ("pi", std::f64::consts::PI),
        "e" => ("e", std::f64::consts::E),
        _ => {
            return CalcOutput::Err {
                error: CalcError {
                    op: "get_constant".to_string(),
                    inputs: vec![],
                    code: "unknown_constant".to_string(),
                    message: format!(
                        "Unknown constant '{}'. Supported: pi, e.",
                        name
                    ),
                },
            };
        }
    };
    CalcOutput::Ok {
        value: CalcResult {
            op: format!("get_constant({})", canonical),
            inputs: vec![],
            result: value,
            display: format_number(value),
        },
    }
}

macro_rules! binary_handler {
    ($name:ident, $f:ident) => {
        fn $name(
            input: BinaryInput,
            _extra: RequestHandlerExtra,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CalcOutput>> + Send>> {
            Box::pin(async move { Ok($f(input.a, input.b)) })
        }
    };
}

binary_handler!(add_handler, add);
binary_handler!(subtract_handler, subtract);
binary_handler!(multiply_handler, multiply);
binary_handler!(divide_handler, divide);

fn negate_handler(
    input: UnaryInput,
    _extra: RequestHandlerExtra,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CalcOutput>> + Send>> {
    Box::pin(async move { Ok(negate(input.x)) })
}

fn power_handler(
    input: PowerInput,
    _extra: RequestHandlerExtra,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CalcOutput>> + Send>> {
    Box::pin(async move { Ok(power(input.base, input.exponent)) })
}

fn sqrt_handler(
    input: UnaryInput,
    _extra: RequestHandlerExtra,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CalcOutput>> + Send>> {
    Box::pin(async move { Ok(sqrt(input.x)) })
}

fn log_handler(
    input: LogInput,
    _extra: RequestHandlerExtra,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CalcOutput>> + Send>> {
    Box::pin(async move { Ok(log(input.x, input.base)) })
}

fn get_constant_handler(
    input: ConstantInput,
    _extra: RequestHandlerExtra,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CalcOutput>> + Send>> {
    Box::pin(async move { Ok(get_constant(&input.name)) })
}

// =============================================================================
// Server builder — used by both the local HTTP binary and the Lambda wrapper.
// =============================================================================

pub fn build_server() -> Result<Server> {
    let resources = ResourceCollection::new().add_ui_resource(
        UIResource::html_mcp_app(KEYPAD_URI, "keypad"),
        UIResourceContents::html(KEYPAD_URI, KEYPAD_HTML),
    );

    ServerBuilder::new()
        .name("scientific-calculator")
        .version("0.1.0")
        .tool(
            "add",
            TypedToolWithOutput::new("add", add_handler)
                .with_description("Add two numbers. Returns { ok: true, op, inputs, result, display }.")
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "subtract",
            TypedToolWithOutput::new("subtract", subtract_handler)
                .with_description("Subtract b from a. Returns { ok: true, op, inputs, result, display }.")
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "multiply",
            TypedToolWithOutput::new("multiply", multiply_handler)
                .with_description("Multiply two numbers. Returns { ok: true, op, inputs, result, display }.")
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "divide",
            TypedToolWithOutput::new("divide", divide_handler)
                .with_description(
                    "Divide a by b. On b == 0, returns { ok: false, code: 'divide_by_zero', ... }.",
                )
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "negate",
            TypedToolWithOutput::new("negate", negate_handler)
                .with_description("Negate x. Returns { ok: true, op, inputs, result, display }.")
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "power",
            TypedToolWithOutput::new("power", power_handler)
                .with_description(
                    "Raise base to exponent (base^exponent). \
                     Returns a structured CalcOutput. On overflow or undefined results \
                     (e.g. 0^-1, (-1)^0.5), returns { ok: false, code: 'domain_error', ... }.",
                )
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "sqrt",
            TypedToolWithOutput::new("sqrt", sqrt_handler)
                .with_description(
                    "Square root of x. For x < 0, returns \
                     { ok: false, code: 'domain_error', ... }.",
                )
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "log",
            TypedToolWithOutput::new("log", log_handler)
                .with_description(
                    "Logarithm of x with the given base (e.g. log(1000, 10) = 3). \
                     For x <= 0 or base <= 0 or base == 1, returns \
                     { ok: false, code: 'domain_error', ... }. \
                     Use base = 10 for log10, base = 2.718281828459045 for ln.",
                )
                .with_ui(KEYPAD_URI),
        )
        .tool(
            "get_constant",
            TypedToolWithOutput::new("get_constant", get_constant_handler)
                .with_description(
                    "Look up a mathematical constant by name. Supported names: \
                     'pi' (3.14159…) and 'e' (2.71828…). Returns the same \
                     CalcOutput shape as the arithmetic tools so the result \
                     can be fed directly into multiply/divide/power. \
                     Useful when decomposing word problems like 'area of a \
                     circle with radius r' → get_constant('pi'), \
                     power(r, 2), multiply(pi, r²). Unknown names return \
                     { ok: false, code: 'unknown_constant', ... }.",
                )
                .with_ui(KEYPAD_URI),
        )
        .resources(resources)
        .with_host_layer(HostType::ChatGpt)
        .build()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ok(out: &CalcOutput, expected_op: &str, expected: f64) {
        match out {
            CalcOutput::Ok { value } => {
                assert_eq!(value.op, expected_op);
                assert!(
                    (value.result - expected).abs() < 1e-9,
                    "expected {} got {}",
                    expected,
                    value.result
                );
            }
            CalcOutput::Err { error } => panic!("expected Ok, got Err: {:?}", error),
        }
    }

    fn assert_err(out: &CalcOutput, expected_code: &str) {
        match out {
            CalcOutput::Err { error } => assert_eq!(error.code, expected_code),
            CalcOutput::Ok { value } => panic!("expected Err, got Ok: {:?}", value),
        }
    }

    #[test]
    fn add_basic() {
        assert_ok(&add(1.0, 1.0), "add", 2.0);
        assert_ok(&add(-3.5, 4.5), "add", 1.0);
    }

    #[test]
    fn subtract_basic() {
        assert_ok(&subtract(10.0, 3.0), "subtract", 7.0);
        assert_ok(&subtract(0.0, 5.0), "subtract", -5.0);
    }

    #[test]
    fn multiply_basic() {
        assert_ok(&multiply(6.0, 7.0), "multiply", 42.0);
        assert_ok(&multiply(0.0, 1234.0), "multiply", 0.0);
    }

    #[test]
    fn divide_basic() {
        assert_ok(&divide(64.0, 8.0), "divide", 8.0);
    }

    #[test]
    fn divide_by_zero_is_structured_error() {
        assert_err(&divide(1.0, 0.0), "divide_by_zero");
        match divide(1.0, 0.0) {
            CalcOutput::Err { error } => {
                assert_eq!(error.op, "divide");
                assert_eq!(error.inputs, vec![1.0, 0.0]);
                assert!(error.message.to_lowercase().contains("zero"));
            }
            _ => panic!("expected Err"),
        }
    }

    #[test]
    fn negate_basic() {
        assert_ok(&negate(5.0), "negate", -5.0);
        assert_ok(&negate(-2.5), "negate", 2.5);
    }

    #[test]
    fn invalid_inputs_are_structured_errors() {
        assert_err(&add(f64::NAN, 1.0), "invalid_input");
        assert_err(&divide(f64::INFINITY, 1.0), "invalid_input");
        assert_err(&negate(f64::NAN), "invalid_input");
    }

    #[test]
    fn format_number_strips_trailing_zeros() {
        assert_eq!(format_number(2.0), "2");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(1.5), "1.5");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
    }

    #[test]
    fn structured_output_serializes_with_ok_discriminator() {
        let v = serde_json::to_value(add(1.0, 1.0)).unwrap();
        assert_eq!(v["ok"], serde_json::Value::String("true".to_string()));
        assert_eq!(v["result"], 2.0);
        assert_eq!(v["display"], "2");

        let v = serde_json::to_value(divide(1.0, 0.0)).unwrap();
        assert_eq!(v["ok"], serde_json::Value::String("false".to_string()));
        assert_eq!(v["code"], "divide_by_zero");
    }

    // ---------------------------------------------------------------------
    // V2 — scientific primitives
    // ---------------------------------------------------------------------

    #[test]
    fn power_basic() {
        assert_ok(&power(2.0, 10.0), "power", 1024.0);
        assert_ok(&power(8.0, 2.0), "power", 64.0);
        assert_ok(&power(9.0, 0.5), "power", 3.0);
        assert_ok(&power(2.0, -1.0), "power", 0.5);
        assert_ok(&power(0.0, 0.0), "power", 1.0); // f64 convention: 0^0 = 1
    }

    #[test]
    fn power_invalid_input() {
        assert_err(&power(f64::NAN, 2.0), "invalid_input");
        assert_err(&power(2.0, f64::INFINITY), "invalid_input");
    }

    #[test]
    fn power_domain_error_on_undefined_result() {
        // (-1)^0.5 is non-real -> NaN
        assert_err(&power(-1.0, 0.5), "domain_error");
        // 0^-1 -> Infinity
        assert_err(&power(0.0, -1.0), "domain_error");
    }

    #[test]
    fn sqrt_basic() {
        assert_ok(&sqrt(0.0), "sqrt", 0.0);
        assert_ok(&sqrt(1.0), "sqrt", 1.0);
        assert_ok(&sqrt(64.0), "sqrt", 8.0);
        assert_ok(&sqrt(2.0), "sqrt", std::f64::consts::SQRT_2);
    }

    #[test]
    fn sqrt_negative_is_domain_error() {
        assert_err(&sqrt(-1.0), "domain_error");
        assert_err(&sqrt(-1e-12), "domain_error");
    }

    #[test]
    fn sqrt_invalid_input() {
        assert_err(&sqrt(f64::NAN), "invalid_input");
        assert_err(&sqrt(f64::INFINITY), "invalid_input");
    }

    #[test]
    fn log_basic() {
        assert_ok(&log(1000.0, 10.0), "log", 3.0);
        assert_ok(&log(8.0, 2.0), "log", 3.0);
        assert_ok(&log(1.0, 10.0), "log", 0.0);
        assert_ok(&log(std::f64::consts::E, std::f64::consts::E), "log", 1.0);
    }

    #[test]
    fn log_domain_errors() {
        assert_err(&log(-1.0, 10.0), "domain_error");
        assert_err(&log(0.0, 10.0), "domain_error");
        assert_err(&log(10.0, 0.0), "domain_error");
        assert_err(&log(10.0, 1.0), "domain_error");
        assert_err(&log(10.0, -2.0), "domain_error");
    }

    #[test]
    fn log_invalid_input() {
        assert_err(&log(f64::NAN, 10.0), "invalid_input");
        assert_err(&log(10.0, f64::INFINITY), "invalid_input");
    }

    /// Walks the example expression `(3 + 5)^2 / log10(1000) = 64 / 3` using
    /// only the primitive tools. This is the canonical V2 decomposition test:
    /// the LLM produces this ordered sequence and the server provides every
    /// step.
    #[test]
    fn decomposition_example_3_plus_5_squared_over_log10_1000() {
        let step1 = add(3.0, 5.0);
        assert_ok(&step1, "add", 8.0);
        let inner = match &step1 {
            CalcOutput::Ok { value } => value.result,
            _ => panic!("step1 not ok"),
        };

        let step2 = power(inner, 2.0);
        assert_ok(&step2, "power", 64.0);
        let numerator = match &step2 {
            CalcOutput::Ok { value } => value.result,
            _ => panic!("step2 not ok"),
        };

        let step3 = log(1000.0, 10.0);
        assert_ok(&step3, "log", 3.0);
        let denominator = match &step3 {
            CalcOutput::Ok { value } => value.result,
            _ => panic!("step3 not ok"),
        };

        let step4 = divide(numerator, denominator);
        match step4 {
            CalcOutput::Ok { value } => {
                assert!((value.result - 64.0 / 3.0).abs() < 1e-9);
            }
            CalcOutput::Err { error } => panic!("final divide failed: {:?}", error),
        }
    }

    // ---------------------------------------------------------------------
    // V3 — natural-language helpers
    // ---------------------------------------------------------------------

    #[test]
    fn get_constant_pi_and_e() {
        match get_constant("pi") {
            CalcOutput::Ok { value } => {
                assert_eq!(value.op, "get_constant(pi)");
                assert!((value.result - std::f64::consts::PI).abs() < 1e-12);
                assert!(value.inputs.is_empty());
            }
            CalcOutput::Err { error } => panic!("expected Ok, got {:?}", error),
        }
        match get_constant("E") {
            CalcOutput::Ok { value } => {
                assert_eq!(value.op, "get_constant(e)");
                assert!((value.result - std::f64::consts::E).abs() < 1e-12);
            }
            CalcOutput::Err { error } => panic!("expected Ok, got {:?}", error),
        }
        // Unicode π and stray whitespace both resolve.
        match get_constant("  π  ") {
            CalcOutput::Ok { value } => {
                assert!((value.result - std::f64::consts::PI).abs() < 1e-12);
            }
            CalcOutput::Err { error } => panic!("expected Ok for π, got {:?}", error),
        }
    }

    #[test]
    fn get_constant_unknown_is_structured_error() {
        match get_constant("phi") {
            CalcOutput::Err { error } => {
                assert_eq!(error.code, "unknown_constant");
                assert_eq!(error.op, "get_constant");
                assert!(error.message.to_lowercase().contains("unknown"));
            }
            CalcOutput::Ok { value } => panic!("expected Err, got {:?}", value),
        }
    }

    /// Walks the canonical V3 word-problem decomposition for the
    /// hypotenuse of a right triangle with legs 5 and 12. The LLM
    /// translates the natural-language phrasing into four primitive
    /// tool calls; the server provides each step.
    #[test]
    fn hypotenuse_word_problem_decomposes_to_primitives() {
        let s1 = power(5.0, 2.0);
        assert_ok(&s1, "power", 25.0);
        let s2 = power(12.0, 2.0);
        assert_ok(&s2, "power", 144.0);
        let a_sq = match &s1 { CalcOutput::Ok { value } => value.result, _ => panic!() };
        let b_sq = match &s2 { CalcOutput::Ok { value } => value.result, _ => panic!() };
        let s3 = add(a_sq, b_sq);
        assert_ok(&s3, "add", 169.0);
        let sum = match &s3 { CalcOutput::Ok { value } => value.result, _ => panic!() };
        let s4 = sqrt(sum);
        assert_ok(&s4, "sqrt", 13.0);
    }

    /// Walks the circle-area word problem for radius 3:
    ///   get_constant("pi") → π
    ///   power(3, 2)        → 9
    ///   multiply(π, 9)     → 9π
    /// This exercises the V3 `get_constant` tool composing with V2 power
    /// and V1 multiply.
    #[test]
    fn circle_area_word_problem_uses_get_constant() {
        let pi = match get_constant("pi") {
            CalcOutput::Ok { value } => value.result,
            CalcOutput::Err { error } => panic!("get_constant(pi) failed: {:?}", error),
        };
        let r_sq = match power(3.0, 2.0) {
            CalcOutput::Ok { value } => value.result,
            CalcOutput::Err { error } => panic!("power(3,2) failed: {:?}", error),
        };
        match multiply(pi, r_sq) {
            CalcOutput::Ok { value } => {
                assert!((value.result - std::f64::consts::PI * 9.0).abs() < 1e-9);
            }
            CalcOutput::Err { error } => panic!("multiply failed: {:?}", error),
        }
    }

    #[test]
    fn server_builds() {
        assert!(build_server().is_ok());
    }

    #[test]
    fn output_schema_declares_object_type() {
        // MCP-conformance: structuredOutput / outputSchema must declare
        // `type: object` at the top level. Regression guard for the
        // `#[schemars(extend(...))]` attribute on CalcOutput.
        let schema = schemars::schema_for!(CalcOutput);
        let v = serde_json::to_value(&schema).unwrap();
        assert_eq!(
            v["type"],
            serde_json::Value::String("object".to_string()),
            "CalcOutput schema must declare type: object — got {}",
            serde_json::to_string_pretty(&v).unwrap()
        );
        // Sanity check: discriminated-union shape is preserved.
        assert!(
            v["oneOf"].is_array(),
            "CalcOutput schema should still be a oneOf union",
        );
    }
}
