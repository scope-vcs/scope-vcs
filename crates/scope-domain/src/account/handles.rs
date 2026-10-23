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
