use crate::policy::ScopePath;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileLineChanges {
    Text { additions: u64, deletions: u64 },
    Binary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
}

impl DiffStat {
    pub fn add(&mut self, lines: FileLineChanges) {
        if let FileLineChanges::Text {
            additions,
            deletions,
        } = lines
        {
            self.additions = self.additions.saturating_add(additions);
            self.deletions = self.deletions.saturating_add(deletions);
        }
        self.changed_files = self.changed_files.saturating_add(1);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiffStat {
    pub path: ScopePath,
    pub lines: FileLineChanges,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionDiffStat {
    totals: DiffStat,
    files: Option<Vec<FileDiffStat>>,
}

impl RevisionDiffStat {
    pub fn recorded(totals: DiffStat, files: Option<Vec<FileDiffStat>>) -> Self {
        Self { totals, files }
    }

    pub fn totals(&self) -> DiffStat {
        self.totals
    }

    pub fn files(&self) -> Option<&[FileDiffStat]> {
        self.files.as_deref()
    }

    pub fn readable_by(&self, can_read: impl Fn(&ScopePath) -> bool) -> Option<DiffStat> {
        let files = self.files.as_ref()?;
        let mut readable = DiffStat::default();
        for file in files.iter().filter(|file| can_read(&file.path)) {
            readable.add(file.lines);
        }
        Some(readable)
    }
}

pub struct RevisionDiffStatBuilder {
    totals: DiffStat,
    files: Option<Vec<FileDiffStat>>,
    max_files: usize,
}

impl RevisionDiffStatBuilder {
    pub fn new(max_files: usize) -> Self {
        Self {
            totals: DiffStat::default(),
            files: Some(Vec::new()),
            max_files,
        }
    }

    pub fn add(&mut self, path: Option<ScopePath>, lines: FileLineChanges) {
        self.totals.add(lines);
        let listed = match (self.files.as_mut(), path) {
            (Some(files), Some(path)) if files.len() < self.max_files => {
                files.push(FileDiffStat { path, lines });
                true
            }
            _ => false,
        };
        if !listed {
            self.files = None;
        }
    }

    pub fn finish(self) -> RevisionDiffStat {
        RevisionDiffStat {
            totals: self.totals,
            files: self.files,
        }
    }
}
