pub const RESERVED_HANDLES: &[&str] = &[
    "account",
    "api",
    "cli-login",
    "docs",
    "invites",
    "licenses",
    "pricing",
    "repos",
    "settings",
    "sign-in",
    "sign-up",
];

pub fn is_reserved_handle(handle: &str) -> bool {
    RESERVED_HANDLES
        .iter()
        .any(|reserved| handle.eq_ignore_ascii_case(reserved))
}

pub fn normalize_handle(value: &str) -> Option<String> {
    let mut handle = String::new();
    let mut last_was_separator = false;
    for byte in value.trim().bytes() {
        let next = if byte.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(byte.to_ascii_lowercase() as char)
        } else if matches!(byte, b'-' | b'_') && !last_was_separator {
            last_was_separator = true;
            Some('-')
        } else {
            None
        };

        if let Some(next) = next {
            handle.push(next);
        }
    }

    let handle = handle.trim_matches('-').to_string();
    if handle.is_empty() || handle.len() > 40 {
        None
    } else {
        Some(handle)
    }
}

pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_is_case_insensitive_but_does_not_claim_derived_handles() {
        assert!(is_reserved_handle("SETTINGS"));
        assert!(!is_reserved_handle("settings-2"));
        assert!(!is_reserved_handle("adamblumoff"));
    }
}
