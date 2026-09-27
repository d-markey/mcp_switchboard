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
use mcp_switchboard::backend_stats::BackendStatsRegistry;

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
    // Security Posture (per README): No request timeout to backends by design.
    // Timing out a long-running call is the client/harness's call to make, not the proxy's.
    let client = Client::builder().build()?;

    let backends = Arc::new(app_config.servers);

    // Initialize backend stats and health cache at startup with all counters at 0, empty tools list, and status "waiting".
    let mut backend_stats = BackendStatsRegistry::new();
    backend_stats.init_backends(backends.keys());

    let state = AppState {
        backends: backends.clone(),
        client,
        backend_stats,
    };

    // Route format captures the backend name dynamically (e.g., /my_backend/mcp)
    // so a single running proxy can multiplex requests across multiple upstreams.
    let mut app = Router::new()
        .route(
            "/:name/mcp",
            get(proxy_handler)
                .post(proxy_handler)
                .delete(proxy_handler),
        )
        .route(
            "/stats",
            get(mcp_switchboard::backend_stats::stats_handler),
        )
        .route(
            "/health",
            get(mcp_switchboard::health::health_handler),
        );

    // Apply CORS configuration dynamically. This allows web-based LLM clients (like browser extensions or web UIs)
    // to securely interact with the proxy across different origins.
    if let Some(cors_cfg) = app_config.cors {
        if !cors_cfg.allow_origins.is_empty() {
            let allowed_origin = if cors_cfg.allow_origins.iter().any(|o| o == "*") {
                AllowOrigin::any()
            } else {
                let origins: Vec<_> = cors_cfg
                    .allow_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect();
                AllowOrigin::list(origins)
            };

            let cors = CorsLayer::new()
                .allow_origin(allowed_origin)
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
