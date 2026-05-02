//! Scientific Calculator MCP App — V1
//!
//! Educational MCP server that exposes primitive arithmetic tools and serves
//! an interactive calculator-keypad widget. The point of V1 is to make the
//! three paths of an MCP App widget visible:
//!
//! 1. **Local widget UI updates** — clicking digits and operators updates the
//!    calculator's expression/display immediately, without leaving the widget.
//! 2. **LLM reasoning path** — when the user types math in the chat (e.g.
//!    "compute 1 + 1" or "what is (3+5)*4?"), the host LLM decomposes the
//!    expression into primitive tool calls. The host then pushes the
//!    structuredContent of each tool result back to the widget via
//!    `ui/notifications/tool-result`, so the widget's display reflects the
//!    LLM-driven computation.
//! 3. **MCP server computation path** — when the user clicks `=` in the
//!    widget, the widget itself invokes a primitive tool (e.g. `add`) via
//!    `mcpBridge.callTool(...)`. This is the "widget acts as the client"
//!    flavor of the same path. The server is the only place authoritative
//!    arithmetic happens.
//!
//! V1 deliberately avoids:
//!   - Server-side expression parsing (`evaluate_expression`).
//!   - Calculator history (the chat transcript and stacked widgets are the
//!     history).
//!   - Scientific functions, plotting, code mode (those are V2+).

use async_trait::async_trait;
use pmcp::server::mcp_apps::{McpAppsAdapter, UIAdapter, WidgetDir};
use pmcp::server::streamable_http_server::{StreamableHttpServer, StreamableHttpServerConfig};
use pmcp::server::typed_tool::TypedToolWithOutput;
use pmcp::server::ServerBuilder;
use pmcp::types::mcp_apps::{ExtendedUIMimeType, HostType};
use pmcp::types::Content;
use pmcp::types::{ListResourcesResult, ReadResourceResult, ResourceInfo};
use pmcp::{RequestHandlerExtra, ResourceHandler, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

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

/// Standard structured output for a successful primitive arithmetic call.
///
/// `display` is a human-friendly string the widget can render directly without
/// having to know how the host serialized the number.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CalcResult {
    /// Tool name that produced this result (e.g. "add").
    pub op: String,
    /// Numeric inputs in the order the tool received them.
    pub inputs: Vec<f64>,
    /// Numeric result.
    pub result: f64,
    /// Pre-formatted display string for the widget.
    pub display: String,
}

/// Structured error for divide-by-zero or invalid input. Returned as an `Err`
/// branch wrapped in a discriminated union so the widget can render either
/// shape without parsing free-form strings.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CalcError {
    pub op: String,
    pub inputs: Vec<f64>,
    /// Stable machine-readable code: "divide_by_zero" | "invalid_input".
    pub code: String,
    /// Human-readable explanation for display.
    pub message: String,
}

/// Discriminated union returned by every primitive tool. Using `serde(tag, ...)`
/// gives the widget a stable shape: `{ "ok": true, ... }` or
/// `{ "ok": false, ... }`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "ok")]
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

