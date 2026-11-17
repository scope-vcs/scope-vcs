use crate::error::ApiError;
use futures_util::{Stream, StreamExt as _};
use scope_git_process::ProcessLimits;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug)]
pub(crate) struct GitTransferLimits {
    pub(crate) idle: Duration,
    pub(crate) pack_processing: Duration,
    pub(crate) total: Duration,
}

impl GitTransferLimits {
    pub(crate) fn streaming_process(self) -> ProcessLimits {
        ProcessLimits::new(self.total).with_stdout_idle_timeout(self.idle)
    }

    pub(crate) fn start(self) -> GitTransferClock {
        GitTransferClock {
            idle: self.idle,
            pack_processing: self.pack_processing,
            deadline: Instant::now() + self.total,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GitTransferClock {
    idle: Duration,
    pack_processing: Duration,
    deadline: Instant,
}

impl GitTransferClock {
    pub(crate) fn deadline(self) -> Instant {
        self.deadline
    }

    pub(crate) fn pack_processing_deadline(self) -> Instant {
        (Instant::now() + self.pack_processing).min(self.deadline)
    }

    pub(crate) async fn next_upload_chunk<S>(
        self,
        chunks: &mut S,
    ) -> Result<Option<S::Item>, ApiError>
    where
        S: Stream + Unpin,
    {
        let idle_deadline = Instant::now() + self.idle;
        tokio::time::timeout_at(idle_deadline.min(self.deadline), chunks.next())
            .await
            .map_err(|_| {
                ApiError::infrastructure_unavailable(if idle_deadline < self.deadline {
                    "git request upload stalled"
                } else {
                    "git request upload timed out"
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn limits(idle_ms: u64, total_ms: u64) -> GitTransferLimits {
        GitTransferLimits {
            idle: Duration::from_millis(idle_ms),
            pack_processing: Duration::from_millis(300),
            total: Duration::from_millis(total_ms),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pack_processing_starts_when_asked_and_never_outlasts_the_total_cap() {
        let clock = limits(100, 1_000).start();
        let started = Instant::now();
        tokio::time::advance(Duration::from_millis(200)).await;
        assert_eq!(
            clock.pack_processing_deadline() - started,
            Duration::from_millis(500)
        );
        tokio::time::advance(Duration::from_millis(700)).await;
        assert_eq!(
            clock.pack_processing_deadline() - started,
            Duration::from_millis(1_000)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn steady_upload_outlives_the_idle_limit_until_the_total_cap() {
        let clock = limits(100, 1_000).start();
        let mut chunks = Box::pin(stream::unfold((), |()| async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            Some(((), ()))
        }));
        let started = Instant::now();
        let error = loop {
            match clock.next_upload_chunk(&mut chunks).await {
                Ok(chunk) => assert!(chunk.is_some()),
                Err(error) => break error,
            }
        };
        assert_eq!(error.operator_diagnostic(), "git request upload timed out");
        assert_eq!(started.elapsed(), Duration::from_millis(1_000));
    }

    #[tokio::test(start_paused = true)]
    async fn silent_upload_ends_at_the_idle_limit() {
        let clock = limits(100, 1_000).start();
        let mut chunks = stream::pending::<()>();
        let started = Instant::now();
        let error = clock.next_upload_chunk(&mut chunks).await.unwrap_err();
        assert_eq!(error.operator_diagnostic(), "git request upload stalled");
        assert_eq!(started.elapsed(), Duration::from_millis(100));
    }
}
