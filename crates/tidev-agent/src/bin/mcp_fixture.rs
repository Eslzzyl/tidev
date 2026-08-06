//! Local MCP fixture used by the tidev-agent integration tests.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::transport::stdio;
use rmcp::{ServiceExt, tool, tool_router};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
struct EchoInput {
    text: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EchoOutput {
    value: String,
}

struct FixtureServer;

#[tool_router(server_handler)]
impl FixtureServer {
    #[tool(
        name = "fixture_echo",
        description = "Return a deterministic fixture value."
    )]
    fn echo(&self, Parameters(EchoInput { text }): Parameters<EchoInput>) -> Json<EchoOutput> {
        Json(EchoOutput {
            value: format!("fixture:{text}"),
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = FixtureServer.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
