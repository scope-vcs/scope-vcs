use super::{CodecFailure, CodecFailureKind, InputFormat};
use serde::Deserialize;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Debug, Deserialize)]
pub(super) struct Probe {
    #[serde(default)]
    pub(super) streams: Vec<ProbeStream>,
    #[serde(default)]
    frames: Vec<ProbeFrame>,
    pub(super) format: Option<ProbeFormat>,
}

impl Probe {
    pub(super) fn display_dimensions(&self, stream: &ProbeStream) -> (u32, u32) {
        let rotation = stream.rotation().or_else(|| {
            self.frames
                .iter()
                .find(|frame| frame.width > 0 && frame.height > 0)
                .and_then(ProbeFrame::rotation)
        });
        if rotation.is_some_and(|rotation| matches!(rotation.rem_euclid(360), 90 | 270)) {
            (stream.height, stream.width)
        } else {
            (stream.width, stream.height)
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ProbeStream {
    pub(super) codec_type: Option<String>,
    pub(super) codec_name: Option<String>,
    #[serde(default)]
    pub(super) width: u32,
    #[serde(default)]
    pub(super) height: u32,
    pub(super) duration: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    color_space: Option<String>,
    #[serde(default)]
    tags: ProbeTags,
    #[serde(default)]
    side_data_list: Vec<ProbeSideData>,
}

impl ProbeStream {
    pub(super) fn is_hdr(&self) -> bool {
        matches!(
            self.color_transfer.as_deref(),
            Some("smpte2084" | "arib-std-b67")
        ) || self.color_primaries.as_deref() == Some("bt2020")
            || self.color_space.as_deref() == Some("bt2020nc")
    }

    fn rotation(&self) -> Option<i32> {
        self.side_data_list
            .iter()
            .find_map(|side_data| side_data.rotation)
            .or_else(|| self.tags.rotate.as_deref()?.parse::<i32>().ok())
    }
}

#[derive(Debug, Deserialize)]
struct ProbeFrame {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    side_data_list: Vec<ProbeSideData>,
}

impl ProbeFrame {
    fn rotation(&self) -> Option<i32> {
        self.side_data_list
            .iter()
            .find_map(|side_data| side_data.rotation)
    }
}

#[derive(Debug, Default, Deserialize)]
struct ProbeTags {
    rotate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeSideData {
    rotation: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProbeFormat {
    pub(super) duration: Option<String>,
}

pub(super) fn sniff_format(path: &Path) -> Result<InputFormat, CodecFailure> {
    let mut file = fs::File::open(path).map_err(|error| {
        CodecFailure::new(
            CodecFailureKind::Internal,
            format!("reading media source: {error}"),
        )
    })?;
    let mut buffer = [0_u8; 64];
    let read = file.read(&mut buffer).map_err(|error| {
        CodecFailure::new(
            CodecFailureKind::Internal,
            format!("reading media source: {error}"),
        )
    })?;
    let head = &buffer[..read];
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(InputFormat::Png);
    }
    if head.starts_with(b"\xff\xd8\xff") {
        return Ok(InputFormat::Jpeg);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Ok(InputFormat::Gif);
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Ok(InputFormat::Webp);
    }
    if head.starts_with(b"\x1aE\xdf\xa3") {
        return Ok(InputFormat::Webm);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        let brand = &head[8..12];
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1"
        ) {
            return Ok(InputFormat::Heic);
        }
        if brand == b"qt  " {
            return Ok(InputFormat::Mov);
        }
        return Ok(InputFormat::Mp4);
    }
    Err(CodecFailure::new(
        CodecFailureKind::UnsupportedMedia,
        "media signature is not PNG, JPEG, WebP, GIF, HEIC, MP4, MOV, or WebM",
    ))
}

pub(super) fn media_type(format: InputFormat) -> &'static str {
    match format {
        InputFormat::Png => "image/png",
        InputFormat::Jpeg => "image/jpeg",
        InputFormat::Webp => "image/webp",
        InputFormat::Gif => "image/gif",
        InputFormat::Heic => "image/heic",
        InputFormat::Mp4 => "video/mp4",
        InputFormat::Mov => "video/quicktime",
        InputFormat::Webm => "video/webm",
    }
}

pub(super) fn single_video_stream(probe: &Probe) -> Result<&ProbeStream, CodecFailure> {
    let mut streams = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("video"));
    let stream = streams.next().ok_or_else(|| {
        CodecFailure::new(
            CodecFailureKind::InvalidMedia,
            "media contains no visual stream",
        )
    })?;
    if streams.next().is_some() || stream.width == 0 || stream.height == 0 {
        return Err(CodecFailure::new(
            CodecFailureKind::UnsupportedMedia,
            "media must contain exactly one nonempty visual stream",
        ));
    }
    Ok(stream)
}

pub(super) fn validate_pixels(
    width: u32,
    height: u32,
    maximum: u64,
    label: &str,
) -> Result<(), CodecFailure> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| {
            CodecFailure::new(
                CodecFailureKind::MediaLimitExceeded,
                format!("{label} dimensions overflow"),
            )
        })?;
    if pixels > maximum {
        return Err(CodecFailure::new(
            CodecFailureKind::MediaLimitExceeded,
            format!("{label} exceeds the {maximum}-pixel limit"),
        ));
    }
    Ok(())
}

