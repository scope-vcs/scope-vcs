use proptest::{collection::vec, option, prelude::*};
use scope_domain::{
    content::SourceBlob,
    content_ref::ContentRef,
    policy::{LabelRule, Policy, ScopePath},
    projection::{
        FileChange, LogicalCommit, LogicalCommitOrigin, NativeRequestCommit,
        NativeRequestCommitDetails, SourceGraph,
    },
    repo_control::is_private_control_path,
    views::{ViewDefinition, ViewId, ViewIncludes, ViewReaders, Views, ViewsTransition},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const HIGH_PATHS: [&str; 6] = [
    "/h/a",
    "/h/b",
    "/h/deep/c",
    "/.scope",
    "/.scope/secret",
    "/.scope/workflows/ci.yml",
];
const LOW_PATHS: [&str; 6] = [
    "/README.md",
    "/x",
    "/l/a",
    "/l/b",
    "/l/sub/c",
    "/.scope/RULES.md",
];
const LOW_RULE_PATHS: [&str; 5] = ["/", "/README.md", "/l", "/l/sub", "/x"];
const HIGH_RULE_PATHS: [&str; 4] = ["/h", "/h/a", "/h/deep", "/.scope/secret"];
const LABELS: [&str; 5] = ["public", "private", "c0", "c1", "c2"];
const READERS: [&str; 4] = ["public", "c0", "c1", "c2"];
const MODES: [&str; 3] = ["100644", "100755", "120000"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    A,
    B,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ChangeKind {
    Write,
    Delete,
    Rename(usize),
}

#[derive(Clone, Debug)]
struct RawChange {
    path: usize,
    label: usize,
    kind: ChangeKind,
    content: u8,
    mode: usize,
}

#[derive(Clone, Debug)]
struct RawMerge {
    view: Option<usize>,
    preserve: bool,
    native: usize,
}

#[derive(Clone, Debug)]
struct RawCommit {
    low: Vec<RawChange>,
    high: [Vec<RawChange>; 2],
    merge: Option<RawMerge>,
    present: [bool; 2],
    message: [u8; 2],
}

#[derive(Clone, Debug)]
struct RawRelabel {
    path: usize,
    old: usize,
    new: usize,
    content: Option<u8>,
}

#[derive(Clone, Debug)]
struct RawSet {
    anchor: Option<usize>,
    source: Option<usize>,
    low: Vec<RawRelabel>,
    high: [Vec<RawRelabel>; 2],
    transition: bool,
}

#[derive(Clone, Debug)]
struct RawPolicy {
    default: usize,
    low: Vec<(usize, usize)>,
    high: [Vec<(usize, usize)>; 2],
}

type RawViews = [Option<u8>; 3];

#[derive(Clone, Debug)]
pub(crate) struct Case {
    pub(crate) reader: ViewId,
    versions: Vec<RawViews>,
    commits: Vec<RawCommit>,
    sets: Vec<RawSet>,
    policy: RawPolicy,
}

pub(crate) struct Source {
    pub(crate) graph: SourceGraph,
    pub(crate) sets: Vec<VisibilityChangeSet>,
    pub(crate) current: Views,
    pub(crate) policy: Policy,
    pub(crate) native_details: BTreeMap<String, NativeRequestCommitDetails>,
}

fn change() -> impl Strategy<Value = RawChange> {
    let kind = prop_oneof![
        3 => Just(ChangeKind::Write),
        1 => Just(ChangeKind::Delete),
        1 => (0..6usize).prop_map(ChangeKind::Rename),
    ];
    (0..6usize, 0..LABELS.len(), kind, 0..6u8, 0..MODES.len()).prop_map(
        |(path, label, kind, content, mode)| RawChange {
            path,
            label,
            kind,
            content,
            mode,
        },
    )
}

fn commit() -> impl Strategy<Value = RawCommit> {
    let merge = (
        option::weighted(0.3, 0..LABELS.len()),
        proptest::bool::weighted(0.8),
        1..3usize,
    )
        .prop_map(|(view, preserve, native)| RawMerge {
            view,
            preserve,
            native,
        });
    (
        vec(change(), 0..3),
        [vec(change(), 0..3), vec(change(), 0..3)],
        option::weighted(0.4, merge),
        [any::<bool>(), any::<bool>()],
        [any::<u8>(), any::<u8>()],
    )
        .prop_map(|(low, high, merge, present, message)| RawCommit {
            low,
            high,
            merge,
            present,
            message,
        })
}

fn relabel() -> impl Strategy<Value = RawRelabel> {
    (
        0..6usize,
        0..LABELS.len(),
        0..LABELS.len(),
        option::of(0..6u8),
    )
        .prop_map(|(path, old, new, content)| RawRelabel {
            path,
            old,
            new,
            content,
        })
}

fn set() -> impl Strategy<Value = RawSet> {
    (
        option::of(0..8usize),
        option::weighted(0.3, 0..8usize),
        vec(relabel(), 0..3),
        [vec(relabel(), 0..2), vec(relabel(), 0..2)],
        any::<bool>(),
    )
        .prop_map(|(anchor, source, low, high, transition)| RawSet {
            anchor,
            source,
            low,
            high,
            transition,
        })
}

fn rules(paths: usize) -> impl Strategy<Value = Vec<(usize, usize)>> {
    vec((0..paths, 0..LABELS.len()), 0..3)
}

pub(crate) fn case() -> impl Strategy<Value = Case> {
    let policy = (
        0..LABELS.len(),
        rules(LOW_RULE_PATHS.len()),
        [rules(HIGH_RULE_PATHS.len()), rules(HIGH_RULE_PATHS.len())],
    )
        .prop_map(|(default, low, high)| RawPolicy { default, low, high });
    (
        0..READERS.len(),
        vec(
            [
                option::of(0..16u8),
                option::of(0..16u8),
                option::of(0..16u8),
            ],
            1..4,
        ),
        vec(commit(), 1..7),
        vec(set(), 0..5),
        policy,
    )
        .prop_map(|(reader, versions, commits, sets, policy)| Case {
            reader: id(READERS[reader]),
            versions,
            commits,
            sets,
            policy,
        })
}

fn id(value: &str) -> ViewId {
    ViewId::parse(value).unwrap()
}

fn label(index: usize) -> ViewId {
    id(LABELS[index % LABELS.len()])
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn blob(content: u8, mode: usize) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::blob_sha256(format!("{content}")),
        sha256: format!("sha256-{content}"),
        git_oid: format!("oid-{content}"),
        git_file_mode: MODES[mode % MODES.len()].to_string(),
        size_bytes: u64::from(content),
    }
}

fn views(spec: &RawViews) -> Views {
    let present = |index: usize| spec[index].is_some();
    let includable = ["public", "c0", "c1", "c2"];
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    for (index, mask) in spec.iter().enumerate() {
        let Some(mask) = mask else { continue };
        let includes = includable
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0 && (*bit == 0 || present(bit - 1)))
            .map(|(_, view)| id(view))
            .collect::<BTreeSet<_>>();
        definitions.push(ViewDefinition {
            id: id(&format!("c{index}")),
            name: format!("C{index}"),
            includes: ViewIncludes::Some(includes),
            readers: ViewReaders::Assigned,
        });
    }
    Views::new(definitions).unwrap()
}

