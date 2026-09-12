/// Use the same commit abbreviation throughout human command output.
pub(crate) fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

pub(crate) fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::terminal_text;

    #[test]
    fn terminal_text_replaces_control_characters() {
        assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
    }
}