pub(super) fn duration_millis(probe: &Probe, stream: &ProbeStream) -> Result<u64, CodecFailure> {
    let value = stream
        .duration
        .as_deref()
        .or_else(|| probe.format.as_ref()?.duration.as_deref())
        .ok_or_else(|| {
            CodecFailure::new(CodecFailureKind::InvalidMedia, "video duration is missing")
        })?;
    let mut duration = parse_duration_millis(value)?;
    for candidate in probe
        .format
        .as_ref()
        .and_then(|format| format.duration.as_deref())
        .into_iter()
        .chain(
            probe
                .streams
                .iter()
                .filter(|candidate| candidate.codec_type.as_deref() == Some("audio"))
                .filter_map(|candidate| candidate.duration.as_deref()),
        )
    {
        duration = duration.max(parse_duration_millis(candidate)?);
    }
    Ok(duration)
}

fn parse_duration_millis(value: &str) -> Result<u64, CodecFailure> {
    let seconds = value.parse::<f64>().map_err(|_| {
        CodecFailure::new(CodecFailureKind::InvalidMedia, "media duration is invalid")
    })?;
    if !seconds.is_finite() || seconds <= 0.0 || seconds > u64::MAX as f64 / 1_000.0 {
        return Err(CodecFailure::new(
            CodecFailureKind::InvalidMedia,
            "media duration is invalid",
        ));
    }
    Ok((seconds * 1_000.0).round() as u64)
}

pub(super) fn dimensions_from_heif_info(output: &str) -> Result<(u32, u32), CodecFailure> {
    output
        .split_ascii_whitespace()
        .filter_map(|word| {
            let word = word.trim_matches(|character: char| !character.is_ascii_alphanumeric());
            let (width, height) = word.split_once('x')?;
            let width = width.parse::<u32>().ok()?;
            let height = height.parse::<u32>().ok()?;
            (width > 0 && height > 0).then_some((width, height))
        })
        .max_by_key(|(width, height)| u64::from(*width) * u64::from(*height))
        .ok_or_else(|| {
            CodecFailure::new(
                CodecFailureKind::CorruptMedia,
                "heif-info did not report image dimensions",
            )
        })
}

