#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args != ["--grant-only"] {
        anyhow::bail!("usage: scope-smoke-seed --grant-only");
    }
    api::smoke_seed::run().await
}
