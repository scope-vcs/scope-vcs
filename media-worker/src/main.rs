use anyhow::Context as _;
use scope_media_worker::{
    cleanup,
    codec::{CodecOutput, CodecPipeline, DerivativeKind, MediaKind},
    config::{CodecLimits, CodecPrograms, WorkerSettings},
    health::WorkerHealth,
    jobs,
    scratch::ScratchSpace,
    storage,
};
use scope_postgres::db::MetadataStore;
use std::{path::Path, time::Duration};
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let mut args = std::env::args_os();
    let _program = args.next();
    match args.next().as_deref().and_then(|value| value.to_str()) {
        None => run_service().await,
        Some("codec-info") if args.next().is_none() => codec_info().await,
        Some("codec-self-test") => {
            let fixture_dir = args
                .next()
                .context("usage: scope-media-worker codec-self-test FIXTURE_DIR")?;
            if args.next().is_some() {
                anyhow::bail!("usage: scope-media-worker codec-self-test FIXTURE_DIR");
            }
            codec_self_test(Path::new(&fixture_dir)).await
        }
        Some(command) => anyhow::bail!("unknown scope-media-worker command: {command}"),
    }
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scope_media_worker=info,scope_postgres=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn run_service() -> anyhow::Result<()> {
    let settings = WorkerSettings::from_env()?;
    let scratch = ScratchSpace::prepare(settings.scratch_root.clone())?;
    let pipeline = CodecPipeline::new(
        settings.codec_programs.clone(),
        settings.codec_limits.clone(),
    );
    let capabilities = pipeline
        .verify_capabilities(&settings.scratch_root)
        .await
        .context("checking required media codecs")?;
    tracing::info!(capabilities = %serde_json::to_string(&capabilities)?, "media codecs ready");

    let storage = storage::from_env().context("configuring encrypted media storage")?;
    let health = WorkerHealth::new(settings.poll_interval);
    health.mark_codecs_ready();
    let mut health_task = tokio::spawn(health.clone().serve(settings.health_port));
    let Some(metadata) = connect_metadata(&settings, &health, &mut health_task).await? else {
        return Ok(());
    };

    let mut processing_task = tokio::spawn(jobs::run_processing_loop(
        metadata.clone(),
        storage.clone(),
        pipeline,
        scratch,
        settings.clone(),
        health.clone(),
    ));
    let mut cleanup_task = tokio::spawn(cleanup::run(metadata, storage, settings, health));

    let result = tokio::select! {
        result = &mut health_task => flatten_task("health server", result),
        result = &mut processing_task => flatten_task("processing loop", result),
        result = &mut cleanup_task => flatten_task("cleanup loop", result),
    };
    health_task.abort();
    processing_task.abort();
    cleanup_task.abort();
    result
}

