use std::{
    any::Any,
    cell::Cell,
    future::Future,
    panic::{AssertUnwindSafe, PanicHookInfo},
    sync::OnceLock,
    task::Poll,
};

thread_local! {
    static PANIC_LOCATION: Cell<Option<String>> = const { Cell::new(None) };
}

pub(super) fn run_test_future<R: Send + 'static>(
    future: impl Future<Output = R> + Send + 'static,
) -> R {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    test_runtime().spawn(async move {
        let mut future = Some(Box::pin(future));
        let outcome = std::future::poll_fn(|cx| {
            PANIC_LOCATION.set(None);
            let polled = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let running = future
                    .as_mut()
                    .expect("a finished test database future is not polled again");
                let poll = running.as_mut().poll(cx);
                if poll.is_ready() {
                    future = None;
                }
                poll
            }));
            match polled {
                Ok(poll) => poll.map(Ok),
                Err(payload) => Poll::Ready(Err(panic_report(payload))),
            }
        })
        .await;
        let _ = sender.send(outcome);
    });
    match receiver
        .recv()
        .expect("test database runtime should not stop")
    {
        Ok(output) => output,
        Err(report) => panic!("test database future panicked at {report}"),
    }
}

pub(super) fn test_runtime() -> &'static tokio::runtime::Handle {
    static HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        record_panic_locations();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new()
                .expect("creating test database runtime should succeed");
            sender.send(runtime.handle().clone()).unwrap();
            runtime.block_on(std::future::pending::<()>());
        });
        receiver.recv().expect("test database runtime should start")
    })
}

fn record_panic_locations() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        PANIC_LOCATION.set(info.location().map(ToString::to_string));
        previous(info);
    }));
}

fn panic_report(payload: Box<dyn Any + Send>) -> String {
    let location = PANIC_LOCATION
        .take()
        .unwrap_or_else(|| "an unknown location".to_owned());
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
        .to_owned();
    std::mem::forget(payload);
    format!("{location}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_test_future_reports_the_future_panic_location_and_message() {
        let panic_line = line!() + 2;
        let payload = std::panic::catch_unwind(|| {
            run_test_future(async { panic!("future failed for its own reason") })
        })
        .unwrap_err();

        let message = payload.downcast_ref::<String>().unwrap();
        let location = format!("{}:{panic_line}:", file!());
        assert!(
            message.starts_with(&format!("test database future panicked at {location}"))
                && message.ends_with(": future failed for its own reason"),
            "{message}"
        );
    }

    #[test]
    fn run_test_future_reports_a_panic_while_dropping_a_finished_future() {
        struct ReadyThenPanicsOnDrop;
        impl Future for ReadyThenPanicsOnDrop {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }
        impl Drop for ReadyThenPanicsOnDrop {
            fn drop(&mut self) {
                panic!("finished future failed while dropping");
            }
        }

        let payload =
            std::panic::catch_unwind(|| run_test_future(ReadyThenPanicsOnDrop)).unwrap_err();

        let message = payload.downcast_ref::<String>().unwrap();
        assert!(
            message.ends_with(": finished future failed while dropping"),
            "{message}"
        );
    }
}
