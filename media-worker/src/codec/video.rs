use super::{
    CodecFailure, CodecFailureKind, CodecOutput, CodecPipeline, InputFormat, ValidatedSource,
    codec_process_failure,
    inspection::{
        Probe, ProbeStream, duration_millis, media_type, reject_webp_metadata, single_video_stream,
        validate_pixels, verify_faststart,
    },
};
use crate::process::{os_args, path_arg};
use scope_domain::requests::attachments::{RequestAttachmentDerivativeKind, RequestAttachmentKind};
use std::{ffi::OsString, path::Path};

const VIDEO_MAX_WIDTH: u32 = 1_920;
const VIDEO_MAX_HEIGHT: u32 = 1_080;
const REMUX_MAX_BITS_PER_SECOND: u64 = 12_000_000;
const REMUX_PROFILES: [&str; 4] = ["Constrained Baseline", "Baseline", "Main", "High"];
const REMUX_MAX_LEVEL: i32 = 42;
const REMUX_BITSTREAM_FILTERS: &str = "h264_metadata=colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1:video_full_range_flag=0,filter_units=remove_types=6";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackConversion {
    Remux,
    Transcode,
}

impl PlaybackConversion {
    fn choose(
        format: InputFormat,
        probe: &Probe,
        stream: &ProbeStream,
        size_bytes: u64,
        duration_millis: u64,
    ) -> Self {
        let (width, height) = probe.display_dimensions(stream);
        let audio_copyable = probe
            .streams
            .iter()
            .find(|candidate| candidate.codec_type.as_deref() == Some("audio"))
            .is_none_or(|audio| audio.codec_name.as_deref() == Some("aac"));
        let bits_per_second = size_bytes.saturating_mul(8_000) / duration_millis.max(1);
        let remuxable = matches!(format, InputFormat::Mp4 | InputFormat::Mov)
            && stream.codec_name.as_deref() == Some("h264")
            && stream
                .profile
                .as_deref()
                .is_some_and(|profile| REMUX_PROFILES.contains(&profile))
            && stream
                .level
                .is_some_and(|level| (1..=REMUX_MAX_LEVEL).contains(&level))
            && stream.pix_fmt.as_deref() == Some("yuv420p")
            && stream.color_range.as_deref() != Some("pc")
            && !stream.is_hdr()
            && !stream.is_rotated()
            && width <= VIDEO_MAX_WIDTH
            && height <= VIDEO_MAX_HEIGHT
            && audio_copyable
            && bits_per_second <= REMUX_MAX_BITS_PER_SECOND;
        if remuxable {
            Self::Remux
        } else {
            Self::Transcode
        }
    }
}

impl CodecPipeline {
    pub(super) async fn process_video(
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
            kind: RequestAttachmentKind::Video,
            media_type: media_type(format),
            size_bytes,
            width: display_width,
            height: display_height,
            duration_millis: Some(source_duration_millis),
        };

        let playback = work_dir.join("video-playback.mp4");
        let conversion = PlaybackConversion::choose(
            format,
            &source_probe,
            source_stream,
            size_bytes,
            source_duration_millis,
        );
        let args = match conversion {
            PlaybackConversion::Remux => self.remux_args(source, &playback),
            PlaybackConversion::Transcode => {
                self.transcode_args(source, source_stream.is_hdr(), &playback)
            }
        };
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
                RequestAttachmentDerivativeKind::VideoPlayback,
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
                RequestAttachmentDerivativeKind::VideoPoster,
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
            playback: Some(conversion),
        })
    }

    fn remux_args(&self, source: &Path, playback: &Path) -> Vec<OsString> {
        let mut args = self.transcode_input(source, false);
        args.extend(os_args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c",
            "copy",
            "-bsf:v",
            REMUX_BITSTREAM_FILTERS,
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-movflags",
            "+faststart",
        ]));
        args.push(path_arg(playback));
        args
    }

    fn transcode_args(&self, source: &Path, hdr: bool, playback: &Path) -> Vec<OsString> {
        let scale = format!(
            "scale=w='min({VIDEO_MAX_WIDTH},iw)':h='min({VIDEO_MAX_HEIGHT},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
        );
        let filter = if hdr {
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
            "-x264-params".into(),
            "colorprim=bt709:transfer=bt709:colormatrix=bt709".into(),
            "-preset".into(),
            "veryfast".into(),
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
            path_arg(playback),
        ]);
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS: u64 = 12_500_000;

    fn screen_recording() -> Value {
        json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "profile": "High",
                    "level": 42,
                    "pix_fmt": "yuv420p",
                    "width": 1920,
                    "height": 1080
                },
                { "codec_type": "audio", "codec_name": "aac" }
            ]
        })
    }

    fn conversion(format: InputFormat, probe: Value, size_bytes: u64) -> PlaybackConversion {
        let probe: Probe = serde_json::from_value(probe).unwrap();
        let stream = single_video_stream(&probe).unwrap();
        PlaybackConversion::choose(format, &probe, stream, size_bytes, 10_000)
    }

    #[test]
    fn browser_ready_mp4_and_mov_sources_are_remuxed() {
        for format in [InputFormat::Mp4, InputFormat::Mov] {
            assert_eq!(
                conversion(format, screen_recording(), SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS),
                PlaybackConversion::Remux
            );
        }
        for profile in ["Constrained Baseline", "Baseline", "Main"] {
            let mut probe = screen_recording();
            probe["streams"][0]["profile"] = json!(profile);
            assert_eq!(
                conversion(InputFormat::Mp4, probe, SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS),
                PlaybackConversion::Remux,
                "{profile}"
            );
        }
        let mut silent = screen_recording();
        silent["streams"].as_array_mut().unwrap().pop();
        assert_eq!(
            conversion(InputFormat::Mp4, silent, SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS),
            PlaybackConversion::Remux
        );
    }

    #[test]
    fn sources_a_browser_cannot_play_as_is_are_transcoded() {
        let video_changes = [
            ("codec_name", json!("hevc")),
            ("profile", json!("High 10")),
            ("profile", json!("High 4:2:2")),
            ("profile", json!("High 4:4:4 Predictive")),
            ("level", json!(50)),
            ("pix_fmt", json!("yuv420p10le")),
            ("color_range", json!("pc")),
            ("color_transfer", json!("smpte2084")),
            ("side_data_list", json!([{ "rotation": 90 }])),
            ("width", json!(3840)),
        ];
        for (field, value) in video_changes {
            let mut probe = screen_recording();
            probe["streams"][0][field] = value;
            assert_eq!(
                conversion(InputFormat::Mp4, probe, SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS),
                PlaybackConversion::Transcode,
                "{field}"
            );
        }
        let mut opus_audio = screen_recording();
        opus_audio["streams"][1]["codec_name"] = json!("opus");
        assert_eq!(
            conversion(
                InputFormat::Mp4,
                opus_audio,
                SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS
            ),
            PlaybackConversion::Transcode
        );
        assert_eq!(
            conversion(
                InputFormat::Webm,
                screen_recording(),
                SIZE_AT_TEN_MBPS_FOR_TEN_SECONDS
            ),
            PlaybackConversion::Transcode
        );
        assert_eq!(
            conversion(InputFormat::Mp4, screen_recording(), 15_001_000),
            PlaybackConversion::Transcode
        );
    }
}
