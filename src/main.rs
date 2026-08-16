use std::net::SocketAddr;

use chatgpt_codex_tools_mcp_rust::{config::load_config, server::build_router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config()?;
    let address: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!(
        "Forge listening on http://{}:{}/mcp ({:?}/{:?})",
        config.host, config.port, config.tool_profile, config.access_mode
    );
    axum::serve(listener, build_router(config)).await?;
    Ok(())
}
