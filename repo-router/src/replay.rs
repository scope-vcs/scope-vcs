use axum::{
    body::{Body, Bytes, HttpBody, to_bytes},
    http::StatusCode,
};
use std::time::Duration;
use tokio::sync::{Semaphore, SemaphorePermit};

const BUDGET_UNIT_BYTES: usize = 1024;

pub(crate) struct ReplayBuffer {
    budget: Semaphore,
    max_bytes: usize,
    body_timeout: Duration,
}

impl ReplayBuffer {
    pub(crate) fn new(
        budget_bytes: usize,
        max_bytes: usize,
        body_timeout: Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            max_bytes > 0 && budget_bytes >= max_bytes,
            "replay budget must hold at least one maximum-size body"
        );
        let units = u32::try_from(budget_bytes.div_ceil(BUDGET_UNIT_BYTES))
            .map_err(|_| anyhow::anyhow!("replay budget is too large"))?;
        anyhow::ensure!(
            !body_timeout.is_zero(),
            "incoming body timeout must be positive"
        );
        Ok(Self {
            budget: Semaphore::new(units as usize),
            max_bytes,
            body_timeout,
        })
    }

    pub(crate) async fn collect(
        &self,
        body: Body,
    ) -> Result<(SemaphorePermit<'_>, Bytes), (StatusCode, &'static str)> {
        let limit = match HttpBody::size_hint(&body).exact() {
            Some(length) if length > self.max_bytes as u64 => return Err(self.too_large()),
            Some(length) => length as usize,
            None => self.max_bytes,
        };
        let units =
            u32::try_from(limit.div_ceil(BUDGET_UNIT_BYTES)).map_err(|_| self.too_large())?;
        let deadline = tokio::time::Instant::now() + self.body_timeout;
        let permit = tokio::time::timeout_at(deadline, self.budget.acquire_many(units))
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Git replay capacity is occupied",
            ))?;
        let bytes = tokio::time::timeout_at(deadline, to_bytes(body, limit))
            .await
            .map_err(|_| {
                (
                    StatusCode::REQUEST_TIMEOUT,
                    "Git upload-pack body deadline exceeded",
                )
            })?
            .map_err(|_| self.too_large())?;
        Ok((permit, bytes))
    }

    fn too_large(&self) -> (StatusCode, &'static str) {
        tracing::warn!(
            max_bytes = self.max_bytes,
            "Git upload-pack request exceeds router replay bound"
        );
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "Git upload-pack request is too large",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::wrappers::ReceiverStream;

    #[tokio::test]
    async fn incomplete_incoming_body_times_out_and_releases_admission() {
        let buffers = ReplayBuffer::new(4, 4, Duration::from_millis(20)).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
        sender.send(Ok(Bytes::from_static(b"ab"))).await.unwrap();
        let response = buffers
            .collect(Body::from_stream(ReceiverStream::new(receiver)))
            .await
            .err()
            .unwrap();
        assert_eq!(response.0, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(buffers.collect(Body::from("1234")).await.unwrap().1, "1234");
        drop(sender);
    }

    #[tokio::test]
    async fn small_bodies_share_the_budget_and_large_bodies_wait_for_release() {
        let buffers = ReplayBuffer::new(4 * 1024, 4 * 1024, Duration::from_millis(200)).unwrap();
        let mut small = Vec::new();
        for _ in 0..3 {
            small.push(buffers.collect(Body::from("0032want")).await.unwrap());
        }
        assert!(small.iter().all(|(_, body)| body == "0032want"));

        let (_sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
        let unknown_length = buffers
            .collect(Body::from_stream(ReceiverStream::new(receiver)))
            .await
            .err()
            .unwrap();
        assert_eq!(
            unknown_length.0,
            StatusCode::SERVICE_UNAVAILABLE,
            "a body without a length reserves the maximum before it is read"
        );

        let large = vec![b'x'; 2 * 1024];
        let waiting = buffers.collect(Body::from(large.clone()));
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err(),
            "a body larger than the free budget waits"
        );
        drop(small);
        assert_eq!(waiting.await.unwrap().1, large);

        let held = buffers
            .collect(Body::from(vec![b'y'; 4 * 1024]))
            .await
            .unwrap();
        let denied = buffers.collect(Body::from("1")).await.err().unwrap();
        assert_eq!(denied.0, StatusCode::SERVICE_UNAVAILABLE);
        drop(held);
        assert!(buffers.collect(Body::empty()).await.is_ok());
    }
}
