use super::{
    job::RunJobState,
    step::{RunAttemptStep, StepState},
};

#[derive(Clone, Copy, Debug)]
pub struct JobLatestSteps<'a> {
    pub job_state: RunJobState,
    pub steps: &'a [RunAttemptStep],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunOpening<'a> {
    pub job: usize,
    pub step: Option<&'a RunAttemptStep>,
}

pub fn job_opening_step(steps: &[RunAttemptStep]) -> Option<&RunAttemptStep> {
    step_in(steps, StepState::Failed)
        .or_else(|| step_in(steps, StepState::Running))
        .or_else(|| steps.last())
}

pub fn run_opening<'a>(jobs: &[JobLatestSteps<'a>]) -> Option<RunOpening<'a>> {
    if let Some(job) = jobs
        .iter()
        .position(|latest| latest.job_state == RunJobState::Failed)
    {
        return Some(RunOpening {
            job,
            step: step_in(jobs[job].steps, StepState::Failed),
        });
    }
    jobs.iter()
        .enumerate()
        .find_map(|(job, latest)| {
            step_in(latest.steps, StepState::Running).map(|step| RunOpening {
                job,
                step: Some(step),
            })
        })
        .or_else(|| {
            let job = jobs.len().checked_sub(1)?;
            jobs[job].steps.last().map(|step| RunOpening {
                job,
                step: Some(step),
            })
        })
}

fn step_in(steps: &[RunAttemptStep], state: StepState) -> Option<&RunAttemptStep> {
    steps.iter().find(|step| step.state == state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RunJobState::{Failed, Running, Succeeded};

    fn steps(states: &[StepState]) -> Vec<RunAttemptStep> {
        states
            .iter()
            .enumerate()
            .map(|(index, state)| RunAttemptStep {
                attempt_id: "attempt".into(),
                step_index: index as u32,
                state: *state,
                started_at_unix: None,
                completed_at_unix: None,
                exit_code: None,
            })
            .collect()
    }

    fn opening(jobs: &[(RunJobState, &[RunAttemptStep])]) -> Option<(usize, Option<u32>)> {
        let latest = jobs
            .iter()
            .map(|(job_state, steps)| JobLatestSteps {
                job_state: *job_state,
                steps,
            })
            .collect::<Vec<_>>();
        run_opening(&latest).map(|opening| (opening.job, opening.step.map(|step| step.step_index)))
    }

    #[test]
    fn a_job_opens_on_its_failed_step_then_its_running_step_then_its_last() {
        use StepState::{Failed, Pending, Running, Succeeded};
        let index =
            |states: &[StepState]| job_opening_step(&steps(states)).map(|step| step.step_index);
        assert_eq!(index(&[Succeeded, Running, Failed, Pending]), Some(2));
        assert_eq!(index(&[Succeeded, Running, Pending]), Some(1));
        assert_eq!(index(&[Succeeded, Succeeded]), Some(1));
        assert_eq!(index(&[]), None);
    }

    #[test]
    fn a_run_opens_on_a_failed_job_then_the_first_running_step_then_the_last_jobs_last_step() {
        let passed = steps(&[StepState::Succeeded, StepState::Succeeded]);
        let failed = steps(&[StepState::Succeeded, StepState::Failed]);
        let lost = steps(&[StepState::Succeeded, StepState::Lost]);
        let running = steps(&[StepState::Succeeded, StepState::Running]);
        assert_eq!(
            opening(&[(Running, &running), (Failed, &failed), (Failed, &failed)]),
            Some((1, Some(1)))
        );
        assert_eq!(opening(&[(Failed, &lost)]), Some((0, None)));
        assert_eq!(
            opening(&[
                (Succeeded, &passed),
                (Running, &running),
                (Running, &running)
            ]),
            Some((1, Some(1)))
        );
        assert_eq!(
            opening(&[(Running, &passed), (Succeeded, &passed)]),
            Some((1, Some(1)))
        );
        assert_eq!(opening(&[(Succeeded, &passed), (Running, &[])]), None);
        assert_eq!(opening(&[]), None);
    }
}
