//! Exit codes the run pipeline assigns on behalf of a process that never
//! produced one, and the rule for reading a step's exit code.

use super::step::StepConclusion;

/// A failure reported before any workflow step ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupFailure {
    /// The execution provider refused to start the attempt.
    ProviderRejected,
    /// The runtime started but could not prepare the workspace.
    RuntimeSetup,
}

impl SetupFailure {
    /// sysexits.h: `EX_UNAVAILABLE` for a provider that would not serve the
    /// attempt, `EX_SOFTWARE` for a runtime that failed before the first step.
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::ProviderRejected => 69,
            Self::RuntimeSetup => 70,
        }
    }
}

/// Recorded for a step whose process was terminated by a signal instead of
/// exiting on its own, mirroring the shell convention.
pub const SIGNAL_TERMINATED_EXIT_CODE: i32 = 128;

impl StepConclusion {
    /// Zero is the only successful exit code.
    pub const fn from_exit_code(exit_code: i32) -> Self {
        if exit_code == 0 {
            Self::Succeeded
        } else {
            Self::Failed { exit_code }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_zero_is_a_successful_step_exit_code() {
        assert_eq!(StepConclusion::from_exit_code(0), StepConclusion::Succeeded);
        assert_eq!(
            StepConclusion::from_exit_code(3),
            StepConclusion::Failed { exit_code: 3 }
        );
        assert_eq!(
            StepConclusion::from_exit_code(SIGNAL_TERMINATED_EXIT_CODE),
            StepConclusion::Failed { exit_code: 128 }
        );
    }

    #[test]
    fn setup_failures_are_distinct_and_never_success() {
        assert_ne!(
            SetupFailure::ProviderRejected.exit_code(),
            SetupFailure::RuntimeSetup.exit_code()
        );
        for failure in [SetupFailure::ProviderRejected, SetupFailure::RuntimeSetup] {
            assert_ne!(failure.exit_code(), 0);
        }
    }
}
