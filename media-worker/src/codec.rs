use crate::{
    config::{CodecLimits, CodecPrograms},
    process::{ProcessFailure, ProcessLimits, os_args, path_arg, run_bounded},
};
use serde::Serialize;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

mod inspection;
use inspection::{
    Probe, dimensions_from_heif_info, duration_millis, media_type, reject_webp_metadata,
    single_video_stream, sniff_format, validate_pixels, verify_faststart,
};

const IMAGE_MAX_EDGE: u32 = 2_048;
const VIDEO_MAX_WIDTH: u32 = 1_920;
const VIDEO_MAX_HEIGHT: u32 = 1_080;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSource {
    pub kind: MediaKind,
    pub media_type: &'static str,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub duration_millis: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DerivativeKind {
    ImagePreview,
    VideoPlayback,
    VideoPoster,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecDerivative {
    pub kind: DerivativeKind,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecFailureKind {
    InvalidMedia,
    CorruptMedia,
    UnsupportedMedia,
    MediaLimitExceeded,
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
        for encoder in ["libx264", "aac", "libwebp", "gif"] {
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
        let heif_output = self
            .tool_output(
                &self.programs.heif_convert,
                &os_args(["--list-decoders"]),
                cwd,
            )
            .await?;
        if !heif_output.contains("HEIC") || !heif_output.contains("-") {
            anyhow::bail!("heif-convert has no HEIC decoder");
        }
        self.tool_output(&self.programs.heif_info, &os_args(["--version"]), cwd)
            .await?;

        Ok(CodecCapabilities {
            ffmpeg: first_line(&ffmpeg_version),
            ffprobe: first_line(&probe_version),
            heif: first_line(&heif_output),
            encoders: vec![
                "libx264".into(),
                "aac".into(),
                "libwebp".into(),
                "gif".into(),
            ],
        })
    }

    pub async fn process(
        &self,
        source: &Path,
        work_dir: &Path,
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
        let source_limit = match format {
            InputFormat::Png
            | InputFormat::Jpeg
            | InputFormat::Webp
            | InputFormat::Gif
            | InputFormat::Heic => self.limits.max_image_source_bytes,
            InputFormat::Mp4 | InputFormat::Mov | InputFormat::Webm => {
                self.limits.max_video_source_bytes
            }
        };
        if size_bytes > source_limit {
            return Err(CodecFailure::new(
                CodecFailureKind::MediaLimitExceeded,
                format!("media source exceeds its {source_limit}-byte type limit"),
            ));
        }
        match format {
            InputFormat::Png
            | InputFormat::Jpeg
            | InputFormat::Webp
            | InputFormat::Gif
            | InputFormat::Heic => {
                self.process_image(source, work_dir, format, size_bytes)
                    .await
            }
            InputFormat::Mp4 | InputFormat::Mov | InputFormat::Webm => {
                self.process_video(source, work_dir, format, size_bytes)
                    .await
            }
        }
    }

    /// Keep decoder isolation and input thread limits identical for each transcode.
    fn transcode_input(&self, source: &Path, strict: bool) -> Vec<OsString> {
        let mut args = os_args(["-hide_banner", "-loglevel", "error"]);
        if strict {
            args.push("-xerror".into());
        }
        args.extend(os_args([
            "-nostdin",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
        ]));
        args.extend([
            "-filter_threads".into(),
            self.limits.process_threads.to_string().into(),
            // Input and output thread limits are separate FFmpeg options.
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            "-i".into(),
            path_arg(source),
        ]);
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
            kind: MediaKind::Image,
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
                DerivativeKind::ImagePreview,
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
        })
    }

    async fn process_video(
        &self,
        source: &Path,
        work_dir: &Path,
        format: InputFormat,
        size_bytes: u64,
    ) -> Result<CodecOutput, CodecFailure> {
        let source_probe = self.probe(source, work_dir).await?;
        let source_stream = single_video_stream(&source_probe)?;
        let (display_width, display_height) = source_probe.display_dimensions(source_stream);
        validate_pixels(
            display_width,
            display_height,
            self.limits.max_video_pixels,
            "video",
        )?;
        let source_duration_millis = duration_millis(&source_probe, source_stream)?;
        if source_duration_millis > self.limits.max_video_duration.as_millis() as u64 {
            return Err(CodecFailure::new(
                CodecFailureKind::MediaLimitExceeded,
                format!(
                    "video duration exceeds {} seconds",
                    self.limits.max_video_duration.as_secs()
                ),
            ));
        }
        self.validate_full_decode(source, work_dir, true).await?;
        let validated_source = ValidatedSource {
            kind: MediaKind::Video,
            media_type: media_type(format),
            size_bytes,
            width: display_width,
            height: display_height,
            duration_millis: Some(source_duration_millis),
        };

        let playback = work_dir.join("video-playback.mp4");
        let scale = format!(
            "scale=w='min({VIDEO_MAX_WIDTH},iw)':h='min({VIDEO_MAX_HEIGHT},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
        );
        let filter = if source_stream.is_hdr() {
            format!(
                "{scale},zscale=transfer=linear:npl=100,format=gbrpf32le,zscale=primaries=bt709,tonemap=tonemap=hable:desat=0,zscale=transfer=bt709:matrix=bt709:range=tv,format=yuv420p,setsar=1"
            )
        } else {
            format!("{scale},format=yuv420p,setsar=1")
        };
        let mut args = self.transcode_input(source, false);
        args.extend([
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a:0?".into(),
            "-vf".into(),
            filter.into(),
            "-sn".into(),
            "-dn".into(),
            "-map_metadata".into(),
            "-1".into(),
            "-map_chapters".into(),
            "-1".into(),
            "-metadata:s:v:0".into(),
            "rotate=0".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "medium".into(),
            "-crf".into(),
            "23".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-color_primaries".into(),
            "bt709".into(),
            "-color_trc".into(),
            "bt709".into(),
            "-colorspace".into(),
            "bt709".into(),
            "-color_range".into(),
            "tv".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-ac".into(),
            "2".into(),
            "-movflags".into(),
            "+faststart".into(),
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            path_arg(&playback),
        ]);
        self.run_codec(&self.programs.ffmpeg, &args, work_dir)
            .await
            .map_err(|error| {
                codec_process_failure(error, CodecFailureKind::CodecFailed)
                    .after_validation(validated_source.clone())
            })?;
        verify_faststart(&playback)
            .map_err(|error| error.after_validation(validated_source.clone()))?;
        let playback_probe = self
            .probe(&playback, work_dir)
            .await
            .map_err(|error| error.after_validation(validated_source.clone()))?;
        let playback_stream = single_video_stream(&playback_probe)?;
        if playback_stream.codec_name.as_deref() != Some("h264") {
            return Err(CodecFailure::new(
                CodecFailureKind::CodecFailed,
                "video derivative is not H.264",
            )
            .after_validation(validated_source));
        }
        if playback_probe.streams.iter().any(|stream| {
            stream.codec_type.as_deref() == Some("audio")
                && stream.codec_name.as_deref() != Some("aac")
        }) {
            return Err(CodecFailure::new(
                CodecFailureKind::CodecFailed,
                "video derivative audio is not AAC",
            )
            .after_validation(validated_source));
        }
        let playback_duration = duration_millis(&playback_probe, playback_stream)
            .map_err(|error| error.after_validation(validated_source.clone()))?;
        let playback_derivative = self
            .file_derivative(
                DerivativeKind::VideoPlayback,
                playback,
                "video/mp4",
                playback_stream.width,
                playback_stream.height,
                Some(playback_duration),
            )
            .map_err(|error| error.after_validation(validated_source.clone()))?;

        let poster = work_dir.join("video-poster.webp");
        let mut poster_args = self.transcode_input(&playback_derivative.path, false);
        poster_args.extend([
            "-map".into(),
            "0:v:0".into(),
            "-frames:v".into(),
            "1".into(),
            "-an".into(),
            "-sn".into(),
            "-dn".into(),
            "-map_metadata".into(),
            "-1".into(),
            "-map_chapters".into(),
            "-1".into(),
            "-c:v".into(),
            "libwebp".into(),
            "-quality".into(),
            "82".into(),
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            path_arg(&poster),
        ]);
        self.run_codec(&self.programs.ffmpeg, &poster_args, work_dir)
            .await
            .map_err(|error| {
                codec_process_failure(error, CodecFailureKind::CodecFailed)
                    .after_validation(validated_source.clone())
            })?;
        reject_webp_metadata(&poster)
            .map_err(|error| error.after_validation(validated_source.clone()))?;
        let poster_derivative = self
            .derivative(
                DerivativeKind::VideoPoster,
                poster,
                "image/webp",
                None,
                work_dir,
            )
            .await
            .map_err(|error| error.after_validation(validated_source.clone()))?;

        Ok(CodecOutput {
            source: validated_source,
            derivatives: vec![playback_derivative, poster_derivative],
        })
    }

    async fn derivative(
        &self,
        kind: DerivativeKind,
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
        kind: DerivativeKind,
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
            "format=format_name,duration:stream=index,codec_type,codec_name,width,height,duration,color_transfer,color_primaries,color_space:stream_tags=rotate:stream_side_data=rotation"
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
        let mut args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-xerror".into(),
            "-nostdin".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-filter_threads".into(),
            self.limits.process_threads.to_string().into(),
            // Input and output thread limits are separate FFmpeg options.
            "-threads".into(),
            self.limits.process_threads.to_string().into(),
            "-i".into(),
            path_arg(path),
            "-map".into(),
            "0:v:0".into(),
        ];
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

    fn process_limits(&self) -> ProcessLimits {
        ProcessLimits {
            timeout: self.limits.process_timeout,
            memory_bytes: self.limits.max_process_memory_bytes,
            output_file_bytes: self.limits.max_derivative_bytes,
            captured_output_bytes: self.limits.max_process_output_bytes,
            cpu_seconds: self.limits.process_timeout.as_secs().saturating_add(1),
            open_files: 64,
            // RLIMIT_NPROC is charged to every thread owned by the container UID.
            // The worker launches one bounded child at a time and also constrains
            // FFmpeg's codec/filter threads, so this remains a backstop rather
            // than a normal concurrency control.
            processes: 4_096,
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
        ProcessFailure::Timeout { .. } => CodecFailureKind::MediaLimitExceeded,
        ProcessFailure::Start { .. } | ProcessFailure::Output { .. } => CodecFailureKind::Internal,
        ProcessFailure::Exit { .. } => default_kind,
    };
    CodecFailure::new(kind, error.to_string())
}

fn first_line(output: &str) -> String {
    output.lines().next().unwrap_or_default().trim().to_owned()
}
