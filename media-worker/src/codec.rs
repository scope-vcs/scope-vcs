use crate::{
    config::{CodecLimits, CodecPrograms},
    process::{CodecProcessLimits, ProcessFailure, os_args, path_arg, run_bounded},
};
use scope_domain::requests::attachments::{RequestAttachmentDerivativeKind, RequestAttachmentKind};
use serde::Serialize;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

mod inspection;
mod video;
use inspection::{
    Probe, dimensions_from_heif_info, media_type, reject_webp_metadata, single_video_stream,
    sniff_format, validate_pixels,
};
pub use video::PlaybackConversion;

const IMAGE_MAX_EDGE: u32 = 2_048;
const PROCESS_COUNT_BACKSTOP: u64 = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
    Heic,
    Mp4,
    Mov,
    Webm,
}

impl InputFormat {
    fn kind(self) -> RequestAttachmentKind {
        match self {
            Self::Png | Self::Jpeg | Self::Webp | Self::Gif | Self::Heic => {
                RequestAttachmentKind::Photo
            }
            Self::Mp4 | Self::Mov | Self::Webm => RequestAttachmentKind::Video,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSource {
    pub kind: RequestAttachmentKind,
    pub media_type: &'static str,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub duration_millis: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecDerivative {
    pub kind: RequestAttachmentDerivativeKind,
    pub path: PathBuf,
    pub media_type: &'static str,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub duration_millis: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecOutput {
    pub source: ValidatedSource,
    pub derivatives: Vec<CodecDerivative>,
    pub playback: Option<PlaybackConversion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecFailureKind {
    InvalidMedia,
    CorruptMedia,
    UnsupportedMedia,
    MediaLimitExceeded,
    DeclaredKindMismatch,
    TimedOut,
    CodecFailed,
    Internal,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct CodecFailure {
    pub kind: CodecFailureKind,
    pub message: String,
    pub validated_source: Option<ValidatedSource>,
}

impl CodecFailure {
    fn new(kind: CodecFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            validated_source: None,
        }
    }

    fn after_validation(mut self, source: ValidatedSource) -> Self {
        self.validated_source = Some(source);
        self
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self.kind,
            CodecFailureKind::CodecFailed | CodecFailureKind::Internal
        )
    }
}

#[derive(Clone)]
pub struct CodecPipeline {
    programs: CodecPrograms,
    limits: CodecLimits,
}

impl CodecPipeline {
    pub fn new(programs: CodecPrograms, limits: CodecLimits) -> Self {
        Self { programs, limits }
    }

    pub async fn verify_capabilities(&self, cwd: &Path) -> anyhow::Result<CodecCapabilities> {
        let probe_version = self
            .tool_output(&self.programs.ffprobe, &os_args(["-version"]), cwd)
            .await?;
        let ffmpeg_version = self
            .tool_output(&self.programs.ffmpeg, &os_args(["-version"]), cwd)
            .await?;
        let encoders = self
            .tool_output(
                &self.programs.ffmpeg,
                &os_args(["-hide_banner", "-encoders"]),
                cwd,
            )
            .await?;
        const REQUIRED_ENCODERS: [&str; 4] = ["libx264", "aac", "libwebp", "gif"];
        for encoder in REQUIRED_ENCODERS {
            if !encoders.split_whitespace().any(|word| word == encoder) {
                anyhow::bail!("required FFmpeg encoder is unavailable: {encoder}");
            }
        }
        let filters = self
            .tool_output(
                &self.programs.ffmpeg,
                &os_args(["-hide_banner", "-filters"]),
                cwd,
            )
            .await?;
        for filter in ["zscale", "tonemap"] {
            if !filters.split_whitespace().any(|word| word == filter) {
                anyhow::bail!("required FFmpeg filter is unavailable: {filter}");
            }
        }
        let bitstream_filters = self
            .tool_output(
                &self.programs.ffmpeg,
                &os_args(["-hide_banner", "-bsfs"]),
                cwd,
            )
            .await?;
        for filter in ["h264_metadata", "filter_units"] {
            if !bitstream_filters.lines().any(|line| line.trim() == filter) {
                anyhow::bail!("required FFmpeg bitstream filter is unavailable: {filter}");
            }
        }
        let heif_output = self
            .tool_output(
                &self.programs.heif_convert,
                &os_args(["--list-decoders"]),
                cwd,
            )
            .await?;
        if !heif_output.contains("HEIC") {
            anyhow::bail!("heif-convert has no HEIC decoder");
        }
        self.tool_output(&self.programs.heif_info, &os_args(["--version"]), cwd)
            .await?;

        Ok(CodecCapabilities {
            ffmpeg: first_line(&ffmpeg_version),
            ffprobe: first_line(&probe_version),
            heif: first_line(&heif_output),
            encoders: REQUIRED_ENCODERS.into_iter().map(str::to_owned).collect(),
        })
    }

    pub async fn process(
        &self,
        source: &Path,
        work_dir: &Path,
        declared_kind: RequestAttachmentKind,
    ) -> Result<CodecOutput, CodecFailure> {
        let size_bytes = fs::metadata(source)
            .map_err(|error| CodecFailure::new(CodecFailureKind::Internal, error.to_string()))?
            .len();
        if size_bytes == 0 {
            return Err(CodecFailure::new(
                CodecFailureKind::InvalidMedia,
                "media file is empty",
            ));
        }
        if size_bytes > self.limits.max_source_bytes {
            return Err(CodecFailure::new(
                CodecFailureKind::MediaLimitExceeded,
                format!(
                    "media source exceeds {} bytes",
                    self.limits.max_source_bytes
                ),
            ));
        }
        let format = sniff_format(source)?;
        if format.kind() != declared_kind {
            return Err(CodecFailure::new(
                CodecFailureKind::DeclaredKindMismatch,
                format!("media signature is {format:?}, not a {declared_kind:?}"),
            ));
        }
        let source_limit = match format.kind() {
            RequestAttachmentKind::Photo => self.limits.max_image_source_bytes,
            RequestAttachmentKind::Video => self.limits.max_video_source_bytes,
        };
        if size_bytes > source_limit {
            return Err(CodecFailure::new(
                CodecFailureKind::MediaLimitExceeded,
                format!("media source exceeds its {source_limit}-byte type limit"),
            ));
        }
        match format.kind() {
            RequestAttachmentKind::Photo => {
                self.process_image(source, work_dir, format, size_bytes)
                    .await
            }
            RequestAttachmentKind::Video => {
                self.process_video(source, work_dir, format, size_bytes)
                    .await
            }
        }
    }

    fn transcode_input(&self, source: &Path, strict: bool) -> Vec<OsString> {
        let mut args = self.bounded_ffmpeg_prefix();
        if strict {
            args.push("-xerror".into());
        }
        args.push("-y".into());
        args.extend(["-i".into(), path_arg(source)]);
        args
    }

    async fn process_image(
        &self,
        source: &Path,
        work_dir: &Path,
        format: InputFormat,
        size_bytes: u64,
    ) -> Result<CodecOutput, CodecFailure> {
        let decoded_heic = work_dir.join("decoded-heic.png");
        let probe_path = if format == InputFormat::Heic {
            let info = self
                .tool_output(&self.programs.heif_info, &[path_arg(source)], work_dir)
                .await
                .map_err(|error| {
                    CodecFailure::new(CodecFailureKind::CorruptMedia, error.to_string())
                })?;
            let (heic_width, heic_height) = dimensions_from_heif_info(&info)?;
            validate_pixels(
                heic_width,
                heic_height,
                self.limits.max_image_pixels,
                "photo",
            )?;
            self.run_codec(
                &self.programs.heif_convert,
                &[
                    OsString::from("--quiet"),
                    path_arg(source),
                    path_arg(&decoded_heic),
                ],
                work_dir,
            )
            .await
            .map_err(|error| codec_process_failure(error, CodecFailureKind::CorruptMedia))?;
            &decoded_heic
        } else {
            source
        };
        let probe = self.probe(probe_path, work_dir).await?;
        let stream = single_video_stream(&probe)?;
        let (bounded_width, bounded_height) = probe.display_dimensions(stream);
        validate_pixels(
            bounded_width,
            bounded_height,
            self.limits.max_image_pixels,
            "photo",
        )?;
        let frame_probe = self.probe_first_frame(probe_path, work_dir).await?;
        let (display_width, display_height) = frame_probe.display_dimensions(stream);
        self.validate_full_decode(probe_path, work_dir, false)
            .await?;
        let validated_source = ValidatedSource {
            kind: RequestAttachmentKind::Photo,
            media_type: media_type(format),
            size_bytes,
            width: display_width,
            height: display_height,
            duration_millis: None,
        };

        let animated_gif = format == InputFormat::Gif;
        let preview = work_dir.join(if animated_gif {
            "image-preview.gif"
        } else {
            "image-preview.webp"
        });
        let filter = format!(
            "scale=w='min({IMAGE_MAX_EDGE},iw)':h='min({IMAGE_MAX_EDGE},ih)':force_original_aspect_ratio=decrease,setsar=1"
        );
        let mut args = self.transcode_input(probe_path, true);
        args.extend([
            "-map".into(),
            "0:v:0".into(),
            "-vf".into(),
            filter.into(),
            "-an".into(),
            "-sn".into(),
            "-dn".into(),
            "-map_metadata".into(),
            "-1".into(),
            "-map_chapters".into(),
            "-1".into(),
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
        ]);
        if animated_gif {
            args.extend(os_args([
                "-c:v",
                "gif",
                "-fps_mode",
                "passthrough",
                "-loop",
                "0",
            ]));
        } else {
            args.extend(os_args([
                "-frames:v",
                "1",
                "-c:v",
                "libwebp",
                "-quality",
                "82",
                "-compression_level",
                "4",
            ]));
        }
        args.push(path_arg(&preview));
        self.run_codec(&self.programs.ffmpeg, &args, work_dir)
            .await
            .map_err(|error| {
                codec_process_failure(error, CodecFailureKind::CodecFailed)
                    .after_validation(validated_source.clone())
            })?;
        if !animated_gif {
            reject_webp_metadata(&preview)
                .map_err(|error| error.after_validation(validated_source.clone()))?;
        }
        let derivative = self
            .derivative(
                RequestAttachmentDerivativeKind::ImagePreview,
                preview,
                if animated_gif {
                    "image/gif"
                } else {
                    "image/webp"
                },
                None,
                work_dir,
            )
            .await
            .map_err(|error| error.after_validation(validated_source.clone()))?;
        Ok(CodecOutput {
            source: validated_source,
            derivatives: vec![derivative],
            playback: None,
        })
    }

    async fn derivative(
        &self,
        kind: RequestAttachmentDerivativeKind,
        path: PathBuf,
        media_type: &'static str,
        duration_millis: Option<u64>,
        work_dir: &Path,
    ) -> Result<CodecDerivative, CodecFailure> {
        let probe = self.probe(&path, work_dir).await?;
        let stream = single_video_stream(&probe)?;
        self.file_derivative(
            kind,
            path,
            media_type,
            stream.width,
            stream.height,
            duration_millis,
        )
    }

    fn file_derivative(
        &self,
        kind: RequestAttachmentDerivativeKind,
        path: PathBuf,
        media_type: &'static str,
        width: u32,
        height: u32,
        duration_millis: Option<u64>,
    ) -> Result<CodecDerivative, CodecFailure> {
        let size_bytes = fs::metadata(&path)
            .map_err(|error| CodecFailure::new(CodecFailureKind::Internal, error.to_string()))?
            .len();
        if size_bytes == 0 || size_bytes > self.limits.max_derivative_bytes {
            return Err(CodecFailure::new(
                CodecFailureKind::CodecFailed,
                "codec produced an empty or oversized derivative",
            ));
        }
        Ok(CodecDerivative {
            kind,
            path,
            media_type,
            size_bytes,
            width,
            height,
            duration_millis,
        })
    }

    async fn probe(&self, path: &Path, cwd: &Path) -> Result<Probe, CodecFailure> {
        let args = vec![
            "-v".into(),
            "error".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-show_entries".into(),
            "format=format_name,duration:stream=index,codec_type,codec_name,profile,level,width,height,duration,pix_fmt,color_range,color_transfer,color_primaries,color_space:stream_tags=rotate:stream_side_data=rotation"
                .into(),
            "-of".into(),
            "json".into(),
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            path_arg(path),
        ];
        let output = self
            .run_codec(&self.programs.ffprobe, &args, cwd)
            .await
            .map_err(|error| codec_process_failure(error, CodecFailureKind::CorruptMedia))?;
        serde_json::from_slice(&output.stdout).map_err(|error| {
            CodecFailure::new(
                CodecFailureKind::CodecFailed,
                format!("ffprobe returned invalid JSON: {error}"),
            )
        })
    }

    async fn probe_first_frame(&self, path: &Path, cwd: &Path) -> Result<Probe, CodecFailure> {
        let args = vec![
            "-v".into(),
            "error".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-select_streams".into(),
            "v:0".into(),
            "-read_intervals".into(),
            "%+#1".into(),
            "-show_frames".into(),
            "-show_entries".into(),
            "frame=width,height:frame_side_data=rotation".into(),
            "-of".into(),
            "json".into(),
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            path_arg(path),
        ];
        let output = self
            .run_codec(&self.programs.ffprobe, &args, cwd)
            .await
            .map_err(|error| codec_process_failure(error, CodecFailureKind::CorruptMedia))?;
        serde_json::from_slice(&output.stdout).map_err(|error| {
            CodecFailure::new(
                CodecFailureKind::CodecFailed,
                format!("ffprobe returned invalid frame JSON: {error}"),
            )
        })
    }

    async fn validate_full_decode(
        &self,
        path: &Path,
        cwd: &Path,
        include_audio: bool,
    ) -> Result<(), CodecFailure> {
        let mut args = self.bounded_ffmpeg_prefix();
        args.extend(os_args(["-xerror"]));
        args.extend(vec![
            "-i".into(),
            path_arg(path),
            "-map".into(),
            "0:v:0".into(),
        ]);
        if include_audio {
            args.extend(os_args(["-map", "0:a?"]));
        } else {
            args.push("-an".into());
        }
        args.extend(vec![
            "-sn".into(),
            "-dn".into(),
            "-f".into(),
            "null".into(),
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            "-".into(),
        ]);
        self.run_codec(&self.programs.ffmpeg, &args, cwd)
            .await
            .map_err(|error| codec_process_failure(error, CodecFailureKind::CorruptMedia))?;
        Ok(())
    }

    async fn run_codec(
        &self,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<crate::process::ProcessOutput, ProcessFailure> {
        run_bounded(program, args, cwd, &self.process_limits()).await
    }

    async fn tool_output(
        &self,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
    ) -> anyhow::Result<String> {
        let output = self.run_codec(program, args, cwd).await?;
        let mut combined = output.stdout;
        combined.extend_from_slice(&output.stderr);
        Ok(String::from_utf8_lossy(&combined).into_owned())
    }

    fn bounded_ffmpeg_prefix(&self) -> Vec<std::ffi::OsString> {
        let mut args = os_args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file,pipe",
            "-filter_threads",
        ]);
        args.push(self.limits.process_threads.to_string().into());
        args.push("-threads".into());
        args.push(self.limits.process_threads.to_string().into());
        args
    }

    fn process_limits(&self) -> CodecProcessLimits {
        CodecProcessLimits {
            timeout: self.limits.process_timeout,
            memory_bytes: self.limits.max_process_memory_bytes,
            output_file_bytes: self.limits.max_derivative_bytes,
            captured_output_bytes: self.limits.max_process_output_bytes,
            open_files: 64,
            processes: PROCESS_COUNT_BACKSTOP,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CodecCapabilities {
    pub ffmpeg: String,
    pub ffprobe: String,
    pub heif: String,
    pub encoders: Vec<String>,
}

fn codec_process_failure(error: ProcessFailure, default_kind: CodecFailureKind) -> CodecFailure {
    let kind = match error {
        ProcessFailure::Timeout { .. } => CodecFailureKind::TimedOut,
        ProcessFailure::Start { .. } | ProcessFailure::Output { .. } => CodecFailureKind::Internal,
        ProcessFailure::Exit { .. } => default_kind,
    };
    CodecFailure::new(kind, error.to_string())
}

fn first_line(output: &str) -> String {
    output.lines().next().unwrap_or_default().trim().to_owned()
}
