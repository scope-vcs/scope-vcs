mod cleanup;
mod codec;
mod config;
mod health;
mod jobs;
mod lease;
mod process;
mod runtime;
mod scratch;

use anyhow::Context as _;
use codec::{CodecOutput, CodecPipeline, PlaybackConversion};
use config::{CodecLimits, CodecPrograms, WorkerSettings};
use health::WorkerHealth;
use scope_domain::requests::attachments::{
    RequestAttachmentDerivativeKind as DerivativeKind, RequestAttachmentKind as MediaKind,
    RequestAttachmentProcessingLane as ProcessingLane,
};
use scope_postgres::db::MetadataStore;
use scratch::ScratchSpace;
use std::{path::Path, time::Duration};

fn main() -> anyhow::Result<()> {
    let telemetry = scope_service_runtime::init_telemetry(
        "media-worker",
        "scope_media_worker=info,scope_postgres=info",
    )?;
    let result = run_command();
    telemetry.shutdown();
    result
}

#[tokio::main]
async fn run_command() -> anyhow::Result<()> {
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

    let storage = scope_media_storage::MediaStorageSettings::from_env()?
        .connect(4)
        .await
        .context("configuring encrypted media storage")?;
    let health = WorkerHealth::new(settings.poll_interval);
    health.mark_codecs_ready();
    let mut health_task = tokio::spawn(health.clone().serve(settings.health_port));
    let Some(metadata) = connect_metadata(&settings, &health, &mut health_task).await? else {
        return Ok(());
    };

    let processing_task = |lane| {
        tokio::spawn(jobs::run_processing_loop(
            lane,
            metadata.clone(),
            storage.clone(),
            pipeline.clone(),
            scratch.clone(),
            settings.clone(),
            health.clone(),
        ))
    };
    let mut photo_task = processing_task(ProcessingLane::Photo);
    let mut general_task = processing_task(ProcessingLane::General);
    let mut cleanup_task = tokio::spawn(cleanup::run(metadata, storage, settings, health));

    let result = tokio::select! {
        result = &mut health_task => flatten_task("health server", result),
        result = &mut photo_task => flatten_task("photo processing loop", result),
        result = &mut general_task => flatten_task("processing loop", result),
        result = &mut cleanup_task => flatten_task("cleanup loop", result),
    };
    health_task.abort();
    photo_task.abort();
    general_task.abort();
    cleanup_task.abort();
    result
}

async fn connect_metadata(
    settings: &WorkerSettings,
    health: &WorkerHealth,
    health_task: &mut tokio::task::JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<Option<MetadataStore>> {
    loop {
        let connection = MetadataStore::connect(settings.database_url.clone());
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
    let remux = Some(PlaybackConversion::Remux);
    let transcode = Some(PlaybackConversion::Transcode);
    let cases = [
        ("fixture.png", MediaKind::Photo, false, None),
        ("fixture.jpg", MediaKind::Photo, false, None),
        ("fixture.webp", MediaKind::Photo, false, None),
        ("fixture.gif", MediaKind::Photo, false, None),
        ("fixture.heic", MediaKind::Photo, false, None),
        ("fixture-oriented.jpg", MediaKind::Photo, true, None),
        ("fixture.mp4", MediaKind::Video, false, remux),
        ("fixture-1080p.mp4", MediaKind::Video, false, transcode),
        ("fixture.mov", MediaKind::Video, false, remux),
        ("fixture.webm", MediaKind::Video, false, transcode),
        ("fixture-rotated.mov", MediaKind::Video, true, transcode),
        ("fixture-hdr.mp4", MediaKind::Video, false, transcode),
    ];
    let output_root = fixture_dir.join("processed");
    if output_root.exists() {
        std::fs::remove_dir_all(&output_root)?;
    }
    std::fs::create_dir_all(&output_root)?;
    let pipeline = CodecPipeline::new(CodecPrograms::from_env(), CodecLimits::default());
    let capabilities = pipeline.verify_capabilities(&output_root).await?;

    for &(name, kind, rotated, playback) in &cases {
        let source = fixture_dir.join(name);
        let work = output_root.join(name);
        std::fs::create_dir(&work)?;
        let output = pipeline
            .process(&source, &work, kind)
            .await
            .map_err(anyhow::Error::new)
            .with_context(|| format!("processing synthetic fixture {name}"))?;
        assert_fixture(name, &output, kind, rotated)?;
        if output.playback != playback {
            anyhow::bail!("{name} used {:?} instead of {playback:?}", output.playback);
        }
    }
    for name in ["corrupt.jpg", "spoof.jpg"] {
        let source = fixture_dir.join(name);
        let work = output_root.join(name);
        std::fs::create_dir(&work)?;
        if pipeline
            .process(&source, &work, MediaKind::Photo)
            .await
            .is_ok()
        {
            anyhow::bail!("invalid synthetic fixture {name} was accepted");
        }
    }
    let rejections = [
        (
            "fixture-oversize.png",
            MediaKind::Photo,
            codec::CodecFailureKind::MediaLimitExceeded,
        ),
        (
            "fixture.mp4",
            MediaKind::Photo,
            codec::CodecFailureKind::DeclaredKindMismatch,
        ),
    ];
    for (name, declared_kind, expected) in rejections {
        let work = output_root.join(format!("rejected-{name}"));
        std::fs::create_dir(&work)?;
        match pipeline
            .process(&fixture_dir.join(name), &work, declared_kind)
            .await
        {
            Err(error) if error.kind == expected => {}
            Ok(_) => anyhow::bail!("{name} declared as {declared_kind:?} was accepted"),
            Err(error) => anyhow::bail!("{name} failed incorrectly: {error}"),
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "capabilities": capabilities,
            "fixtures": cases.len(),
            "invalid_fixtures": 4,
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
        MediaKind::Photo => {
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
