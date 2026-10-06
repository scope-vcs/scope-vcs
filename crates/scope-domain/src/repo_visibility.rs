use super::{
    policy::ScopePath,
    repo_config::{
        RepoConfig, RepoConfigFileRule, pattern_base_path, pattern_matches_path, pattern_weight,
    },
    repo_control::is_repo_control_pattern,
    views::ViewId,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewLabel {
    View(ViewId),
    Mixed,
}

impl ReviewLabel {
    pub fn combine(self, other: Self) -> Self {
        if self == other { self } else { Self::Mixed }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityNodeKind {
    Root,
    Directory,
    File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToggleResult {
    pub changed: bool,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibilityTarget<'a> {
    pub name: &'a str,
    pub path: &'a str,
    pub kind: VisibilityNodeKind,
    pub reserved: bool,
    pub file_paths_under: Vec<&'a str>,
}

pub fn toggle_visibility_target(
    config: &mut RepoConfig,
    target: VisibilityTarget<'_>,
) -> ToggleResult {
    if target.reserved {
        return ToggleResult {
            changed: false,
            message: ".scope visibility is managed by Scope".to_string(),
        };
    }

    let before = config.files.rules.clone();
    let before_default = config.files.default.clone();
    match target.kind {
        VisibilityNodeKind::Root => {
            config.files.default = next_view(config, &config.files.default.clone());
        }
        VisibilityNodeKind::Directory => {
            let next = next_directory_visibility(config, &target);
            replace_visibility_rules_in_subtree(config, target.path, next.clone());
            upsert_visibility_rule(config, folder_rule_path(target.path), next);
        }
        VisibilityNodeKind::File => {
            if file_path_collides_with_pattern_syntax(target.path) {
                return ToggleResult {
                    changed: false,
                    message: format!(
                        "{} cannot be configured with current pattern syntax",
                        target.name
                    ),
                };
            }
            let current = effective_config_label_for_path(config, target.path);
            remove_same_base_folder_rule(config, target.path);
            upsert_visibility_rule(config, target.path.to_string(), next_view(config, &current));
        }
    }
    canonicalize_visibility_rules(config);

    ToggleResult {
        changed: config.files.rules != before || config.files.default != before_default,
        message: format!(
            "{} set to {}",
            target.name,
            visibility_label(target_visibility(config, &target), config)
        ),
    }
}

pub fn target_visibility(config: &RepoConfig, target: &VisibilityTarget<'_>) -> ReviewLabel {
    if target.kind == VisibilityNodeKind::Root {
        return aggregate_visibility(
            target
                .file_paths_under
                .iter()
                .map(|path| effective_config_label_for_path(config, path)),
            config.files.default.clone(),
        );
    }
    if target.kind == VisibilityNodeKind::File {
        return config_visibility_to_review(effective_config_label_for_path(config, target.path));
    }

    if target.file_paths_under.is_empty() {
        return config_visibility_to_review(effective_config_label_for_path(config, target.path));
    }
    aggregate_visibility(
        target
            .file_paths_under
            .iter()
            .map(|path| effective_config_label_for_path(config, path)),
        config.files.default.clone(),
    )
}

pub fn rule_label(config: &RepoConfig, target: &VisibilityTarget<'_>) -> String {
    if target.reserved {
        return if target.kind == VisibilityNodeKind::File
            && target_visibility(config, target) == ReviewLabel::View(ViewId::public())
        {
            "forced public".to_string()
        } else if target.kind == VisibilityNodeKind::File {
            "forced private".to_string()
        } else {
            "managed by Scope".to_string()
        };
    }
    if target.kind == VisibilityNodeKind::Root {
        return format!("default {}", config_visibility_label(&config.files.default));
    }

    let direct_rule_path = match target.kind {
        VisibilityNodeKind::Root => None,
        VisibilityNodeKind::Directory => Some(folder_rule_path(target.path)),
        VisibilityNodeKind::File => Some(target.path.to_string()),
    };
    if let Some(path) = direct_rule_path
        && config.files.rules.iter().any(|rule| rule.path == path)
    {
        return format!("explicit {path}");
    }

    matching_visibility_rule(config, target.path)
        .map(|rule| format!("inherited {}", rule.path))
        .unwrap_or_else(|| "inherited default".to_string())
}

pub fn visibility_label(label: ReviewLabel, config: &RepoConfig) -> String {
    match label {
        ReviewLabel::View(view) => config
            .views
            .get(&view)
            .map(|definition| definition.name.as_str())
            .unwrap_or(view.as_str())
            .to_string(),
        ReviewLabel::Mixed => "mixed".to_string(),
    }
}

pub fn config_visibility_label(view: &ViewId) -> &str {
    view.as_str()
}

pub fn canonicalize_visibility_rules(config: &mut RepoConfig) {
    let base_visibilities = effective_visibilities_by_rule_base(config);
    let mut rules_by_path = BTreeMap::new();
    for rule in &config.files.rules {
        if is_repo_control_pattern(&rule.path) {
            continue;
        }
        rules_by_path.insert(rule.path.clone(), rule.view.clone());
    }
    config.files.rules = rules_by_path
        .into_iter()
        .map(|(path, visibility)| RepoConfigFileRule {
            path,
            view: visibility,
        })
        .collect();

    while let Some(index) = redundant_rule_index(config) {
        config.files.rules.remove(index);
    }
    restore_rule_base_visibilities(config, &base_visibilities);
    sort_visibility_rules(config, &base_visibilities);
}

fn redundant_rule_index(config: &RepoConfig) -> Option<usize> {
    config
        .files
        .rules
        .iter()
        .enumerate()
        .find_map(|(index, rule)| rule_is_redundant(config, index, rule).then_some(index))
}

fn rule_is_redundant(config: &RepoConfig, index: usize, rule: &RepoConfigFileRule) -> bool {
    let without_rule = |path: &str| {
        ScopePath::parse(path)
            .map(|path| config.label_for_path_skipping_rule(&path, Some(index)))
            .unwrap_or(config.files.default.clone())
    };
    let base = pattern_base_path(&rule.path);
    if without_rule(base) != rule.view {
        return false;
    }

    if !rule.path.ends_with("/**") {
        return true;
    }

    let descendant_probe = format!("{base}/__scope_probe__");
    without_rule(&descendant_probe) == rule.view
}

fn upsert_visibility_rule(config: &mut RepoConfig, path: String, visibility: ViewId) {
    config.files.rules.retain(|rule| rule.path != path);
    config.files.rules.push(RepoConfigFileRule {
        path,
        view: visibility,
    });
}

fn effective_visibilities_by_rule_base(config: &RepoConfig) -> BTreeMap<String, ViewId> {
    config
        .files
        .rules
        .iter()
        .map(|rule| {
            let base = pattern_base_path(&rule.path).to_string();
            let visibility = effective_config_label_for_path(config, &base);
            (base, visibility)
        })
        .collect()
}

fn restore_rule_base_visibilities(
    config: &mut RepoConfig,
    base_visibilities: &BTreeMap<String, ViewId>,
) {
    for (base, visibility) in base_visibilities {
        if effective_config_label_for_path(config, base) != *visibility {
            upsert_visibility_rule(config, base.clone(), visibility.clone());
        }
    }
}

fn sort_visibility_rules(config: &mut RepoConfig, base_visibilities: &BTreeMap<String, ViewId>) {
    config.files.rules.sort_by(|left, right| {
        pattern_base_path(&left.path)
            .cmp(pattern_base_path(&right.path))
            .then_with(|| {
                semantic_sort_rank(left, base_visibilities)
                    .cmp(&semantic_sort_rank(right, base_visibilities))
            })
            .then_with(|| rule_sort_rank(&left.path).cmp(&rule_sort_rank(&right.path)))
            .then_with(|| left.path.cmp(&right.path))
    });
}

fn semantic_sort_rank(
    rule: &RepoConfigFileRule,
    base_visibilities: &BTreeMap<String, ViewId>,
) -> u8 {
    let base = pattern_base_path(&rule.path);
    if base_visibilities.get(base).cloned() == Some(rule.view.clone()) {
        1
    } else {
        0
    }
}

fn rule_sort_rank(path: &str) -> u8 {
    if path.ends_with("/**") { 0 } else { 1 }
}

fn replace_visibility_rules_in_subtree(config: &mut RepoConfig, folder_path: &str, next: ViewId) {
    config.files.rules.retain(|rule| {
        if !pattern_is_inside_subtree(&rule.path, folder_path) {
            return true;
        }

        next.is_public() && rule.view.is_private()
    });
}

fn remove_same_base_folder_rule(config: &mut RepoConfig, file_path: &str) {
    let stale_folder_rule = folder_rule_path(file_path);
    config
        .files
        .rules
        .retain(|rule| rule.path != stale_folder_rule);
}

fn aggregate_visibility(
    visibilities: impl Iterator<Item = ViewId>,
    fallback: ViewId,
) -> ReviewLabel {
    let mut selected: Option<ReviewLabel> = None;
    for visibility in visibilities {
        let visibility = config_visibility_to_review(visibility);
        let combined = match selected {
            Some(previous) => previous.combine(visibility),
            None => visibility,
        };
        if combined == ReviewLabel::Mixed {
            return combined;
        }
        selected = Some(combined);
    }
    selected.unwrap_or_else(|| config_visibility_to_review(fallback))
}

fn effective_config_label_for_path(config: &RepoConfig, path: &str) -> ViewId {
    let Ok(scope_path) = ScopePath::parse(path) else {
        return config.files.default.clone();
    };
    config.label_for_path(&scope_path)
}

fn next_directory_visibility(config: &RepoConfig, target: &VisibilityTarget<'_>) -> ViewId {
    match target_visibility(config, target) {
        ReviewLabel::View(view) => next_view(config, &view),
        ReviewLabel::Mixed => next_view(
            config,
            &effective_config_label_for_path(config, target.path),
        ),
    }
}

fn next_view(config: &RepoConfig, current: &ViewId) -> ViewId {
    let ids = config
        .views
        .iter()
        .map(|definition| &definition.id)
        .collect::<Vec<_>>();
    let index = ids.iter().position(|id| *id == current).unwrap_or(0);
    ids[(index + 1) % ids.len()].clone()
}

fn config_visibility_to_review(view: ViewId) -> ReviewLabel {
    ReviewLabel::View(view)
}

fn matching_visibility_rule<'a>(
    config: &'a RepoConfig,
    path: &str,
) -> Option<&'a RepoConfigFileRule> {
    config
        .files
        .rules
        .iter()
        .filter(|rule| pattern_matches_path(&rule.path, path))
        .max_by_key(|rule| pattern_weight(&rule.path))
}

fn folder_rule_path(path: &str) -> String {
    format!("{path}/**")
}

fn pattern_is_inside_subtree(pattern: &str, folder_path: &str) -> bool {
    let base = pattern_base_path(pattern);
    base == folder_path
        || base
            .strip_prefix(folder_path)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn file_path_collides_with_pattern_syntax(path: &str) -> bool {
    path.ends_with("/**")
}

#[cfg(test)]
#[path = "repo_visibility_tests.rs"]
mod tests;
