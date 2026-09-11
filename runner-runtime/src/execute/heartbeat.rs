use super::ExecutionSink;
use anyhow::Context as _;
use std::{
    sync::{Arc, mpsc},
    thread,
};

/// The supervisor never performs network I/O on its deadline-checking thread.
/// One owned worker serializes heartbeats and is joined after process cleanup.
pub(super) struct Heartbeat {
    requests: Option<mpsc::Sender<()>>,
    responses: mpsc::Receiver<anyhow::Result<bool>>,
    thread: Option<thread::JoinHandle<()>>,
    in_flight: bool,
}

impl Heartbeat {
    pub(super) fn start<S: ExecutionSink>(sink: Arc<S>) -> anyhow::Result<Self> {
        let (requests, receive) = mpsc::channel();
        let (send, responses) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("step-heartbeat".into())
            .spawn(move || {
                while receive.recv().is_ok() {
                    if send.send(sink.heartbeat()).is_err() {
                        break;
                    }
                }
            })
            .context("start step heartbeat worker")?;
        Ok(Self {
            requests: Some(requests),
            responses,
            thread: Some(thread),
            in_flight: false,
        })
    }

    pub(super) fn request(&mut self) -> anyhow::Result<()> {
        if !self.in_flight {
            self.requests
                .as_ref()
                .expect("heartbeat worker is active")
                .send(())
                .context("step heartbeat worker stopped")?;
            self.in_flight = true;
        }
        Ok(())
    }

    pub(super) fn poll(&mut self) -> Option<anyhow::Result<bool>> {
        match self.responses.try_recv() {
            Ok(result) => {
                self.in_flight = false;
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("step heartbeat worker stopped")))
            }
        }
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("step heartbeat worker panicked");
        }
    }
}
