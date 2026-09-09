#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let grant_only = match args.as_slice() {
        [] => false,
        [arg] if arg == "--grant-only" => true,
        _ => anyhow::bail!("usage: scope-smoke-seed [--grant-only]"),
    };
    api::smoke_seed::run(grant_only).await
}
