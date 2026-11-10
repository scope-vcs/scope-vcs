use super::ExecutionSink;
use anyhow::Context as _;
use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
pub(crate) const HEARTBEAT_FAILURE_TOLERANCE: Duration = Duration::from_secs(60);

pub(crate) struct Heartbeat {
    stop: mpsc::Sender<()>,
    results: mpsc::Receiver<anyhow::Result<bool>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Heartbeat {
    pub(crate) fn start<S: ExecutionSink>(
        sink: Arc<S>,
        interval: Duration,
        failure_tolerance: Duration,
    ) -> anyhow::Result<Self> {
        let (stop, receive) = mpsc::channel();
        let (send, results) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("runtime-heartbeat".into())
            .spawn(move || {
                let mut last_success = Instant::now();
                while matches!(
                    receive.recv_timeout(interval),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    match sink.heartbeat() {
                        Ok(false) => last_success = Instant::now(),
                        Ok(true) => {
                            let _ = send.send(Ok(true));
                            break;
                        }
                        Err(error) if last_success.elapsed() < failure_tolerance => {
                            eprintln!("runtime heartbeat failed; retrying: {error:#}");
                        }
                        Err(error) => {
                            let _ = send.send(Err(error));
                            break;
                        }
                    }
                }
            })
            .context("start runtime heartbeat")?;
        Ok(Self {
            stop,
            results,
            thread: Some(thread),
        })
    }

    pub(crate) fn poll(&self) -> Option<anyhow::Result<bool>> {
        match self.results.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("runtime heartbeat stopped")))
            }
        }
    }

    pub(crate) fn finish(mut self) -> anyhow::Result<bool> {
        self.join()?;
        self.results.try_recv().unwrap_or(Ok(false))
    }

    fn join(&mut self) -> anyhow::Result<()> {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("runtime heartbeat panicked"))?;
        }
        Ok(())
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        if let Err(error) = self.join() {
            eprintln!("{error}");
        }
    }
}
