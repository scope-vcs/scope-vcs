use anyhow::{Context, bail};
use std::process::Command;

const MINIMUM_VERSION: (u32, u32, u32) = (2, 38, 0);

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
    parse_number(number).is_some_and(|version| version >= MINIMUM_VERSION)
}

pub fn minimum_version_text() -> String {
    let (major, minor, patch) = MINIMUM_VERSION;
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
        assert!(supported("git version 2.38.0"));
        assert!(supported("git version 2.39.5 (Apple Git-154)"));
        assert!(supported("git version 2.55.1.windows.1"));
        assert!(supported("git version 3.0.0"));
        for old_or_malformed in [
            "git version 2.37.7",
            "git version 1.99.0",
            "git version 2.38",
            "git version 2.38.x",
            "git version 2.38x.0",
            "git version 2.38.0x",
            "other version 2.38.0",
        ] {
            assert!(!supported(old_or_malformed), "{old_or_malformed}");
        }
    }
}
