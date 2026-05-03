//! Scientific Calculator MCP App — V1 (local HTTP binary)
//!
//! Educational MCP server that exposes primitive arithmetic tools and serves
//! an interactive calculator-keypad widget. The point of V1 is to make the
//! three paths of an MCP App widget visible:
//!
//! 1. **Local widget UI updates** — clicking digits and operators updates the
//!    calculator's expression/display immediately, without leaving the widget.
//! 2. **LLM reasoning path** — when the user types math in the chat, the host
//!    LLM decomposes the expression into primitive tool calls. The host then
//!    pushes the structuredContent of each tool result back to the widget via
//!    `ui/notifications/tool-result`.
//! 3. **MCP server computation path** — when the user clicks `=` in the
//!    widget, the widget itself invokes a primitive tool via
//!    `mcpBridge.callTool(...)`. Server is the only place authoritative
//!    arithmetic happens.
//!
//! V1 deliberately avoids server-side expression parsing, calculator history,
//! scientific functions, plotting, and code mode (those are V2+).
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
