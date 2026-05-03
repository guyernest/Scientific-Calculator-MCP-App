//! Lambda wrapper for the Scientific Calculator MCP server.
//!
//! `pmcp::server::axum_router::router_with_config` returns a fully-layered
//! `axum::Router` (CORS, DNS-rebinding protection, security headers, the
//! streamable-HTTP MCP handler). `lambda_http::run` accepts any
//! `tower::Service` over `Request`, so the Router goes in directly — no
//! loopback bind, no proxy, no header copying.
//!
//! `AllowedOrigins::any()` is correct here: the function is reachable only
//! through API Gateway (which gates origin separately), so localhost-only
//! defaults would reject every real request.
//! `StreamableHttpServerConfig::stateless()` skips per-session state since
//! Lambda invocations don't persist a long-lived session.

use lambda_http::{run, Error};
use pmcp::server::axum_router::{router_with_config, AllowedOrigins, RouterConfig};
use pmcp::server::streamable_http_server::StreamableHttpServerConfig;
use scientific_calculator_mcp_app::build_server;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(false)
        .try_init();

    let server = build_server().map_err(|e| Error::from(e.to_string()))?;
    let app = router_with_config(
        Arc::new(Mutex::new(server)),
        RouterConfig {
            allowed_origins: Some(AllowedOrigins::any()),
            server_config: StreamableHttpServerConfig::stateless(),
            ..Default::default()
        },
    );

    run(app).await
}