pub(super) fn reject_webp_metadata(path: &Path) -> Result<(), CodecFailure> {
    let mut file = fs::File::open(path).map_err(|error| {
        CodecFailure::new(
            CodecFailureKind::Internal,
            format!("reading derivative: {error}"),
        )
    })?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).map_err(|error| {
        CodecFailure::new(
            CodecFailureKind::CodecFailed,
            format!("invalid WebP derivative: {error}"),
        )
    })?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WEBP" {
        return Err(CodecFailure::new(
            CodecFailureKind::CodecFailed,
            "invalid WebP derivative",
        ));
    }
    loop {
        let mut chunk_header = [0_u8; 8];
        match file.read_exact(&mut chunk_header) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => {
                return Err(CodecFailure::new(
                    CodecFailureKind::Internal,
                    format!("reading WebP derivative: {error}"),
                ));
            }
        }
        if matches!(&chunk_header[..4], b"EXIF" | b"XMP ") {
            return Err(CodecFailure::new(
                CodecFailureKind::CodecFailed,
                "image derivative retained source metadata",
            ));
        }
        let size = u32::from_le_bytes(chunk_header[4..].try_into().expect("four-byte chunk size"));
        let padded = u64::from(size) + u64::from(size % 2);
        file.seek(SeekFrom::Current(i64::try_from(padded).map_err(|_| {
            CodecFailure::new(CodecFailureKind::CodecFailed, "invalid WebP chunk size")
        })?))
        .map_err(|error| CodecFailure::new(CodecFailureKind::Internal, error.to_string()))?;
    }
}

pub(super) fn verify_faststart(path: &Path) -> Result<(), CodecFailure> {
    let mut file = fs::File::open(path).map_err(|error| {
        CodecFailure::new(
            CodecFailureKind::Internal,
            format!("reading video derivative: {error}"),
        )
    })?;
    let file_len = file
        .metadata()
        .map_err(|error| CodecFailure::new(CodecFailureKind::Internal, error.to_string()))?
        .len();
    let mut offset = 0_u64;
    let mut saw_moov = false;
    while offset.saturating_add(8) <= file_len {
        let mut header = [0_u8; 8];
        file.read_exact(&mut header).map_err(|error| {
            CodecFailure::new(
                CodecFailureKind::CodecFailed,
                format!("invalid MP4 derivative: {error}"),
            )
        })?;
        let kind = &header[4..8];
        if kind == b"moov" {
            saw_moov = true;
        }
        if kind == b"mdat" {
            return if saw_moov {
                Ok(())
            } else {
                Err(CodecFailure::new(
                    CodecFailureKind::CodecFailed,
                    "MP4 derivative is missing a fast-start moov atom",
                ))
            };
        }
        let short_size = u32::from_be_bytes(header[..4].try_into().expect("four-byte box size"));
        let (box_size, header_size) = match short_size {
            0 => (file_len.saturating_sub(offset), 8_u64),
            1 => {
                let mut extended = [0_u8; 8];
                file.read_exact(&mut extended).map_err(|error| {
                    CodecFailure::new(
                        CodecFailureKind::CodecFailed,
                        format!("invalid MP4 derivative: {error}"),
                    )
                })?;
                (u64::from_be_bytes(extended), 16)
            }
            size => (u64::from(size), 8),
        };
        if box_size < header_size || offset.saturating_add(box_size) > file_len {
            break;
        }
        offset += box_size;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| CodecFailure::new(CodecFailureKind::Internal, error.to_string()))?;
    }
    Err(CodecFailure::new(
        CodecFailureKind::CodecFailed,
        "MP4 derivative has no media data atom",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_signatures_without_trusting_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let cases: &[(&[u8], InputFormat)] = &[
            (b"\x89PNG\r\n\x1a\nrest", InputFormat::Png),
            (b"\xff\xd8\xffrest", InputFormat::Jpeg),
            (b"GIF89arest", InputFormat::Gif),
            (b"RIFF0000WEBPrest", InputFormat::Webp),
            (b"\x00\x00\x00\x18ftypheicrest", InputFormat::Heic),
            (b"\x00\x00\x00\x18ftypqt  rest", InputFormat::Mov),
            (b"\x00\x00\x00\x18ftypisomrest", InputFormat::Mp4),
            (b"\x1aE\xdf\xa3rest", InputFormat::Webm),
        ];
        for (index, (bytes, expected)) in cases.iter().enumerate() {
            let path = dir.path().join(format!("fixture-{index}.bin"));
            fs::write(&path, bytes).unwrap();
            assert_eq!(sniff_format(&path).unwrap(), *expected);
        }
    }

    #[test]
    fn rejects_non_media_signatures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.jpg");
        fs::write(&path, b"not really a jpeg").unwrap();
        assert_eq!(
            sniff_format(&path).unwrap_err().kind,
            CodecFailureKind::UnsupportedMedia
        );
    }
}
