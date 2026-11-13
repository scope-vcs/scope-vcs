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
    let mut nodes = BTreeMap::new();
    for member in members {
        let next = nodes.len();
        nodes.entry(member.request_id).or_insert(next);
    }
    for parent in members.iter().filter_map(|member| member.parent_request_id) {
        let next = nodes.len();
        nodes.entry(parent).or_insert(next);
    }
    let mut stacks = StackSets::new(nodes.len());
    for member in members {
        if let Some(parent) = member.parent_request_id {
            stacks.join(nodes[member.request_id], nodes[parent]);
        }
    }
    let needing_you = members
        .iter()
        .filter(|member| member.group == RequestQueueGroup::NeedsYou)
        .map(|member| stacks.stack(nodes[member.request_id]))
        .collect::<BTreeSet<_>>();
    members
        .iter()
        .map(|member| match member.group {
            RequestQueueGroup::Waiting
                if needing_you.contains(&stacks.stack(nodes[member.request_id])) =>
            {
                RequestQueueGroup::NeedsYou
            }
            group @ RequestQueueGroup::NeedsYou
            | group @ RequestQueueGroup::Waiting
            | group @ RequestQueueGroup::Unclaimed
            | group @ RequestQueueGroup::SetAside
            | group @ RequestQueueGroup::Done => group,
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
