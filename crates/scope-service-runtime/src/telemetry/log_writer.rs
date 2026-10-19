use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
};
use tracing_subscriber::fmt::MakeWriter;

const QUEUED_LOG_LINES: usize = 16_384;

pub(super) struct QueuedLogWriter {
    lines: SyncSender<Vec<u8>>,
    dropped: Arc<AtomicU64>,
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
}

fn write_queued_lines(output: impl Write, queued: Receiver<Vec<u8>>, dropped: &AtomicU64) {
    let mut output = io::BufWriter::new(output);
    while let Ok(line) = queued.recv() {
        let _ = output.write_all(&line);
        while let Ok(line) = queued.try_recv() {
            let _ = output.write_all(&line);
        }
        let skipped = dropped.swap(0, Ordering::Relaxed);
        if skipped > 0 {
            let _ = writeln!(
                output,
                "scope log writer dropped {skipped} lines while its output was saturated"
            );
        }
        let _ = output.flush();
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
        match self.writer.lines.try_send(std::mem::take(&mut self.line)) {
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

    impl Write for SharedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn wait_for(output: &SharedOutput, expected: &str) -> String {
        for _ in 0..200 {
            let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
            if text.contains(expected) {
                return text;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("log writer never produced {expected:?}");
    }

    #[test]
    fn queued_lines_reach_the_output_whole_and_in_order() {
        let output = SharedOutput::default();
        let target = output.clone();
        let writer = QueuedLogWriter::start(move || target, 8).unwrap();
        for index in 0..3 {
            let mut line = writer.make_writer();
            write!(line, "line ").unwrap();
            writeln!(line, "{index}").unwrap();
        }
        assert_eq!(wait_for(&output, "line 2\n"), "line 0\nline 1\nline 2\n");
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
        drop(held);
        assert_eq!(
            wait_for(&output, "dropped 3 lines"),
            "line 0\nline 1\nscope log writer dropped 3 lines while its output was saturated\n"
        );
    }
}
