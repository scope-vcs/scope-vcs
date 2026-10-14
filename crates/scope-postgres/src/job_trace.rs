use std::sync::OnceLock;

pub struct JobTracing {
    pub current_traceparent: fn() -> Option<String>,
    pub link_queued_traces: fn(&tracing::Span, &[&str]),
}

static JOB_TRACING: OnceLock<JobTracing> = OnceLock::new();

pub fn install(tracing: JobTracing) {
    let _ = JOB_TRACING.set(tracing);
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueuedTrace(Option<String>);

impl QueuedTrace {
    pub(crate) fn current() -> Self {
        Self(
            JOB_TRACING
                .get()
                .and_then(|tracing| (tracing.current_traceparent)()),
        )
    }

    pub(crate) fn from_column(traceparent: Option<String>) -> Self {
        Self(traceparent)
    }

    pub(crate) fn into_column(self) -> Option<String> {
        self.0
    }

    pub fn link(&self, span: &tracing::Span) {
        link_all(span, [self]);
    }
}

pub fn link_all<'a>(span: &tracing::Span, traces: impl IntoIterator<Item = &'a QueuedTrace>) {
    let Some(tracing) = JOB_TRACING.get() else {
        return;
    };
    let traceparents = traces
        .into_iter()
        .filter_map(|trace| trace.0.as_deref())
        .collect::<Vec<_>>();
    if !traceparents.is_empty() {
        (tracing.link_queued_traces)(span, &traceparents);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{JobTracing, install};
    use std::{future::Future, sync::Mutex};

    tokio::task_local! {
        static QUEUING_TRACE: String;
    }

    static LINKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn install_recording() {
        install(JobTracing {
            current_traceparent: || QUEUING_TRACE.try_with(Clone::clone).ok(),
            link_queued_traces: |_, traceparents| {
                LINKED.lock().unwrap().extend(
                    traceparents
                        .iter()
                        .map(|traceparent| traceparent.to_string()),
                )
            },
        });
    }

    pub(crate) async fn queued_by<F: Future>(traceparent: &str, work: F) -> F::Output {
        install_recording();
        QUEUING_TRACE.scope(traceparent.to_owned(), work).await
    }

    pub(crate) fn linked(traceparent: &str) -> bool {
        LINKED
            .lock()
            .unwrap()
            .iter()
            .any(|linked| linked == traceparent)
    }
}
