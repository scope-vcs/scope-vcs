use super::WorkflowValue;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Repeat {
    Once,
    Optional,
    OneOrMore,
    ZeroOrMore,
}

enum Atom {
    Char(char),
    Class(Vec<(char, char)>),
    Segment,
    Any,
}

impl Atom {
    fn accepts(&self, candidate: char) -> bool {
        match self {
            Self::Char(expected) => *expected == candidate,
            Self::Class(ranges) => ranges
                .iter()
                .any(|(low, high)| (*low..=*high).contains(&candidate)),
            Self::Segment => candidate != '/',
            Self::Any => true,
        }
    }
}

struct Token {
    atom: Atom,
    repeat: Repeat,
}

pub(super) fn filter_values(value: &WorkflowValue) -> Vec<String> {
    match value.items() {
        Some(items) => items
            .iter()
            .filter_map(WorkflowValue::scalar_text)
            .collect(),
        None => value.scalar_text().into_iter().collect(),
    }
}

pub(super) fn includes(filter: &WorkflowValue, branch: &str) -> bool {
    filter_values(filter)
        .iter()
        .fold(false, |included, pattern| match pattern.strip_prefix('!') {
            Some(negated) if glob_matches(negated, branch) => false,
            None if glob_matches(pattern, branch) => true,
            _ => included,
        })
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let tokens = parse(pattern);
    let chars = text.chars().collect::<Vec<_>>();
    let mut matched = vec![vec![false; chars.len() + 1]; tokens.len() + 1];
    matched[tokens.len()][chars.len()] = true;
    for index in (0..tokens.len()).rev() {
        let token = &tokens[index];
        for position in (0..=chars.len()).rev() {
            let consumes = chars
                .get(position)
                .is_some_and(|candidate| token.atom.accepts(*candidate));
            matched[index][position] = match token.repeat {
                Repeat::Once => consumes && matched[index + 1][position + 1],
                Repeat::Optional => {
                    matched[index + 1][position] || (consumes && matched[index + 1][position + 1])
                }
                Repeat::OneOrMore => {
                    consumes && (matched[index + 1][position + 1] || matched[index][position + 1])
                }
                Repeat::ZeroOrMore => {
                    matched[index + 1][position] || (consumes && matched[index][position + 1])
                }
            };
        }
    }
    matched[0][0]
}

fn parse(pattern: &str) -> Vec<Token> {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut tokens = Vec::<Token>::new();
    let mut index = 0;
    while let Some(&current) = chars.get(index) {
        index += 1;
        let atom = match current {
            '*' if chars.get(index) == Some(&'*') => {
                index += 1;
                tokens.push(Token {
                    atom: Atom::Any,
                    repeat: Repeat::ZeroOrMore,
                });
                continue;
            }
            '*' => {
                tokens.push(Token {
                    atom: Atom::Segment,
                    repeat: Repeat::ZeroOrMore,
                });
                continue;
            }
            '?' | '+' => {
                let repeat = if current == '?' {
                    Repeat::Optional
                } else {
                    Repeat::OneOrMore
                };
                if let Some(last) = tokens.last_mut()
                    && last.repeat == Repeat::Once
                {
                    last.repeat = repeat;
                    continue;
                }
                Atom::Char(current)
            }
            '\\' => match chars.get(index) {
                Some(&escaped) => {
                    index += 1;
                    Atom::Char(escaped)
                }
                None => Atom::Char(current),
            },
            '[' => match parse_class(&chars[index..]) {
                Some((ranges, consumed)) => {
                    index += consumed;
                    Atom::Class(ranges)
                }
                None => Atom::Char(current),
            },
            _ => Atom::Char(current),
        };
        tokens.push(Token {
            atom,
            repeat: Repeat::Once,
        });
    }
    tokens
}

fn parse_class(rest: &[char]) -> Option<(Vec<(char, char)>, usize)> {
    let end = rest.iter().position(|candidate| *candidate == ']')?;
    if end == 0 {
        return None;
    }
    let body = &rest[..end];
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(&low) = body.get(index) {
        match (body.get(index + 1), body.get(index + 2)) {
            (Some('-'), Some(&high)) => {
                ranges.push((low, high));
                index += 3;
            }
            _ => {
                ranges.push((low, low));
                index += 1;
            }
        }
    }
    Some((ranges, end + 1))
}

#[cfg(test)]
mod tests {
    use super::{WorkflowValue, glob_matches, includes};

    #[test]
    fn glob_patterns_follow_github_filter_semantics() {
        let cases = [
            ("scope/**", "scope/requests/req_1", true),
            ("scope/*", "scope/requests/req_1", false),
            ("scope/*", "scope/requests", true),
            ("scope/requests/*", "scope/requests/req_1", true),
            ("*", "main", true),
            ("*", "feature/x", false),
            ("**", "feature/x/y", true),
            ("**/req_1", "scope/requests/req_1", true),
            ("scope/**/req_1", "scope/req_1", false),
            ("scope/**req_1", "scope/req_1", true),
            ("mains?", "main", true),
            ("mains?", "mains", true),
            ("mains?", "mainss", false),
            ("ma+in", "main", true),
            ("ma+in", "maaain", true),
            ("ma+in", "min", false),
            ("v[0-9]", "v7", true),
            ("v[0-9]", "va", false),
            ("v[0-9a-c]x", "vbx", true),
            ("v[0-9]+.[0-9]+", "v12.04", true),
            ("feature\\*", "feature*", true),
            ("feature\\*", "featurex", false),
            ("release[", "release[", true),
            ("main", "main2", false),
        ];
        for (pattern, branch, expected) in cases {
            assert_eq!(
                glob_matches(pattern, branch),
                expected,
                "{pattern} against {branch}"
            );
        }
    }

    #[test]
    fn last_matching_pattern_decides_branch_lists() {
        let list = |patterns: &[&str]| {
            WorkflowValue::Sequence(
                patterns
                    .iter()
                    .map(|pattern| WorkflowValue::String((*pattern).to_string()))
                    .collect(),
            )
        };
        let branch = "scope/requests/req_1";
        let cases: [(&[&str], bool); 6] = [
            (&["scope/**", "!scope/requests/*"], false),
            (
                &["scope/**", "!scope/requests/*", "scope/requests/req_1"],
                true,
            ),
            (&["!scope/requests/*", "scope/**"], true),
            (&["!main"], false),
            (&["main", "scope/**"], true),
            (&[], false),
        ];
        for (patterns, expected) in cases {
            assert_eq!(includes(&list(patterns), branch), expected, "{patterns:?}");
        }
        assert!(includes(
            &WorkflowValue::String("scope/**".to_string()),
            branch
        ));
    }
}
