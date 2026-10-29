use crate::{error::ApiError, state::AppState};
use std::{future::Future, time::Duration};
use tokio::{
    sync::{Notify, watch},
    task::JoinHandle,
    time::{Interval, MissedTickBehavior},
};

#[derive(Default)]
pub(crate) struct BackgroundWakeups {
    pub(crate) auto_merge: Notify,
    pub(crate) clerk_user_deletion: Notify,
    pub(crate) github_job_logs: Notify,
    pub(crate) github_job_reads: Notify,
    pub(crate) github_pull_requests: Notify,
    pub(crate) github_pushes: Notify,
    pub(crate) github_review_bot_statuses: Notify,
    pub(crate) github_review_imports: Notify,
    pub(crate) github_run_imports: Notify,
    pub(crate) invite_emails: Notify,
    pub(crate) request_diff_stats: Notify,
}

pub(crate) type WakeupSelector = fn(&BackgroundWakeups) -> &Notify;

pub(crate) enum Cadence {
    AfterPass {
        delay: Duration,
        wakeup: Option<WakeupSelector>,
    },
    Fixed {
        period: Duration,
        missed_ticks: MissedTickBehavior,
    },
}

pub(crate) enum Readiness {
    SchemaRequired,
    Ungated,
}

pub(crate) struct BackgroundLoop {
    pub(crate) name: &'static str,
    pub(crate) cadence: Cadence,
    pub(crate) readiness: Readiness,
}

pub struct BackgroundWork {
    state: AppState,
    stop: watch::Sender<bool>,
    tasks: Vec<(&'static str, JoinHandle<()>)>,
}

impl AppState {
    pub fn start_background_work(&self) -> BackgroundWork {
        let mut work = BackgroundWork {
            state: self.clone(),
            stop: watch::channel(false).0,
            tasks: Vec::new(),
        };
        work.start_repository_cache_pruning();
        work.start_run_attempt_recovery();
        work.start_run_cache_reconciliation();
        work.start_retention();
        work.start_invite_email_delivery();
        work.start_github_pushes();
        work.start_github_pull_requests();
        work.start_github_run_imports();
        work.start_github_review_imports();
        work.start_github_review_bot_statuses();
        work.start_github_job_reads();
        work.start_github_job_log_reads();
        work.start_github_check_reconciliation();
        work.start_clerk_user_deletion();
        work.start_git_segment_recovery();
        work.start_request_check_recovery();
        work.start_request_auto_merge();
        work.start_request_diff_stat_backfill();
        work
    }
}

impl BackgroundWork {
    pub fn stop_signal(&self) -> watch::Sender<bool> {
        self.stop.clone()
    }

    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        for (name, task) in self.tasks {
            if let Err(error) = task.await {
                tracing::error!(task = name, %error, "background loop exited unexpectedly");
            }
        }
    }

    pub(crate) fn spawn<F, Fut, T>(&mut self, spec: BackgroundLoop, pass: F)
    where
        F: Fn(AppState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        self.spawn_with_progress(spec, (), move |state, ()| {
            let attempt = pass(state);
            async move { ((), attempt.await.map(drop)) }
        });
    }

    pub(crate) fn spawn_with_progress<P, F, Fut>(
        &mut self,
        spec: BackgroundLoop,
        initial: P,
        pass: F,
    ) where
        P: Clone + Send + 'static,
        F: Fn(AppState, P) -> Fut + Send + 'static,
        Fut: Future<Output = (P, Result<(), ApiError>)> + Send + 'static,
    {
        let BackgroundLoop {
            name,
            cadence,
            readiness,
        } = spec;
        let state = self.state.clone();
        let mut stopped = self.stop.subscribe();
        let task = tokio::spawn(async move {
            let mut schedule = Schedule::new(cadence);
            let mut progress = initial;
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = schedule.before_pass() => {},
                }
                let readiness_state = state.clone();
                let unchanged = progress.clone();
                let attempt = pass(state.clone(), progress.clone());
                let gated = matches!(readiness, Readiness::SchemaRequired);
                match tokio::spawn(async move {
                    if gated
                        && let Err(error) = readiness_state.metadata.admin().readiness_check().await
                    {
                        return (unchanged, Err(error.into()));
                    }
                    attempt.await
                })
                .await
                {
                    Ok((next, result)) => {
                        progress = next;
                        if let Err(error) = result {
                            tracing::warn!(
                                task = name,
                                error = %error.operator_diagnostic(),
                                "{name} failed; retrying"
                            );
                        }
                    }
                    Err(error) => tracing::error!(
                        task = name,
                        %error,
                        "{name} exited unexpectedly; retrying"
                    ),
                }
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = schedule.after_pass(&state) => {},
                }
            }
        });
        self.tasks.push((name, task));
    }
}

enum Schedule {
    AfterPass {
        delay: Duration,
        wakeup: Option<WakeupSelector>,
    },
    Fixed(Interval),
}

impl Schedule {
    fn new(cadence: Cadence) -> Self {
        match cadence {
            Cadence::AfterPass { delay, wakeup } => Self::AfterPass { delay, wakeup },
            Cadence::Fixed {
                period,
                missed_ticks,
            } => {
                let mut interval = tokio::time::interval(period);
                interval.set_missed_tick_behavior(missed_ticks);
                Self::Fixed(interval)
            }
        }
    }

    async fn before_pass(&mut self) {
        if let Self::Fixed(interval) = self {
            interval.tick().await;
        }
    }

    async fn after_pass(&self, state: &AppState) {
        let Self::AfterPass { delay, wakeup } = self else {
            return;
        };
        match wakeup {
            Some(select) => tokio::select! {
                _ = select(&state.wakeups).notified() => {},
                _ = tokio::time::sleep(*delay) => {},
            },
            None => tokio::time::sleep(*delay).await,
        }
    }
}
