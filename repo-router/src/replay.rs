use axum::{
    body::{Body, Bytes, to_bytes},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Each admitted request reserves its full maximum buffer until forwarding ends.
/// This bounds aggregate replay memory as well as partially received bodies.
pub(crate) struct ReplayBuffer {
    slots: Arc<Semaphore>,
    max_bytes: usize,
    body_timeout: Duration,
}

pub(crate) struct BufferedRequest {
    pub(crate) bytes: Bytes,
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug)]
pub(crate) enum ReplayError {
    CapacityOccupied,
    BodyDeadlineExceeded,
    BodyTooLarge,
}

impl IntoResponse for ReplayError {
    fn into_response(self) -> Response {
        match self {
            Self::CapacityOccupied => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Git replay capacity is occupied",
            ),
            Self::BodyDeadlineExceeded => (
                StatusCode::REQUEST_TIMEOUT,
                "Git upload-pack body deadline exceeded",
            ),
            Self::BodyTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Git upload-pack request is too large",
            ),
        }
        .into_response()
    }
}

impl ReplayBuffer {
    pub(crate) fn new(
        slots: usize,
        max_bytes: usize,
        body_timeout: Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            slots > 0 && slots <= 64,
            "replay slots must be between 1 and 64"
        );
        anyhow::ensure!(
            max_bytes > 0 && max_bytes.checked_mul(slots).is_some(),
            "invalid aggregate replay byte budget"
        );
        anyhow::ensure!(
            !body_timeout.is_zero(),
            "incoming body timeout must be positive"
        );
        Ok(Self {
            slots: Arc::new(Semaphore::new(slots)),
            max_bytes,
            body_timeout,
        })
    }

    pub(crate) async fn collect(&self, body: Body) -> Result<BufferedRequest, ReplayError> {
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| ReplayError::CapacityOccupied)?;
        let bytes = tokio::time::timeout(self.body_timeout, to_bytes(body, self.max_bytes)).await
            .map_err(|_| ReplayError::BodyDeadlineExceeded)?
            .map_err(|error| {
                tracing::warn!(%error, max_bytes = self.max_bytes, "Git upload-pack request exceeds router replay bound");
                ReplayError::BodyTooLarge
            })?;
        Ok(BufferedRequest {
            bytes,
            _permit: permit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::wrappers::ReceiverStream;

    #[tokio::test]
    async fn incomplete_incoming_body_times_out_and_releases_admission() {
        let buffers = ReplayBuffer::new(1, 4, Duration::from_millis(20)).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
        sender.send(Ok(Bytes::from_static(b"ab"))).await.unwrap();
        let response = buffers
            .collect(Body::from_stream(ReceiverStream::new(receiver)))
            .await
            .err()
            .unwrap();
        assert_eq!(
            response.into_response().status(),
            StatusCode::REQUEST_TIMEOUT
        );
        assert_eq!(
            buffers.collect(Body::from("1234")).await.unwrap().bytes,
            "1234"
        );
        drop(sender);
    }

    #[tokio::test]
    async fn admission_is_held_through_forwarding_and_released_on_drop() {
        let buffers = ReplayBuffer::new(1, 4, Duration::from_secs(1)).unwrap();
        let first = buffers.collect(Body::from("1234")).await.unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
        let denied = buffers
            .collect(Body::from_stream(ReceiverStream::new(receiver)))
            .await
            .err()
            .unwrap();
        assert_eq!(
            denied.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        drop(first);
        assert!(buffers.collect(Body::empty()).await.is_ok());
        drop(sender);
    }
}
