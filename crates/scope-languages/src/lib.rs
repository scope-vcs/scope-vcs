use gengo_language::{Category, Language};
use std::{collections::HashMap, ffi::OsStr, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageShare {
    pub name: &'static str,
    pub color: &'static str,
    pub bytes: u64,
}

pub fn language_breakdown<'a>(
    files: impl IntoIterator<Item = (&'a str, u64)>,
) -> Vec<LanguageShare> {
    let mut bytes_by_language = HashMap::<Language, u64>::new();
    for (path, bytes) in files {
        if bytes == 0 {
            continue;
        }
        if let Some(language) = language_of(path) {
            *bytes_by_language.entry(language).or_default() += bytes;
        }
    }

    let has_code = bytes_by_language.keys().any(is_code);
    let mut shares = bytes_by_language
        .into_iter()
        .filter(|(language, _)| is_code(language) == has_code)
        .map(|(language, bytes)| LanguageShare {
            name: language.name(),
            color: language.hex(),
            bytes,
        })
        .collect::<Vec<_>>();
    shares.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.name.cmp(right.name))
    });
    shares
}

fn language_of(path: &str) -> Option<Language> {
    Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .and_then(language_of_extension)
        .or_else(|| Language::pick(path, &[], 0))
}

fn language_of_extension(extension: &str) -> Option<Language> {
    Some(match extension {
        "cjs" | "mjs" => Language::Javascript,
        "cts" | "mts" => Language::Typescript,
        "cc" | "cxx" | "hxx" => Language::CPlusPlus,
        "m" | "mm" => Language::ObjectiveC,
        "fs" => Language::FSharp,
        "gql" => Language::Graphql,
        "gradle" => Language::Groovy,
        "htm" => Language::Html,
        "mdx" => Language::Markdown,
        "pyi" => Language::Python,
        "r" => Language::R,
        _ => return None,
    })
}

fn is_code(language: &Language) -> bool {
    !matches!(language.category(), Category::Data | Category::Prose)
}

#[cfg(test)]
mod tests {
    use super::{LanguageShare, language_breakdown};

    fn names_and_bytes(files: &[(&str, u64)]) -> Vec<(&'static str, u64)> {
        language_breakdown(files.iter().copied())
            .into_iter()
            .map(|LanguageShare { name, bytes, .. }| (name, bytes))
            .collect()
    }

    #[test]
    fn code_is_ranked_by_size_and_hides_data_and_prose() {
        assert_eq!(
            names_and_bytes(&[
                ("/web/src/app.ts", 100),
                ("/api/src/main.rs", 250),
                ("/api/src/lib.rs", 50),
                ("/include/scope.h", 40),
                ("/scripts/build.mjs", 30),
                ("/ios/App.m", 20),
                ("/README.md", 9_000),
                ("/package.json", 4_000),
                ("/web/public/logo.png", 20_000),
            ]),
            [
                ("Rust", 300),
                ("TypeScript", 100),
                ("C", 40),
                ("JavaScript", 30),
                ("Objective-C", 20)
            ]
        );
    }

    #[test]
    fn data_and_prose_count_when_there_is_no_code() {
        assert_eq!(
            names_and_bytes(&[
                ("/notes/monday.md", 700),
                ("/config.yaml", 20),
                ("/empty.rs", 0),
                ("/photo.jpg", 5_000),
            ]),
            [("Markdown", 700), ("YAML", 20)]
        );
    }
}
