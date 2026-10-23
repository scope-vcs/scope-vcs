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
