use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryFeed {
    /// Pushes and merges; standalone visibility changes are excluded.
    #[default]
    Updates,
    All,
    /// Every action that changed visibility, including pushes that carried a change.
    Visibility,
}

impl HistoryFeed {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updates => "updates",
            Self::All => "all",
            Self::Visibility => "visibility",
        }
    }

    pub fn generation(self, history_generation: &str, repo_id: &str, audience: &str) -> String {
        let mut hash = Sha256::new();
        for value in [history_generation, repo_id, audience, self.as_str()] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        hex::encode(hash.finalize())
    }
}
