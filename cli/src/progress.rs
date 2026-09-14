use anyhow::Context;
use std::{
    fmt,
    io::{self, IsTerminal, Write},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[path = "progress/process.rs"]
pub mod process;
pub use process::run_cancellable;

static ACTIVE_CANCELLATION: Mutex<Option<Weak<AtomicBool>>> = Mutex::new(None);
static CTRL_C_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> anyhow::Result<()> {
        if self.is_cancelled() {
            Err(Cancelled.into())
        } else {
            Ok(())
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("preparation canceled")
    }
}

impl std::error::Error for Cancelled {}

pub struct PreparationProgress {
    started: Instant,
    stage: Arc<Mutex<String>>,
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    render_lock: Arc<Mutex<()>>,
    rendered: bool,
    cancellation_supervisor: Option<JoinHandle<()>>,
    cancellation: CancellationToken,
}

impl PreparationProgress {
    pub fn start(stage: impl Into<String>) -> anyhow::Result<Self> {
        install_ctrl_c_handler()?;
        let cancellation = CancellationToken::new();
        *ACTIVE_CANCELLATION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Arc::downgrade(&cancellation.cancelled));

        let started = Instant::now();
        let stage = Arc::new(Mutex::new(stage.into()));
        let stopped = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let render_lock = Arc::new(Mutex::new(()));
        let should_render = crate::execution::interactive()
            && !crate::execution::json()
            && io::stderr().is_terminal();
        let renderer = should_render.then(|| {
            {
                let _guard = render_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                render_status(
                    &stage
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                    0,
                    started.elapsed(),
                );
            }
            let stage = Arc::clone(&stage);
            let stopped = Arc::clone(&stopped);
            let paused = Arc::clone(&paused);
            let render_lock = Arc::clone(&render_lock);
            thread::spawn(move || {
                let mut frame = 1;
                while !stopped.load(Ordering::Acquire) {
                    {
                        let _guard = render_lock
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if !paused.load(Ordering::Acquire) {
                            let stage = stage
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .clone();
                            render_status(&stage, frame, started.elapsed());
                        }
                    }
                    frame = frame.wrapping_add(1);
                    thread::sleep(Duration::from_millis(100));
                }
            })
        });
        let cancellation_supervisor = {
            let cancellation = cancellation.clone();
            let stopped = Arc::clone(&stopped);
            let render_lock = Arc::clone(&render_lock);
            Some(thread::spawn(move || {
                let mut terminate = false;
                while !stopped.load(Ordering::Acquire) {
                    if !cancellation.is_cancelled() {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }

                    // Managed children poll more frequently than this grace period and kill their
                    // process group before returning. Blocking in-process work such as reqwest has
                    // no cancellation API, so Ctrl+C terminates the CLI after that cleanup window.
                    let deadline = Instant::now() + Duration::from_millis(250);
                    while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(10));
                    }
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    terminate = true;
                    stopped.store(true, Ordering::Release);
                    break;
                }
                if let Some(renderer) = renderer {
                    let _ = renderer.join();
                    let _guard = render_lock
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    clear_status();
                }
                if terminate {
                    std::process::exit(130);
                }
            }))
        };

        Ok(Self {
            started,
            stage,
            stopped,
            paused,
            render_lock,
            rendered: should_render,
            cancellation_supervisor,
            cancellation,
        })
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn set_stage(&self, stage: impl Into<String>) -> anyhow::Result<()> {
        self.cancellation.check()?;
        let stage = stage.into();
        *self
            .stage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = stage.clone();
        if self.rendered {
            let _guard = self
                .render_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.paused.load(Ordering::Acquire) {
                render_status(&stage, 0, self.started.elapsed());
            }
        }
        Ok(())
    }

    pub fn pause(&self) -> ProgressPause<'_> {
        self.paused.store(true, Ordering::Release);
        let _guard = self
            .render_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Clearing under the render lock keeps an in-flight frame from redrawing
        // over the output that follows the pause.
        if self.rendered {
            clear_status();
        }
        ProgressPause { progress: self }
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        self.stop_renderer();
        self.cancellation.check()
    }

    fn stop_renderer(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(supervisor) = self.cancellation_supervisor.take() {
            let _ = supervisor.join();
        }
        self.unregister_ctrl_c();
    }

    fn unregister_ctrl_c(&mut self) {
        let mut active = ACTIVE_CANCELLATION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.as_ref().is_some_and(|active| {
            Weak::ptr_eq(active, &Arc::downgrade(&self.cancellation.cancelled))
        }) {
            *active = None;
        }
    }
}

impl Drop for PreparationProgress {
    fn drop(&mut self) {
        self.stop_renderer();
    }
}

pub struct ProgressPause<'a> {
    progress: &'a PreparationProgress,
}

impl Drop for ProgressPause<'_> {
    fn drop(&mut self) {
        self.progress.paused.store(false, Ordering::Release);
    }
}

fn install_ctrl_c_handler() -> anyhow::Result<()> {
    CTRL_C_HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(|| {
                if let Ok(active) = ACTIVE_CANCELLATION.lock()
                    && let Some(token) = active.as_ref().and_then(Weak::upgrade)
                {
                    token.store(true, Ordering::Release);
                    return;
                }
                std::process::exit(130);
            })
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| anyhow::anyhow!(message.clone()))
        .context("install Ctrl+C handler")?;
    Ok(())
}

fn render_status(stage: &str, frame: usize, elapsed: Duration) {
    const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let width = crossterm::terminal::size()
        .ok()
        .map(|(columns, _)| usize::from(columns))
        .filter(|width| *width > 0)
        .unwrap_or(80);
    let status = status_text(stage, SPINNER[frame % SPINNER.len()], elapsed, width);
    let mut stderr = io::stderr().lock();
    let _ = write!(stderr, "\r\x1b[2K{status}");
    let _ = stderr.flush();
}

fn status_text(stage: &str, spinner: char, elapsed: Duration, width: usize) -> String {
    let prefix = format!("{spinner} ");
    let suffix = format!(" · {}s elapsed", elapsed.as_secs());
    let fixed_width =
        UnicodeWidthStr::width(prefix.as_str()) + UnicodeWidthStr::width(suffix.as_str());
    if fixed_width >= width {
        return truncate_to_width(&format!("{prefix}{stage}{suffix}"), width, false);
    }
    format!(
        "{prefix}{}{suffix}",
        truncate_to_width(stage, width - fixed_width, true)
    )
}

fn truncate_to_width(value: &str, width: usize, ellipsis: bool) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    let ellipsis_width = usize::from(ellipsis && width > 0);
    let content_width = width.saturating_sub(ellipsis_width);
    let mut output = String::new();
    let mut used = 0;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > content_width {
            break;
        }
        output.push(character);
        used += character_width;
    }
    if ellipsis_width > 0 {
        output.push('…');
    }
    output
}

fn clear_status() {
    let mut stderr = io::stderr().lock();
    let _ = write!(stderr, "\r\x1b[2K");
    let _ = stderr.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_cancellation_tokens_do_not_share_state() {
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        first.cancel();
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        assert!(first.check().unwrap_err().is::<Cancelled>());
    }

    #[test]
    fn status_text_never_wraps_the_terminal_width() {
        for width in [1, 8, 20, 80] {
            let status = status_text(
                "Loading a repository configuration with a long path…",
                '⠋',
                Duration::from_secs(12),
                width,
            );
            assert!(
                UnicodeWidthStr::width(status.as_str()) <= width,
                "{status:?}"
            );
        }
        assert!(status_text("Loading…", '⠋', Duration::ZERO, 80).contains("0s elapsed"));
    }
}
