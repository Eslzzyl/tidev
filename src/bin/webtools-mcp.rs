#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tidev::webtools::run().await
}