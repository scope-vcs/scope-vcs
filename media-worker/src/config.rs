use scope_domain::requests::attachments::RequestAttachmentLimits;
use std::{path::PathBuf, time::Duration};

pub const DEFAULT_HEALTH_PORT: u16 = 8080;

#[derive(Clone, Debug)]
pub struct CodecLimits {
    pub max_source_bytes: u64,
    pub max_image_source_bytes: u64,
    pub max_video_source_bytes: u64,
    pub max_image_pixels: u64,
    pub max_video_pixels: u64,
    pub max_video_duration: Duration,
    pub max_derivative_bytes: u64,
    pub max_process_memory_bytes: u64,
    pub process_timeout: Duration,
    pub max_process_output_bytes: usize,
    pub process_threads: u16,
}

impl Default for CodecLimits {
    fn default() -> Self {
        let attachment_limits = RequestAttachmentLimits::default();
        Self {
            max_source_bytes: attachment_limits
                .max_photo_bytes
                .max(attachment_limits.max_video_bytes),
            max_image_source_bytes: attachment_limits.max_photo_bytes,
            max_video_source_bytes: attachment_limits.max_video_bytes,
            max_image_pixels: attachment_limits.max_photo_pixels,
            max_video_pixels: 35_389_440,
            max_video_duration: Duration::from_secs(attachment_limits.max_video_duration_seconds),
            max_derivative_bytes: 500 * 1024 * 1024,
            max_process_memory_bytes: 1536 * 1024 * 1024,
            process_timeout: Duration::from_secs(15 * 60),
            max_process_output_bytes: 1024 * 1024,
            process_threads: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CodecPrograms {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub heif_convert: PathBuf,
    pub heif_info: PathBuf,
}

impl Default for CodecPrograms {
    fn default() -> Self {
        Self {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
            heif_convert: PathBuf::from("heif-convert"),
            heif_info: PathBuf::from("heif-info"),
        }
    }
}

impl CodecPrograms {
    pub fn from_env() -> Self {
        Self {
            ffmpeg: env_path("SCOPE_MEDIA_FFMPEG", "ffmpeg"),
            ffprobe: env_path("SCOPE_MEDIA_FFPROBE", "ffprobe"),
            heif_convert: env_path("SCOPE_MEDIA_HEIF_CONVERT", "heif-convert"),
            heif_info: env_path("SCOPE_MEDIA_HEIF_INFO", "heif-info"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkerSettings {
    pub database_url: String,
    pub worker_id: String,
    pub health_port: u16,
    pub scratch_root: PathBuf,
    pub poll_interval: Duration,
    pub lease_duration: Duration,
    pub max_attempts: u32,
    pub codec_programs: CodecPrograms,
    pub codec_limits: CodecLimits,
}

impl WorkerSettings {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = required_env("DATABASE_URL")?;
        let worker_id = std::env::var("SCOPE_MEDIA_WORKER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("media-worker-{}", std::process::id()));
        let scratch_root = env_path("SCOPE_MEDIA_SCRATCH_DIR", "/tmp/scope-media-worker");
        let health_port = env_parse("PORT", DEFAULT_HEALTH_PORT)?;
        let poll_interval =
            Duration::from_millis(env_parse("SCOPE_MEDIA_POLL_INTERVAL_MS", 1_000_u64)?);
        let lease_duration = Duration::from_secs(env_parse("SCOPE_MEDIA_LEASE_SECONDS", 120_u64)?);
        if lease_duration < Duration::from_secs(15) {
            anyhow::bail!("SCOPE_MEDIA_LEASE_SECONDS must be at least 15");
        }

        let codec_limits = CodecLimits {
            max_process_memory_bytes: mib(env_parse("SCOPE_MEDIA_PROCESS_MEMORY_MIB", 1536_u64)?)?,
            process_timeout: Duration::from_secs(env_parse(
                "SCOPE_MEDIA_PROCESS_TIMEOUT_SECONDS",
                15 * 60_u64,
            )?),
            process_threads: env_parse("SCOPE_MEDIA_PROCESS_THREADS", 2_u16)?,
            ..CodecLimits::default()
        };
        if codec_limits.process_threads == 0 || codec_limits.process_threads > 8 {
            anyhow::bail!("SCOPE_MEDIA_PROCESS_THREADS must be between 1 and 8");
        }

        Ok(Self {
            database_url,
            worker_id,
            health_port,
            scratch_root,
            poll_interval,
            lease_duration,
            max_attempts: env_parse("SCOPE_MEDIA_MAX_ATTEMPTS", 4_u32)?,
            codec_programs: CodecPrograms::from_env(),
            codec_limits,
        })
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} must be set"))
}

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn env_parse<T>(name: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid {name}: {error}")),
        Err(_) => Ok(default),
    }
}

fn mib(value: u64) -> anyhow::Result<u64> {
    value
        .checked_mul(1024 * 1024)
        .ok_or_else(|| anyhow::anyhow!("media process memory limit is too large"))
}
