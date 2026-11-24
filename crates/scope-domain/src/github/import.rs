use serde::{Deserialize, Serialize};

const GITHUB_IMPORT_ERROR_MAX_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubImportState {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl GitHubImportState {
    pub fn is_working(self, last_error: Option<&str>) -> bool {
        match self {
            Self::Running => true,
            Self::Queued => last_error.is_none(),
            Self::Succeeded | Self::Failed => false,
        }
    }
}

pub fn github_import_error(message: &str) -> String {
    let message = message.trim();
    if message.chars().count() <= GITHUB_IMPORT_ERROR_MAX_CHARS {
        return message.to_string();
    }
    let mut kept = message
        .chars()
        .take(GITHUB_IMPORT_ERROR_MAX_CHARS - 1)
        .collect::<String>();
    kept.push('…');
    kept
}
