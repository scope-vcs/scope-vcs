use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, Sender, SyncSender, TrySendError, channel, sync_channel},
    },
    time::{Duration, Instant},
};
use tracing_subscriber::fmt::MakeWriter;

const QUEUED_LOG_LINES: usize = 16_384;

enum Queued {
    Line(Vec<u8>),
    Flush(Sender<()>),
}

pub(super) struct QueuedLogWriter {
    lines: SyncSender<Queued>,
    dropped: Arc<AtomicU64>,
}

pub(super) struct LogFlush {
    lines: SyncSender<Queued>,
}

impl LogFlush {
    pub(super) fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (done, flushed) = channel();
        let mut request = Queued::Flush(done);
        loop {
            match self.lines.try_send(request) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    request = returned;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Full(_)) => return false,
            }
        }
        flushed
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
    }
}

impl QueuedLogWriter {
    pub(super) fn stdout() -> io::Result<Self> {
        Self::start(io::stdout, QUEUED_LOG_LINES)
    }

    fn start<W>(output: impl FnOnce() -> W + Send + 'static, capacity: usize) -> io::Result<Self>
    where
        W: Write,
    {
        let (lines, queued) = sync_channel(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_for_output = dropped.clone();
        std::thread::Builder::new()
            .name("scope-log-writer".to_string())
            .spawn(move || write_queued_lines(output(), queued, &dropped_for_output))?;
        Ok(Self { lines, dropped })
    }

    pub(super) fn flusher(&self) -> LogFlush {
        LogFlush {
            lines: self.lines.clone(),
        }
    }
}

fn write_queued_lines(output: impl Write, queued: Receiver<Queued>, dropped: &AtomicU64) {
    let mut output = io::BufWriter::new(output);
    let mut flushes = Vec::new();
    while let Ok(first) = queued.recv() {
        for message in std::iter::once(first).chain(queued.try_iter()) {
            match message {
                Queued::Line(line) => {
                    let _ = output.write_all(&line);
                }
                Queued::Flush(done) => flushes.push(done),
            }
        }
        let skipped = dropped.swap(0, Ordering::Relaxed);
        if skipped > 0 {
            let _ = writeln!(
                output,
                "scope log writer dropped {skipped} lines while its output was saturated"
            );
        }
        let _ = output.flush();
        for done in flushes.drain(..) {
            let _ = done.send(());
        }
    }
}

pub(super) struct QueuedLine<'a> {
    line: Vec<u8>,
    writer: &'a QueuedLogWriter,
}

impl Write for QueuedLine<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for QueuedLine<'_> {
    fn drop(&mut self) {
        if self.line.is_empty() {
            return;
        }
        match self
            .writer
            .lines
            .try_send(Queued::Line(std::mem::take(&mut self.line)))
        {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.writer.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl<'a> MakeWriter<'a> for QueuedLogWriter {
    type Writer = QueuedLine<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        QueuedLine {
            line: Vec::new(),
            writer: self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct SharedOutput(Arc<Mutex<Vec<u8>>>);

    impl SharedOutput {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for SharedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_flush_returns_after_every_earlier_line_is_written_whole_and_in_order() {
        let output = SharedOutput::default();
        let target = output.clone();
        let writer = QueuedLogWriter::start(move || target, 8).unwrap();
        for index in 0..3 {
            let mut line = writer.make_writer();
            write!(line, "line ").unwrap();
            writeln!(line, "{index}").unwrap();
        }
        assert!(writer.flusher().flush(Duration::from_secs(5)));
        assert_eq!(output.text(), "line 0\nline 1\nline 2\n");
    }

    #[test]
    fn a_saturated_output_drops_lines_instead_of_blocking_and_reports_the_count() {
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let output = SharedOutput::default();
        let target = output.clone();
        let gate_for_output = gate.clone();
        let writer = QueuedLogWriter::start(
            move || {
                drop(gate_for_output.lock().unwrap());
                target
            },
            2,
        )
        .unwrap();
        for index in 0..5 {
            writeln!(writer.make_writer(), "line {index}").unwrap();
        }
        assert_eq!(writer.dropped.load(Ordering::Relaxed), 3);
        assert!(!writer.flusher().flush(Duration::from_millis(50)));
        drop(held);
        assert!(writer.flusher().flush(Duration::from_secs(5)));
        assert_eq!(
            output.text(),
            "line 0\nline 1\nscope log writer dropped 3 lines while its output was saturated\n"
        );
    }
}
