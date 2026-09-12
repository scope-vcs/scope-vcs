pub(crate) fn required<'a>(name: &str, value: &'a Option<String>) -> anyhow::Result<&'a str> {
    value
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

pub(crate) fn require_exact(
    name: &str,
    actual: Option<&str>,
    expected: &str,
) -> anyhow::Result<()> {
    match actual {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => anyhow::bail!("{name} must be {expected}"),
        None => anyhow::bail!("{name} is required"),
    }
}
