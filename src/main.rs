//! Scientific Calculator MCP App — V3 (local HTTP binary)
//!
//! Educational MCP server that exposes primitive arithmetic + scientific
//! tools and serves an interactive calculator-keypad widget. The point is
//! to make the three paths of an MCP App widget visible:
//!
//! 1. **Local widget UI updates** — clicking digits and operators updates the
//!    calculator's expression/display immediately, without leaving the widget.
//! 2. **LLM reasoning path** — when the user types math in the chat, the host
//!    LLM decomposes the expression into primitive tool calls. The host then
//!    pushes the structuredContent of each tool result back to the widget via
//!    `ui/notifications/tool-result`. V2 adds a step-list view in the widget
//!    that visualizes the ordered decomposition.
//! 3. **MCP server computation path** — when the user clicks `=` in the
//!    widget, the widget itself invokes a primitive tool via
//!    `mcpBridge.callTool(...)`. Server is the only place authoritative
//!    arithmetic happens.
//!
//! V1 deliberately avoided scientific functions; V2 added `power`, `sqrt`,
//! `log` so the LLM can decompose expressions like `(3 + 5)^2 / log10(1000)`
//! into primitive tool calls. V3 adds `get_constant` (π, e) and an
//! interpretation panel in the widget so natural-language word problems
//! ("hypotenuse of a right triangle with sides 5 and 12") expose the full
//! teaching loop: phrasing → interpreted math → executed primitive tool
//! calls → final answer. There is still no server-side expression parser,
//! no calculator history (the chat transcript + step list are the history),
//! no plotting, and no code mode (those are V4+).
//!
//! All server building lives in `lib.rs` so the Lambda wrapper crate
//! (`scientific-calculator-mcp-app-lambda`) can reuse it.

use pmcp::server::streamable_http_server::{StreamableHttpServer, StreamableHttpServerConfig};
use scientific_calculator_mcp_app::build_server;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let server = build_server().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
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

    println!("Scientific Calculator MCP Server (V3) running at http://{}", bound_addr);
    println!();
    println!("Tools (V1 — arithmetic):");
    println!("  - add(a, b)              Add two numbers.");
    println!("  - subtract(a, b)         Subtract b from a.");
    println!("  - multiply(a, b)         Multiply two numbers.");
    println!("  - divide(a, b)           Divide a by b. Structured divide-by-zero error.");
    println!("  - negate(x)              Unary negation.");
    println!();
    println!("Tools (V2 — scientific):");
    println!("  - power(base, exponent)  base^exponent. domain_error for non-finite results.");
    println!("  - sqrt(x)                Square root. domain_error for x < 0.");
    println!("  - log(x, base)           log base of x. domain_error for x <= 0, base <= 0, base == 1.");
    println!();
    println!("Tools (V3 — natural language helpers):");
    println!("  - get_constant(name)     Look up 'pi' or 'e' as a primitive value the LLM can compose with.");
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
