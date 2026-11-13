use super::{GitReader, settled};
use crate::error::ApiError;
use gix::ObjectId;
use std::{cell::Cell, time::Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MergeBases {
    Unrelated,
    Unique(String),
    Several,
}

impl MergeBases {
    pub(super) fn from_bases(bases: Vec<ObjectId>) -> Self {
        match bases.as_slice() {
            [] => Self::Unrelated,
            [base] => Self::Unique(base.to_string()),
            _ => Self::Several,
        }
    }
}

#[derive(Default)]
struct WalkOutcome {
    expired: Cell<bool>,
    missing: Cell<bool>,
}

struct DeadlineObjects<'walk> {
    objects: &'walk gix::OdbHandle,
    deadline: Instant,
    outcome: &'walk WalkOutcome,
}

impl gix::objs::Find for DeadlineObjects<'_> {
    fn try_find<'a>(
        &self,
        id: &gix::oid,
        buffer: &'a mut Vec<u8>,
    ) -> gix::Result<Option<gix::objs::Data<'a>>> {
        if Instant::now() >= self.deadline {
            self.outcome.expired.set(true);
            return Err(gix::Error::from_error(std::io::Error::from(
                std::io::ErrorKind::TimedOut,
            )));
        }
        let found = self.objects.try_find(id, buffer)?;
        if found.is_none() {
            self.outcome.missing.set(true);
        }
        Ok(found)
    }
}

pub(super) fn merge_bases(
    reader: &GitReader,
    one: ObjectId,
    two: ObjectId,
    action: &str,
) -> Result<Option<Vec<ObjectId>>, ApiError> {
    let outcome = WalkOutcome::default();
    let mut graph = gix::revwalk::Graph::new(
        DeadlineObjects {
            objects: &reader.repo.objects,
            deadline: reader.deadline,
            outcome: &outcome,
        },
        None,
    );
    let bases = gix_revision::merge_base(one, &[two], &mut graph);
    if outcome.expired.get() {
        return Err(reader.timed_out(action));
    }
    if outcome.missing.get() {
        return Ok(None);
    }
    Ok(settled(bases).map(|bases| {
        bases
            .map(|bases| bases.into_iter().collect())
            .unwrap_or_default()
    }))
}