/// Trim trailing zeros from a `f64` so the widget gets `2` not `2.0`,
/// while still preserving precision for non-integer results.
pub fn format_number(x: f64) -> String {
    if !x.is_finite() {
        if x.is_nan() {
            return "NaN".to_string();
        }
        return if x > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() };
    }
    if x == 0.0 {
        // Collapse -0.0 so the display reads "0".
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
// Tool Handlers — the actual primitive arithmetic.
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

// Small helper to wrap a sync handler into the boxed-future signature
// `TypedToolWithOutput` expects.
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
// Resource Handler — serves the keypad widget HTML.
// =============================================================================

struct CalculatorResources {
    adapter: McpAppsAdapter,
    widget_dir: WidgetDir,
}

impl CalculatorResources {
    fn new(widgets_path: PathBuf) -> Self {
        Self {
            adapter: McpAppsAdapter::new(),
            widget_dir: WidgetDir::new(widgets_path),
        }
    }
}

#[async_trait]
impl ResourceHandler for CalculatorResources {
    async fn read(&self, uri: &str, _extra: RequestHandlerExtra) -> Result<ReadResourceResult> {
        let name = uri
            .strip_prefix("ui://app/")
            .or_else(|| uri.strip_prefix("ui://calculator/"))
            .and_then(|s| s.strip_suffix(".html").or(Some(s)));

        if let Some(widget_name) = name {
            let html = self.widget_dir.read_widget(widget_name);
            let transformed = self.adapter.transform(uri, widget_name, &html);

            Ok(ReadResourceResult::new(vec![Content::Resource {
                uri: uri.to_string(),
                text: Some(transformed.content),
                mime_type: Some(ExtendedUIMimeType::HtmlMcpApp.to_string()),
                meta: None,
            }]))
        } else {
            Err(pmcp::Error::protocol(
                pmcp::ErrorCode::METHOD_NOT_FOUND,
                format!("Resource not found: {}", uri),
            ))
        }
    }

    async fn list(
        &self,
        _cursor: Option<String>,
        _extra: RequestHandlerExtra,
    ) -> Result<ListResourcesResult> {
        let entries = self.widget_dir.discover().unwrap_or_default();
        let resources = entries
            .into_iter()
            .map(|entry| {
                ResourceInfo::new(&entry.uri, &entry.filename)
                    .with_description(format!("Interactive {} widget", entry.filename))
                    .with_mime_type(ExtendedUIMimeType::HtmlMcpApp.to_string())
            })
            .collect();

        Ok(ListResourcesResult::new(resources))
    }
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let widgets_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("widgets");

    let server = ServerBuilder::new()
        .name("scientific-calculator")
        .version("0.1.0")
        .tool(
            "add",
            TypedToolWithOutput::new("add", add_handler)
                .with_description("Add two numbers. Returns { ok: true, op, inputs, result, display }.")
                .with_ui("ui://app/keypad"),
        )
        .tool(
            "subtract",
            TypedToolWithOutput::new("subtract", subtract_handler)
                .with_description("Subtract b from a. Returns { ok: true, op, inputs, result, display }.")
                .with_ui("ui://app/keypad"),
        )
        .tool(
            "multiply",
            TypedToolWithOutput::new("multiply", multiply_handler)
                .with_description("Multiply two numbers. Returns { ok: true, op, inputs, result, display }.")
                .with_ui("ui://app/keypad"),
        )
        .tool(
            "divide",
            TypedToolWithOutput::new("divide", divide_handler)
                .with_description(
                    "Divide a by b. On b == 0, returns { ok: false, code: 'divide_by_zero', ... }.",
                )
                .with_ui("ui://app/keypad"),
        )
        .tool(
            "negate",
            TypedToolWithOutput::new("negate", negate_handler)
                .with_description("Negate x. Returns { ok: true, op, inputs, result, display }.")
                .with_ui("ui://app/keypad"),
        )
        .resources(CalculatorResources::new(widgets_path))
        .with_host_layer(HostType::ChatGpt)
        .build()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    let server = Arc::new(Mutex::new(server));

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000u16);
    let addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port);

    let config = StreamableHttpServerConfig {
        session_id_generator: None,
        enable_json_response: true,
        event_store: None,
        on_session_initialized: None,
        on_session_closed: None,
        http_middleware: None,
        ..Default::default()
    };

    let http_server = StreamableHttpServer::with_config(addr, server, config);
    let (bound_addr, server_handle) = http_server
        .start()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    println!("Scientific Calculator MCP Server (V1) running at http://{}", bound_addr);
    println!();
    println!("Tools:");
    println!("  - add(a, b)        Add two numbers.");
    println!("  - subtract(a, b)   Subtract b from a.");
    println!("  - multiply(a, b)   Multiply two numbers.");
    println!("  - divide(a, b)     Divide a by b. Structured divide-by-zero error.");
    println!("  - negate(x)        Unary negation.");
    println!();
    println!("Widget resource: ui://app/keypad");
    println!();
    println!(
        "Connect with: cargo pmcp connect --server scientific-calculator \\\n             --client claude-code --url http://{}",
        bound_addr
    );

    server_handle
        .await
        .map_err(|e| Box::new(pmcp::Error::Internal(e.to_string())) as Box<dyn std::error::Error>)?;

    Ok(())
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
        // The widget can rely on { ok: false, code: "divide_by_zero", ... }.
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
}
