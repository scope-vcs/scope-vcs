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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameVisibility {
    Renamed,
    AddedAtNewPath,
    DeletedAtOldPath,
    Hidden,
}

impl RenameVisibility {
    pub fn for_reader(old_readable: bool, new_readable: bool) -> Self {
        match (old_readable, new_readable) {
            (true, true) => Self::Renamed,
            (false, true) => Self::AddedAtNewPath,
            (true, false) => Self::DeletedAtOldPath,
            (false, false) => Self::Hidden,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameSource {
    pub path: ScopePath,
    pub old_file: FileLineChanges,
    pub new_file: FileLineChanges,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiffStat {
    pub path: ScopePath,
    pub lines: FileLineChanges,
    pub renamed_from: Option<RenameSource>,
}

impl FileDiffStat {
    fn readable_lines(&self, can_read: impl Fn(&ScopePath) -> bool) -> Option<FileLineChanges> {
        let Some(source) = &self.renamed_from else {
            return can_read(&self.path).then_some(self.lines);
        };
        match RenameVisibility::for_reader(can_read(&source.path), can_read(&self.path)) {
            RenameVisibility::Renamed => Some(self.lines),
            RenameVisibility::AddedAtNewPath => Some(source.new_file),
            RenameVisibility::DeletedAtOldPath => Some(source.old_file),
            RenameVisibility::Hidden => None,
        }
    }
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
        for lines in files
            .iter()
            .filter_map(|file| file.readable_lines(&can_read))
        {
            readable.add(lines);
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

    pub fn add(&mut self, file: FileDiffStat) {
        self.totals.add(file.lines);
        match self.files.as_mut() {
            Some(files) if files.len() < self.max_files => files.push(file),
            _ => self.files = None,
        }
    }

    pub fn add_unlisted(&mut self, lines: FileLineChanges) {
        self.totals.add(lines);
        self.files = None;
    }

    pub fn finish(self) -> RevisionDiffStat {
        RevisionDiffStat {
            totals: self.totals,
            files: self.files,
        }
    }
}
