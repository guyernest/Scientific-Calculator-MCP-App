//! Scientific Calculator MCP App — V1 (library)
//!
//! Library exposing `build_server()` so both the local HTTP binary
//! (`src/main.rs`) and the AWS Lambda wrapper
//! (`scientific-calculator-mcp-app-lambda/src/main.rs`) can construct the
//! same MCP server.
//!
//! See `src/main.rs` for the V1 design narrative.

use async_trait::async_trait;
use pmcp::server::mcp_apps::{McpAppsAdapter, UIAdapter};
use pmcp::server::typed_tool::TypedToolWithOutput;
use pmcp::server::{Server, ServerBuilder};
use pmcp::types::mcp_apps::{ExtendedUIMimeType, HostType};
use pmcp::types::Content;
use pmcp::types::{ListResourcesResult, ReadResourceResult, ResourceInfo};
use pmcp::{RequestHandlerExtra, ResourceHandler, Result};
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

// =============================================================================
// Resource Handler — serves the embedded keypad widget HTML.
// =============================================================================

struct CalculatorResources {
    adapter: McpAppsAdapter,
}

impl CalculatorResources {
    fn new() -> Self {
        Self { adapter: McpAppsAdapter::new() }
    }

    fn lookup(name: &str) -> Option<&'static str> {
        match name {
            "keypad" | "keypad.html" => Some(KEYPAD_HTML),
            _ => None,
        }
    }
}

#[async_trait]
impl ResourceHandler for CalculatorResources {
    async fn read(&self, uri: &str, _extra: RequestHandlerExtra) -> Result<ReadResourceResult> {
        let name = uri
            .strip_prefix("ui://app/")
            .or_else(|| uri.strip_prefix("ui://calculator/"))
            .map(|s| s.strip_suffix(".html").unwrap_or(s));

        if let Some(widget_name) = name {
            if let Some(html) = Self::lookup(widget_name) {
                let transformed = self.adapter.transform(uri, widget_name, html);
                return Ok(ReadResourceResult::new(vec![Content::Resource {
                    uri: uri.to_string(),
                    text: Some(transformed.content),
                    mime_type: Some(ExtendedUIMimeType::HtmlMcpApp.to_string()),
                    meta: None,
                }]));
            }
        }

        Err(pmcp::Error::protocol(
            pmcp::ErrorCode::METHOD_NOT_FOUND,
            format!("Resource not found: {}", uri),
        ))
    }

    async fn list(
        &self,
        _cursor: Option<String>,
        _extra: RequestHandlerExtra,
    ) -> Result<ListResourcesResult> {
        let resources = vec![ResourceInfo::new(KEYPAD_URI, "keypad")
            .with_description("Interactive calculator keypad widget")
            .with_mime_type(ExtendedUIMimeType::HtmlMcpApp.to_string())];
        Ok(ListResourcesResult::new(resources))
    }
}

// =============================================================================
// Server builder — used by both the local HTTP binary and the Lambda wrapper.
// =============================================================================

pub fn build_server() -> Result<Server> {
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
        .resources(CalculatorResources::new())
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
