mod image;

pub use image::{ImageContextPath, ImageContextPathError};

use crate::{
    policy::{ScopePath, without_hfs_ignored_characters},
    runs::workflow::identity::WorkflowPath,
};

pub const REPO_CONTROL_ROOT: &str = "/.scope";
pub const REPO_CONTROL_PREFIX: &str = "/.scope/";
pub const LEGACY_REPO_RULES_PATH: &str = "/.scope/RULES.md";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoControlPath {
    Rules,
    Workflow(WorkflowPath),
    ImageContext(ImageContextPath),
    Forbidden,
}

pub fn classify_repo_control_path(path: &ScopePath) -> Option<RepoControlPath> {
    if !is_repo_control_path(path) {
        return None;
    }
    if is_repo_rules_path(path) {
        return Some(RepoControlPath::Rules);
    }
    if let Ok(workflow) = WorkflowPath::parse(path.as_str()) {
        return Some(RepoControlPath::Workflow(workflow));
    }
    if let Ok(image_context) = ImageContextPath::parse(path.as_str()) {
        return Some(RepoControlPath::ImageContext(image_context));
    }
    Some(RepoControlPath::Forbidden)
}

pub fn is_repo_control_path(path: &ScopePath) -> bool {
    path.as_str() == REPO_CONTROL_ROOT || path.as_str().starts_with(REPO_CONTROL_PREFIX)
}

pub fn is_repo_rules_path(path: &ScopePath) -> bool {
    path.as_str() == LEGACY_REPO_RULES_PATH
}

pub fn is_request_protected_path(path: &ScopePath) -> bool {
    is_filesystem_alias_of_repo_control_path(path) || is_agent_context_path(path)
}

fn is_filesystem_alias_of_repo_control_path(path: &ScopePath) -> bool {
    path.as_str()
        .trim_start_matches('/')
        .split('/')
        .next()
        .is_some_and(|root| without_hfs_ignored_characters(root).eq_ignore_ascii_case(".scope"))
}

const AGENT_CONTEXT_SEGMENTS: [&str; 16] = [
    ".codex",
    ".claude",
    ".agents",
    ".cursor",
    ".gemini",
    ".windsurf",
    ".clinerules",
    ".cursorrules",
    ".windsurfrules",
    ".mcp.json",
    "AGENTS.md",
    "AGENTS.override.md",
    "CLAUDE.md",
    "CLAUDE.local.md",
    "GEMINI.md",
    "copilot-instructions.md",
];

const AGENT_CONTEXT_SEGMENT_PAIRS: [(&str, &str); 2] =
    [(".github", "instructions"), (".vscode", "mcp.json")];

fn is_agent_context_path(path: &ScopePath) -> bool {
    let segments: Vec<String> = path
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .map(without_hfs_ignored_characters)
        .collect();
    segments.iter().any(|segment| {
        AGENT_CONTEXT_SEGMENTS
            .iter()
            .any(|protected| segment.eq_ignore_ascii_case(protected))
    }) || segments.windows(2).any(|pair| {
        AGENT_CONTEXT_SEGMENT_PAIRS.iter().any(|(parent, child)| {
            pair[0].eq_ignore_ascii_case(parent) && pair[1].eq_ignore_ascii_case(child)
        })
    })
}

pub fn is_private_control_path(path: &ScopePath) -> bool {
    is_repo_control_path(path) && !is_repo_rules_path(path)
}

