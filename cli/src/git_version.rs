use anyhow::{Context, bail};
use std::process::Command;

fn minimum_version() -> (u32, u32, u32) {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../dev/tool-versions.json"))
            .expect("valid tool version contract");
    parse_number(
        contract["git"]["version"]
            .as_str()
            .expect("Git version in tool version contract"),
    )
    .expect("valid Git version in tool version contract")
}

fn parse_number(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.split_whitespace().next()?.parse().ok()?,
    ))
}

pub fn supported(version_output: &str) -> bool {
    let Some(number) = version_output.strip_prefix("git version ") else {
        return false;
    };
    parse_number(number).is_some_and(|version| version >= minimum_version())
}

pub fn minimum_version_text() -> String {
    let (major, minor, patch) = minimum_version();
    format!("{major}.{minor}.{patch}")
}

pub fn require_supported() -> anyhow::Result<()> {
    let minimum = minimum_version_text();
    let output = Command::new("git")
        .arg("--version")
        .output()
        .with_context(|| {
            format!("Git {minimum} or newer is required; install Git and add it to PATH")
        })?;
    let actual = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !supported(actual.trim()) {
        let found = if actual.trim().is_empty() {
            "an unrecognized Git version"
        } else {
            actual.trim()
        };
        bail!("Git {minimum} or newer is required; found {}", found);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_minimum_patch_and_native_suffixes() {
        assert!(supported("git version 2.55.0"));
        assert!(supported("git version 2.55.1.windows.1"));
        assert!(supported("git version 2.56.0 (Apple Git-154)"));
        for old_or_malformed in [
            "git version 2.54.9",
            "git version 2.55",
            "git version 2.55.x",
            "git version 2.55x.0",
            "git version 2.55.0x",
            "other version 2.55.0",
        ] {
            assert!(!supported(old_or_malformed), "{old_or_malformed}");
        }
    }
}