struct Labels {
    hidden: Vec<ViewId>,
}

impl Labels {
    fn high(&self, path: &ScopePath, index: usize) -> ViewId {
        if is_private_control_path(path) {
            label(index)
        } else {
            self.hidden[index % self.hidden.len()].clone()
        }
    }
}

fn expand(
    raw: &[RawChange],
    paths: &[&str],
    label_of: impl Fn(&ScopePath, usize) -> ViewId,
) -> Vec<FileChange> {
    let mut seen = BTreeSet::new();
    let mut changes = Vec::new();
    let mut push = |path: ScopePath, label: ViewId, new_content: Option<SourceBlob>| {
        if seen.insert(path.clone()) {
            changes.push(FileChange {
                path,
                old_content: None,
                new_content,
                label,
            });
        }
    };
    for change in raw {
        let from = path(paths[change.path % paths.len()]);
        let written = Some(blob(change.content, change.mode));
        match change.kind {
            ChangeKind::Write => push(from.clone(), label_of(&from, change.label), written),
            ChangeKind::Delete => push(from.clone(), label_of(&from, change.label), None),
            ChangeKind::Rename(to) => {
                let to = path(paths[to % paths.len()]);
                push(from.clone(), label_of(&from, change.label), None);
                push(to.clone(), label_of(&to, change.label), written);
            }
        }
    }
    changes
}

