use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use puerperium::paths::Paths;
use puerperium_mcp::dispatch;
use puerperium_mcp::handlers::Face;
use puerperium_mcp::transport::StdioTransport;

/// puerperium-mcp — MCP-over-stdio server exposing the nursery_* tool surface.
///
/// stdout is the JSON-RPC stream. All tracing goes to stderr.
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let _loaded = puerperium::secrets::load();

    let paths = Paths::from_env().context(
        "no state directory: set PUERPERIUM_STATE_DIR or HOME so the nursery has somewhere to write",
    )?;
    paths.ensure()?;

    let face = Arc::new(Face { paths });
    info!("puerperium-mcp starting");

    let mut transport = StdioTransport::new();

    let init_req = transport.read().await?;
    let init_resp = if init_req["method"].as_str() == Some("initialize") {
        dispatch::handle_initialize(&init_req)
    } else {
        tracing::warn!(
            "first message was not 'initialize': {:?}",
            init_req["method"]
        );
        dispatch::method_not_found(&init_req)
    };
    transport.write(&init_resp).await?;

    loop {
        match transport.read().await {
            Err(e) => {
                if e.to_string().contains("EOF") {
                    break;
                }
                tracing::error!("transport error: {e}");
                break;
            }
            Ok(msg) => {
                // Notifications carry no "id" — never respond to them.
                let is_notification = msg["id"].is_null()
                    || msg["method"]
                        .as_str()
                        .map(|m| m.starts_with("notifications/"))
                        .unwrap_or(false);
                if is_notification {
                    continue;
                }

                let method = msg["method"].as_str().unwrap_or("").to_string();
                let resp = match method.as_str() {
                    "initialize" => dispatch::handle_initialize(&msg),
                    "ping" => dispatch::ping(&msg),
                    "tools/list" => dispatch::tools_list(&msg),
                    "tools/call" => dispatch::dispatch_tool(msg, Arc::clone(&face)).await,
                    _ => dispatch::method_not_found(&msg),
                };
                transport.write(&resp).await?;
            }
        }
    }

    info!("puerperium-mcp exiting");
    Ok(())
}