async fn connect_metadata(
    settings: &WorkerSettings,
    health: &WorkerHealth,
    health_task: &mut tokio::task::JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<Option<MetadataStore>> {
    loop {
        let connection = MetadataStore::connect_worker(settings.database_url.clone());
        tokio::pin!(connection);
        let attempt = tokio::select! {
            result = &mut connection => result,
            result = &mut *health_task => return flatten_task("health server", result).map(|()| None),
        };
        match attempt {
            Ok(metadata) => return Ok(Some(metadata)),
            Err(error) => {
                health.mark_dependencies_waiting();
                tracing::warn!(%error, "media worker database or schema is unavailable");
            }
        }
        tokio::select! {
            result = &mut *health_task => return flatten_task("health server", result).map(|()| None),
            _ = tokio::time::sleep(settings.poll_interval.max(Duration::from_secs(1))) => {},
        }
    }
}

fn flatten_task(
    name: &str,
    result: Result<anyhow::Result<()>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    result
        .with_context(|| format!("{name} task failed"))?
        .with_context(|| format!("{name} failed"))
}

async fn codec_info() -> anyhow::Result<()> {
    let work = tempfile::tempdir()?;
    let pipeline = CodecPipeline::new(CodecPrograms::from_env(), CodecLimits::default());
    let capabilities = pipeline.verify_capabilities(work.path()).await?;
    println!("{}", serde_json::to_string_pretty(&capabilities)?);
    Ok(())
}

async fn codec_self_test(fixture_dir: &Path) -> anyhow::Result<()> {
    let cases = [
        ("fixture.png", MediaKind::Image, false),
        ("fixture.jpg", MediaKind::Image, false),
        ("fixture.webp", MediaKind::Image, false),
        ("fixture.gif", MediaKind::Image, false),
        ("fixture.heic", MediaKind::Image, false),
        ("fixture-oriented.jpg", MediaKind::Image, true),
        ("fixture.mp4", MediaKind::Video, false),
        ("fixture-1080p.mp4", MediaKind::Video, false),
        ("fixture.mov", MediaKind::Video, false),
        ("fixture.webm", MediaKind::Video, false),
        ("fixture-rotated.mov", MediaKind::Video, true),
        ("fixture-hdr.mp4", MediaKind::Video, false),
    ];
    let output_root = fixture_dir.join("processed");
    if output_root.exists() {
        std::fs::remove_dir_all(&output_root)?;
    }
    std::fs::create_dir_all(&output_root)?;
    let pipeline = CodecPipeline::new(CodecPrograms::from_env(), CodecLimits::default());
    let capabilities = pipeline.verify_capabilities(&output_root).await?;

    for &(name, kind, rotated) in &cases {
        let source = fixture_dir.join(name);
        let work = output_root.join(name);
        std::fs::create_dir(&work)?;
        let output = pipeline
            .process(&source, &work)
            .await
            .map_err(anyhow::Error::new)
            .with_context(|| format!("processing synthetic fixture {name}"))?;
        assert_fixture(name, &output, kind, rotated)?;
    }
    for name in ["corrupt.jpg", "spoof.jpg"] {
        let source = fixture_dir.join(name);
        let work = output_root.join(name);
        std::fs::create_dir(&work)?;
        if pipeline.process(&source, &work).await.is_ok() {
            anyhow::bail!("invalid synthetic fixture {name} was accepted");
        }
    }
    let oversized = fixture_dir.join("fixture-oversize.png");
    let oversized_work = output_root.join("fixture-oversize.png");
    std::fs::create_dir(&oversized_work)?;
    match pipeline.process(&oversized, &oversized_work).await {
        Err(error)
            if error.kind == scope_media_worker::codec::CodecFailureKind::MediaLimitExceeded => {}
        Ok(_) => anyhow::bail!("oversized synthetic fixture was accepted"),
        Err(error) => anyhow::bail!("oversized synthetic fixture failed incorrectly: {error}"),
    }
    println!(
        "{}",
        serde_json::json!({
            "capabilities": capabilities,
            "fixtures": cases.len(),
            "invalid_fixtures": 3,
            "output": output_root,
        })
    );
    Ok(())
}

fn assert_fixture(
    name: &str,
    output: &CodecOutput,
    expected_kind: MediaKind,
    rotated: bool,
) -> anyhow::Result<()> {
    if output.source.kind != expected_kind {
        anyhow::bail!("{name} was classified as the wrong media kind");
    }
    match expected_kind {
        MediaKind::Image => {
            let preview = output.derivatives.as_slice();
            if preview.len() != 1 || preview[0].kind != DerivativeKind::ImagePreview {
                anyhow::bail!("{name} did not produce exactly one image preview");
            }
            let expected_type = if name.ends_with(".gif") {
                "image/gif"
            } else {
                "image/webp"
            };
            if preview[0].media_type != expected_type {
                anyhow::bail!("{name} preview has the wrong media type");
            }
            if rotated && output.source.width >= output.source.height {
                anyhow::bail!("{name} did not honor its EXIF orientation");
            }
        }
        MediaKind::Video => {
            if output.derivatives.len() != 2
                || output.derivatives[0].kind != DerivativeKind::VideoPlayback
                || output.derivatives[1].kind != DerivativeKind::VideoPoster
            {
                anyhow::bail!("{name} did not produce playback and poster derivatives");
            }
            if rotated && output.source.width >= output.source.height {
                anyhow::bail!("{name} did not honor its display rotation");
            }
        }
    }
    Ok(())
}
