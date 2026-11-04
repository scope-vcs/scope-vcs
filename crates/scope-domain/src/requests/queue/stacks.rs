use super::RequestQueueGroup;
use std::collections::{BTreeMap, BTreeSet};

pub struct RequestQueueStackMember<'a> {
    pub request_id: &'a str,
    pub parent_request_id: Option<&'a str>,
    pub group: RequestQueueGroup,
}

pub fn request_queue_stack_groups(
    members: &[RequestQueueStackMember<'_>],
) -> Vec<RequestQueueGroup> {
    let positions = members
        .iter()
        .enumerate()
        .map(|(position, member)| (member.request_id, position))
        .collect::<BTreeMap<_, _>>();
    let mut stacks = StackSets::new(members.len());
    for (position, member) in members.iter().enumerate() {
        if let Some(&parent) = member
            .parent_request_id
            .and_then(|parent| positions.get(parent))
        {
            stacks.join(position, parent);
        }
    }
    let needing_you = (0..members.len())
        .filter(|&position| members[position].group == RequestQueueGroup::NeedsYou)
        .map(|position| stacks.stack(position))
        .collect::<BTreeSet<_>>();
    (0..members.len())
        .map(|position| match members[position].group {
            RequestQueueGroup::Waiting if needing_you.contains(&stacks.stack(position)) => {
                RequestQueueGroup::NeedsYou
            }
            group => group,
        })
        .collect()
}

struct StackSets {
    parents: Vec<usize>,
}

impl StackSets {
    fn new(len: usize) -> Self {
        Self {
            parents: (0..len).collect(),
        }
    }

    fn stack(&mut self, mut position: usize) -> usize {
        while self.parents[position] != position {
            self.parents[position] = self.parents[self.parents[position]];
            position = self.parents[position];
        }
        position
    }

    fn join(&mut self, left: usize, right: usize) {
        let (left, right) = (self.stack(left), self.stack(right));
        self.parents[left] = right;
    }
}
