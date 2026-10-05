use super::step::StepConclusion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupFailure {
    ProviderRejected,
    RuntimeSetup,
}

const EX_UNAVAILABLE: i32 = 69;
const EX_SOFTWARE: i32 = 70;

impl SetupFailure {
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::ProviderRejected => EX_UNAVAILABLE,
            Self::RuntimeSetup => EX_SOFTWARE,
        }
    }
}

pub const SIGNAL_TERMINATED_EXIT_CODE: i32 = 128;

impl StepConclusion {
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
