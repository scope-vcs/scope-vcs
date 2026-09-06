use super::HistoryEntryKind;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryFeed {
    #[default]
    Updates,
    All,
}

impl HistoryFeed {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updates => "updates",
            Self::All => "all",
        }
    }

    pub fn includes(self, kind: HistoryEntryKind) -> bool {
        self == Self::All || kind != HistoryEntryKind::VisibilityChange
    }

    pub fn generation(self, history_generation: &str, repo_id: &str, audience: &str) -> String {
        let mut hash = Sha256::new();
        for value in [history_generation, repo_id, audience, self.as_str()] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
}
