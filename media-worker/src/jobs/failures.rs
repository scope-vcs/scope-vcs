use crate::codec::{CodecFailure, CodecFailureKind};
use scope_domain::requests::attachments::{RequestAttachmentFailure, RequestAttachmentFailureCode};
use scope_media_storage::{MediaStorageError, MediaStorageErrorKind};

pub(super) const DECLARED_TYPE_MISMATCH: &str =
    "The uploaded media does not match its declared type. Upload it with the correct file type.";

pub(super) fn codec_failure(error: &CodecFailure) -> RequestAttachmentFailure {
    let code = match error.kind {
        CodecFailureKind::InvalidMedia => RequestAttachmentFailureCode::InvalidMedia,
        CodecFailureKind::CorruptMedia => RequestAttachmentFailureCode::CorruptMedia,
        CodecFailureKind::UnsupportedMedia => RequestAttachmentFailureCode::UnsupportedMedia,
        CodecFailureKind::MediaLimitExceeded => RequestAttachmentFailureCode::MediaLimitExceeded,
        CodecFailureKind::DeclaredKindMismatch => RequestAttachmentFailureCode::CorruptMedia,
        CodecFailureKind::TimedOut | CodecFailureKind::CodecFailed => {
            RequestAttachmentFailureCode::CodecFailed
        }
        CodecFailureKind::Internal => RequestAttachmentFailureCode::Internal,
    };
    RequestAttachmentFailure {
        code,
        message: codec_user_message(error.kind).to_owned(),
        retryable: error.retryable(),
    }
}

pub(super) fn storage_failure(error: &MediaStorageError) -> RequestAttachmentFailure {
    let (code, retryable) = match error.kind {
        MediaStorageErrorKind::Integrity | MediaStorageErrorKind::NotFound => {
            (RequestAttachmentFailureCode::CorruptMedia, false)
        }
        MediaStorageErrorKind::InvalidInput => (RequestAttachmentFailureCode::Internal, false),
        MediaStorageErrorKind::CapacityExhausted
        | MediaStorageErrorKind::Internal
        | MediaStorageErrorKind::ServiceUnavailable => {
            (RequestAttachmentFailureCode::StorageUnavailable, true)
        }
    };
    RequestAttachmentFailure {
        code,
        message: match code {
            RequestAttachmentFailureCode::CorruptMedia => {
                "The uploaded file could not be read. Upload it again."
            }
            RequestAttachmentFailureCode::StorageUnavailable => {
                "Media storage is temporarily unavailable. Retry processing."
            }
            _ => "Media processing failed. Retry processing.",
        }
        .to_owned(),
        retryable,
    }
}

fn codec_user_message(kind: CodecFailureKind) -> &'static str {
    match kind {
        CodecFailureKind::InvalidMedia => "The uploaded file is not valid media.",
        CodecFailureKind::CorruptMedia => {
            "The uploaded file is corrupt or could not be fully decoded."
        }
        CodecFailureKind::UnsupportedMedia => "This media format or codec is not supported.",
        CodecFailureKind::MediaLimitExceeded => "The uploaded file exceeds the media limits.",
        CodecFailureKind::DeclaredKindMismatch => DECLARED_TYPE_MISMATCH,
        CodecFailureKind::TimedOut => {
            "Media conversion took too long. Upload a shorter or smaller file."
        }
        CodecFailureKind::CodecFailed => "Media conversion failed. Retry processing.",
        CodecFailureKind::Internal => "Media processing failed. Retry processing.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::CodecPipeline;
    use scope_domain::requests::attachments::RequestAttachmentKind;

    #[tokio::test]
    async fn storage_retry_policy_distinguishes_absence_from_filesystem_failure() {
        async fn read_error(backend: &dyn scope_storage::ObjectBackend) -> MediaStorageError {
            let error = backend.read("missing").await.err().unwrap();
            MediaStorageError::from(scope_storage::ObjectStoreError::from(error))
        }
        use scope_storage::{FileBackend, MemoryBackend, ObjectBackend};
        let root = tempfile::tempdir().unwrap();
        let file_backend = FileBackend::new(root.path()).unwrap();
        let memory_backend = MemoryBackend::default();
        for backend in [&file_backend as &dyn ObjectBackend, &memory_backend] {
            let failure = storage_failure(&read_error(backend).await);
            assert_eq!(failure.code, RequestAttachmentFailureCode::CorruptMedia);
            assert!(!failure.retryable);
        }
        std::fs::write(root.path().join("objects"), b"not a directory").unwrap();
        let failure = storage_failure(&read_error(&file_backend).await);
        assert_eq!(
            failure.code,
            RequestAttachmentFailureCode::StorageUnavailable
        );
        assert!(failure.retryable);
    }

    #[tokio::test]
    async fn codec_time_limits_are_final_codec_failures() {
        use crate::config::{CodecLimits, CodecPrograms};
        use std::os::unix::fs::PermissionsExt;
        let work = tempfile::tempdir().unwrap();
        let stalled_probe = work.path().join("stalled-ffprobe");
        std::fs::write(&stalled_probe, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&stalled_probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let source = work.path().join("source.webm");
        std::fs::write(&source, b"\x1aE\xdf\xa3 stalled source").unwrap();
        let pipeline = CodecPipeline::new(
            CodecPrograms {
                ffprobe: stalled_probe,
                ..CodecPrograms::from_env()
            },
            CodecLimits {
                process_timeout: std::time::Duration::from_millis(200),
                ..CodecLimits::default()
            },
        );

        let error = pipeline
            .process(&source, work.path(), RequestAttachmentKind::Video)
            .await
            .err()
            .unwrap();

        let failure = codec_failure(&error);
        assert_eq!(failure.code, RequestAttachmentFailureCode::CodecFailed);
        assert!(!failure.retryable);
        assert!(failure.message.contains("took too long"));
    }
}
