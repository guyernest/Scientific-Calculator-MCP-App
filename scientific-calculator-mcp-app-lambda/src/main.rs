//! Lambda wrapper for the Scientific Calculator MCP server.
//!
//! Starts the MCP HTTP server in-process on a loopback address on first
//! invocation, then proxies every Lambda HTTP event to it. CORS / DNS
//! rebinding / security headers are handled by the SDK's Tower layers
//! applied automatically by `StreamableHttpServer::start()`.

use lambda_http::{run, service_fn, Body, Error, Request, Response};
use once_cell::sync::OnceCell;
use pmcp::server::streamable_http_server::{StreamableHttpServer, StreamableHttpServerConfig};
use reqwest::Client;
use scientific_calculator_mcp_app::build_server;
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

static BASE_URL: OnceCell<String> = OnceCell::new();
static HTTP: OnceCell<Client> = OnceCell::new();

async fn start_http_in_background() -> pmcp::Result<SocketAddr> {
    let server = build_server()?;
    let server = std::sync::Arc::new(tokio::sync::Mutex::new(server));

    let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let config = StreamableHttpServerConfig::stateless();
    let http_server = StreamableHttpServer::with_config(addr, server, config);

    let (bound, handle) = http_server.start().await?;
    tracing::info!("Calculator MCP server started on {}", bound);

    tokio::spawn(async move {
        if let Err(e) = handle.await {
            tracing::error!("HTTP server error: {}", e);
        }
    });

    Ok(bound)
}

async fn ensure_server_started() -> Result<String, Error> {
    if let Some(url) = BASE_URL.get() {
        return Ok(url.clone());
    }

    let bound = start_http_in_background()
        .await
        .map_err(|e| lambda_http::Error::from(e.to_string()))?;

    let base = format!("http://{}", bound);
    let _ = BASE_URL.set(base.clone());
    let _ = HTTP.set(Client::builder().build().unwrap());
    Ok(base)
}

async fn handler(event: Request) -> Result<Response<Body>, Error> {
    let method = event.method().clone();
    let path_q = event
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    // Forward everything (including GET — the SDK serves SSE for GET /mcp per
    // MCP streamable-HTTP spec). Do not intercept here; the SDK is the source
    // of truth for protocol compliance.
    let base = ensure_server_started().await?;
    let client = HTTP.get().expect("client");

    let url = format!("{}{}", base, path_q);
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| lambda_http::Error::from(e.to_string()))?;

    let mut req = client.request(reqwest_method, &url);

    for (name, value) in event.headers() {
        if let Ok(val) = value.to_str() {
            if name.as_str().eq_ignore_ascii_case("host") {
                continue;
            }
            req = req.header(name.as_str(), val);
        }
    }

    let body_bytes = match event.body() {
        Body::Empty => Vec::new(),
        Body::Text(s) => s.as_bytes().to_vec(),
        Body::Binary(b) => b.clone(),
    };
    req = req.body(body_bytes);

    let resp = req
        .send()
        .await
        .map_err(|e| lambda_http::Error::from(e.to_string()))?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| lambda_http::Error::from(e.to_string()))?;

    let mut builder = Response::builder().status(status.as_u16());
    for (name, value) in headers.iter() {
        if let Ok(val) = value.to_str() {
            if name.as_str().eq_ignore_ascii_case("transfer-encoding")
                || name.as_str().eq_ignore_ascii_case("content-length")
            {
                continue;
            }
            builder = builder.header(name.as_str(), val);
        }
    }

    Ok(builder.body(Body::Binary(bytes.to_vec())).unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(false)
        .try_init();

    run(service_fn(handler)).await
}
