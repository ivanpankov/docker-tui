use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    docker_tui::run().await
}
