//! The main entry point for the mcp-switchboard proxy server.
//! This application boots up an HTTP/HTTPS server using Axum to listen for incoming
//! MCP clients and routes their requests to configured backends. It handles configuration
//! parsing, command-line argument processing, and sets up tracing logs.

use axum::{
    routing::get,
    Router,
};
use clap::Parser;
use reqwest::Client;
use std::sync::Arc;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

use mcp_switchboard::config::AppConfig;
use mcp_switchboard::proxy::{proxy_handler, AppState};

/// Version constant for the application
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Command-line arguments for configuring the Switchboard server instance.
#[derive(Parser, Debug)]
#[command(name = "mcp-switchboard", version = VERSION, about = "MCP reverse proxy and token saver")]
struct Args {
    /// Path to the YAML configuration file defining the backends and rules.
    #[arg(short, long)]
    config: String,

    /// Host IP address to bind the proxy server to.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port number to listen on.
    #[arg(short, long, default_value_t = 8000)]
    port: u16,

    /// Path to the SSL private key file for HTTPS/TLS termination.
    #[arg(long)]
    ssl_keyfile: Option<String>,

    /// Path to the SSL certificate file for HTTPS/TLS termination.
    #[arg(long)]
    ssl_certfile: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing/logging using environment variables or defaults for standard visibility.
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    // Load backend configuration rules. We fail early if the config is invalid to prevent running in a broken state.
    let app_config = AppConfig::load_from_file(&args.config)?;

    // Reuse a single connection pool Client across all requests to optimize socket reuse and lower latency.
    let client = Client::builder().build()?;

    let backends = Arc::new(app_config.servers);
    let state = AppState {
        backends: backends.clone(),
        client,
    };

    // Route format captures the backend name dynamically (e.g., /my_backend/mcp)
    // so a single running proxy can multiplex requests across multiple upstreams.
    let mut app = Router::new()
        .route(
            "/:name/mcp",
            get(proxy_handler)
                .post(proxy_handler)
                .delete(proxy_handler),
        );

    // Apply CORS configuration dynamically. This allows web-based LLM clients (like browser extensions or web UIs)
    // to securely interact with the proxy across different origins.
    if let Some(cors_cfg) = app_config.cors {
        if !cors_cfg.allow_origins.is_empty() {
            let origins: Vec<_> = cors_cfg
                .allow_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            let cors = CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods(AllowMethods::any())
                .allow_headers(AllowHeaders::any())
                .expose_headers([
                    "mcp-session-id".parse().unwrap(),
                    "content-type".parse().unwrap(),
                ]);
            app = app.layer(cors);
        }
    }

    let app = app.with_state(state);

    let addr: std::net::SocketAddr = format!("{}:{}", args.host, args.port).parse()?;

    // Support both HTTPS (with TLS certs) and standard HTTP depending on the provided flags.
    // Production deployments or corporate networks usually require TLS for end-to-end data security.
    if let (Some(keyfile), Some(certfile)) = (args.ssl_keyfile, args.ssl_certfile) {
        tracing::info!("mcp-switchboard listening on https://{} (TLS enabled)", addr);
        axum_server::bind_rustls(
            addr,
            axum_server::tls_rustls::RustlsConfig::from_pem_file(certfile, keyfile).await?,
        )
        .serve(app.into_make_service())
        .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("mcp-switchboard listening on http://{}", addr);
        axum::serve(listener, app).await?;
    }

    Ok(())
}