fn relabels(
    raw: &[RawRelabel],
    paths: &[&str],
    label_of: impl Fn(&ScopePath, usize) -> ViewId,
) -> Vec<VisibilityChange> {
    let mut seen = BTreeSet::new();
    raw.iter()
        .filter_map(|change| {
            let path = path(paths[change.path % paths.len()]);
            let (old_label, new_label) = (label_of(&path, change.old), label_of(&path, change.new));
            (old_label != new_label && seen.insert(path.clone())).then(|| VisibilityChange {
                path,
                old_label,
                new_label,
                current_content: change.content.map(|content| blob(content, 0)),
            })
        })
        .collect()
}

impl Case {
    pub(crate) fn build(&self, side: Side) -> Source {
        let commit_count = self.commits.len();
        let mut versions = self.versions.iter().map(views).collect::<Vec<_>>();
        versions.dedup();
        let mut sets = self
            .sets
            .iter()
            .map(|set| (set.anchor.map(|anchor| anchor % commit_count), set))
            .collect::<Vec<_>>();
        sets.sort_by_key(|(anchor, _)| *anchor);
        let mut used = 0;
        let transitions = sets
            .iter()
            .map(|(_, set)| {
                (set.transition && used + 1 < versions.len()).then(|| {
                    used += 1;
                    ViewsTransition {
                        before: versions[used - 1].clone(),
                        after: versions[used].clone(),
                    }
                })
            })
            .collect::<Vec<_>>();
        versions.truncate(used + 1);
        let labels = Labels {
            hidden: LABELS
                .iter()
                .map(|value| id(value))
                .filter(|label| {
                    !versions
                        .iter()
                        .any(|views| views.may_read(&self.reader, label))
                })
                .collect(),
        };
        let sources = sets
            .iter()
            .filter_map(|(_, set)| set.source.map(|source| source % commit_count))
            .collect::<BTreeSet<_>>();
        let anchors = sets
            .iter()
            .filter_map(|(anchor, _)| *anchor)
            .collect::<BTreeSet<_>>();
        let merges_into_reader = |index: usize| {
            self.commits[index]
                .merge
                .as_ref()
                .is_some_and(|merge| merge.view.is_none_or(|view| label(view) == self.reader))
        };
        let named = |index: usize| {
            merges_into_reader(index)
                || !self.commits[index].low.is_empty()
                || sources.contains(&index)
        };
        let fixed =
            |index: usize| source_identity_is_disclosed(named(index), anchors.contains(&index));

        let mut native_details = BTreeMap::new();
        let mut commits = Vec::new();
        for (index, raw) in self.commits.iter().enumerate() {
            if !fixed(index) && !raw.present[side.index()] {
                continue;
            }
            let high_of = |side: Side| {
                expand(&raw.high[side.index()], &HIGH_PATHS, |path, index| {
                    labels.high(path, index)
                })
            };
            let mut high = high_of(side);
            if named(index) {
                high = hidden_change_presence_is_disclosed(high_of(Side::A), high);
            }
            let low = expand(&raw.low, &LOW_PATHS, |_, index| label(index));
            let private_message =
                !high.is_empty() || (raw.low.is_empty() && !merges_into_reader(index));
            let message_side = if private_message { side.index() } else { 0 };
            let commit_id = format!("c{index}");
            let origin = match &raw.merge {
                None => LogicalCommitOrigin::CanonicalPush {
                    source_head_oid: commit_id.clone(),
                },
                Some(merge) => {
                    let native = (0..merge.native)
                        .map(|step| NativeRequestCommit {
                            oid: format!("n{index}-{step}"),
                            parent_oids: vec![if step == 0 {
                                format!("base{index}")
                            } else {
                                format!("n{index}-{}", step - 1)
                            }],
                            tree_oid: format!("t{index}-{step}"),
                            changed_paths: Vec::new(),
                        })
                        .collect::<Vec<_>>();
                    for commit in &native {
                        native_details.insert(
                            commit.oid.clone(),
                            NativeRequestCommitDetails {
                                author: "native".into(),
                                message: format!("native {}", commit.oid),
                                occurred_at_unix: 0,
                                changes: Vec::new(),
                            },
                        );
                    }
                    LogicalCommitOrigin::RequestMerge {
                        request_id: format!("r{index}"),
                        view: merge.view.map_or_else(|| self.reader.clone(), label),
                        base_oid: format!("base{index}"),
                        parent_oids: vec![format!("base{index}")],
                        request_head_oid: format!("head{index}"),
                        preserve_commits: merge.preserve,
                        commits: native,
                    }
                }
            };
            let changes = match side {
                Side::A => low.into_iter().chain(high).collect(),
                Side::B => high.into_iter().chain(low).collect(),
            };
            commits.push(LogicalCommit {
                occurred_at_unix: None,
                id: commit_id,
                origin,
                author_id: format!("author-{}", raw.message[message_side] % 3),
                message: format!("message {index}-{}", raw.message[message_side]),
                changes,
            });
        }

        let mut visibility_sets = Vec::new();
        for (position, ((anchor, raw), views)) in sets.iter().zip(transitions).enumerate() {
            let low = relabels(&raw.low, &LOW_PATHS, |_, index| label(index));
            let high = relabels(&raw.high[side.index()], &HIGH_PATHS, |path, index| {
                labels.high(path, index)
            });
            let changes = match side {
                Side::A => low.into_iter().chain(high).collect::<Vec<_>>(),
                Side::B => high.into_iter().chain(low).collect(),
            };
            if changes.is_empty() && views.is_none() {
                continue;
            }
            visibility_sets.push(VisibilityChangeSet {
                occurred_at_unix: None,
                id: format!("s{position}"),
                anchor_commit_id: anchor.map(|anchor| format!("c{anchor}")),
                source_update_id: raw
                    .source
                    .map(|source| format!("c{}", source % commit_count)),
                author_id: "owner".into(),
                changes,
                views,
            });
        }

        let current = versions.last().expect("one views version").clone();
        let mut policy = Policy::new(label(self.policy.default));
        let rules = |raw: &[(usize, usize)], paths: &[&str]| {
            raw.iter()
                .map(|(rule_path, view)| LabelRule {
                    path: path(paths[*rule_path]),
                    view: label(*view),
                })
                .collect::<Vec<_>>()
        };
        let _ = policy.add_rules(rules(&self.policy.low, &LOW_RULE_PATHS));
        let _ = policy.add_rules(rules(&self.policy.high[side.index()], &HIGH_RULE_PATHS));

        Source {
            graph: SourceGraph {
                repo_id: "scope".into(),
                commits,
            },
            sets: visibility_sets,
            current,
            policy,
            native_details,
        }
    }
}

fn source_identity_is_disclosed(named_by_the_projection: bool, anchors_a_set: bool) -> bool {
    named_by_the_projection || anchors_a_set
}

fn hidden_change_presence_is_disclosed(
    reference: Vec<FileChange>,
    mut changes: Vec<FileChange>,
) -> Vec<FileChange> {
    match (reference.is_empty(), changes.is_empty()) {
        (true, false) => changes.clear(),
        (false, true) => changes = reference,
        _ => {}
    }
    changes
}
