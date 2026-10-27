use crate::{limits, resolve::kind_name};
use scope_domain::dependency_analysis::{DependencyEdge, DependencyGap};
use std::collections::BTreeSet;

#[derive(Default)]
pub(crate) struct Findings {
    edges: BTreeSet<DependencyEdge>,
    edges_exceeded: bool,
    gaps: BTreeSet<DependencyGap>,
    gaps_exceeded: bool,
}

impl Findings {
    pub(crate) fn add_edge(&mut self, edge: DependencyEdge) {
        if self.edges_exceeded {
            return;
        }
        self.edges.insert(edge);
        if self.edges.len() > limits::MAX_EDGES {
            self.edges.clear();
            self.edges_exceeded = true;
        }
    }

    pub(crate) fn add_gap(&mut self, gap: DependencyGap) {
        self.gaps.insert(gap);
        if self.gaps.len() > limits::MAX_GAPS {
            self.gaps.pop_last();
            self.gaps_exceeded = true;
        }
    }

    pub(crate) fn add_repository_gap(&mut self, reason: String) {
        self.add_gap(repository_gap(reason));
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.edges_exceeded |= other.edges_exceeded;
        if self.edges_exceeded {
            self.edges.clear();
        } else {
            other.edges.into_iter().for_each(|edge| self.add_edge(edge));
        }
        self.gaps_exceeded |= other.gaps_exceeded;
        other.gaps.into_iter().for_each(|gap| self.add_gap(gap));
    }

    pub(crate) fn within_limits(&self) -> bool {
        !self.edges_exceeded && !self.gaps_exceeded
    }

    pub(crate) fn into_parts(mut self) -> (Vec<DependencyEdge>, Vec<DependencyGap>) {
        if self.edges_exceeded {
            self.add_repository_gap(format!(
                "dependency edge limit exceeded ({})",
                limits::MAX_EDGES
            ));
        }
        let mut edges = self.edges.into_iter().collect::<Vec<_>>();
        edges.sort_by(|left, right| edge_key(left).cmp(&edge_key(right)));
        let mut gaps = self.gaps.into_iter().collect::<Vec<_>>();
        if self.gaps_exceeded {
            gaps.truncate(limits::MAX_GAPS - 1);
            gaps.push(repository_gap(format!(
                "coverage gap limit exceeded ({})",
                limits::MAX_GAPS
            )));
        }
        (edges, gaps)
    }
}

fn edge_key(edge: &DependencyEdge) -> (&str, &str, &str) {
    (&edge.source_path, &edge.target_path, kind_name(edge.kind))
}

pub(crate) fn repository_gap(reason: String) -> DependencyGap {
    DependencyGap {
        path: ".".into(),
        reason,
    }
}