pub fn is_repo_control_pattern(pattern: &str) -> bool {
    let base = pattern.strip_suffix("/**").unwrap_or(pattern);
    base == "/.scope" || base.starts_with("/.scope/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> ScopePath {
        ScopePath::parse(value).unwrap()
    }

    #[test]
    fn classifier_separates_supported_from_forbidden_control_paths() {
        assert_eq!(classify_repo_control_path(&path("/README.md")), None);
        assert_eq!(
            classify_repo_control_path(&path(LEGACY_REPO_RULES_PATH)),
            Some(RepoControlPath::Rules)
        );
        assert!(matches!(
            classify_repo_control_path(&path("/.scope/runs/test.yml")),
            Some(RepoControlPath::Workflow(workflow)) if workflow.name() == "test"
        ));
        assert!(matches!(
            classify_repo_control_path(&path("/.scope/images/checks/Dockerfile")),
            Some(RepoControlPath::ImageContext(image))
                if image.image_name() == "checks" && image.context_path() == "Dockerfile"
        ));
        for forbidden in [
            "/.scope",
            "/.scope/repo.json",
            "/.scope/runs",
            "/.scope/runs/Test.yml",
            "/.scope/runs/nested/test.yml",
            "/.scope/other.yml",
        ] {
            assert_eq!(
                classify_repo_control_path(&path(forbidden)),
                Some(RepoControlPath::Forbidden),
                "{forbidden}"
            );
        }
    }

    #[test]
    fn rules_are_control_but_not_private_control() {
        let rules = path(LEGACY_REPO_RULES_PATH);
        assert!(is_repo_control_path(&rules));
        assert!(!is_private_control_path(&rules));
    }

    #[test]
    fn narrower_view_requests_cannot_change_native_agent_context() {
        for protected in [
            "/AGENTS.md",
            "/src/AGENTS.md",
            "/AGENTS.override.md",
            "/src/AGENTS.override.md",
            "/CLAUDE.md",
            "/docs/CLAUDE.md",
            "/CLAUDE.local.md",
            "/.codex/config.toml",
            "/.claude/settings.json",
            "/.agents/skills/review/SKILL.md",
            "/.mcp.json",
            "/.SCOPE/RULES.md",
            "/agents.md",
            "/src/Agents.Override.Md",
            "/claude.MD",
            "/.CoDeX/config.toml",
            "/.CLAUDE/settings.json",
            "/.AGENTS/skills/review/SKILL.md",
            "/.MCP.JSON",
            "/packages/app/.claude/settings.json",
            "/packages/app/.agents/skills/review/SKILL.md",
            "/services/api/.codex/config.toml",
            "/packages/app/.mcp.json",
            "/.cursor/rules/review.mdc",
            "/.cursor/mcp.json",
            "/.cursorrules",
            "/apps/web/.cursorrules",
            "/GEMINI.md",
            "/docs/gemini.md",
            "/.gemini/settings.json",
            "/.windsurfrules",
            "/.windsurf/rules/review.md",
            "/.clinerules",
            "/.clinerules/review.md",
            "/.github/copilot-instructions.md",
            "/.github/instructions/rust.instructions.md",
            "/.GitHub/Instructions/rust.instructions.md",
            "/.vscode/mcp.json",
            "/packages/app/.github/instructions/review.instructions.md",
            "/packages/app/.vscode/mcp.json",
            "/.cur\u{200c}sor/rules/review.mdc",
            "/src/AGENTS\u{feff}.md",
            "/.github/instruc\u{200d}tions/review.instructions.md",
            "/.sco\u{202e}pe/RULES.md",
        ] {
            assert!(is_request_protected_path(&path(protected)), "{protected}");
        }
        for ordinary in [
            "/README.md",
            "/src/agent-notes.md",
            "/notes/CLAUDE.txt",
            "/.github/workflows/ci.yml",
            "/.github/ISSUE_TEMPLATE/bug.md",
            "/.vscode/settings.json",
            "/docs/cursor.md",
            "/src/gemini.rs",
            "/src/mcp.json",
            "/docs/instructions/setup.md",
            "/.github/docs/instructions/guide.md",
            "/docs/fam\u{200d}ily.md",
        ] {
            assert!(!is_request_protected_path(&path(ordinary)), "{ordinary}");
        }
    }

    #[test]
    fn scope_control_patterns_are_reserved() {
        for pattern in ["/.scope", "/.scope/**", "/.scope/runs/test.yml"] {
            assert!(is_repo_control_pattern(pattern), "{pattern}");
        }
        for pattern in ["/README.md", "/src/**", "/.scope-notes/**"] {
            assert!(!is_repo_control_pattern(pattern), "{pattern}");
        }
    }
}
