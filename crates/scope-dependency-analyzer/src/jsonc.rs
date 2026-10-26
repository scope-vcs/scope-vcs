use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use std::fmt;

pub(crate) fn has_hash_comment(text: &str) -> bool {
    let mut bytes = text.bytes().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            b'#' => return true,
            b'"' => {
                while let Some(byte) = bytes.next() {
                    match byte {
                        b'\\' => {
                            bytes.next();
                        }
                        b'"' => break,
                        _ => {}
                    }
                }
            }
            b'/' if bytes.peek() == Some(&b'/') => {
                bytes.by_ref().find(|byte| *byte == b'\n');
            }
            b'/' if bytes.peek() == Some(&b'*') => {
                bytes.next();
                let mut previous = 0;
                for byte in bytes.by_ref() {
                    if previous == b'*' && byte == b'/' {
                        break;
                    }
                    previous = byte;
                }
            }
            _ => {}
        }
    }
    false
}

pub(crate) fn ordered_path_mappings(text: &str) -> Option<Vec<(String, Vec<String>)>> {
    let document: Document = serde_json::from_str(text).ok()?;
    Some(document.compiler_options?.paths?.0)
}

#[derive(Deserialize)]
struct Document {
    #[serde(rename = "compilerOptions")]
    compiler_options: Option<CompilerOptions>,
}

#[derive(Deserialize)]
struct CompilerOptions {
    paths: Option<OrderedPaths>,
}

struct OrderedPaths(Vec<(String, Vec<String>)>);

impl<'de> Deserialize<'de> for OrderedPaths {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(OrderedPathsVisitor)
    }
}

struct OrderedPathsVisitor;

impl<'de> Visitor<'de> for OrderedPathsVisitor {
    type Value = OrderedPaths;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a paths object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut mappings: Vec<(String, Vec<String>)> = Vec::new();
        while let Some(pattern) = map.next_key::<String>()? {
            let targets = map.next_value::<Vec<String>>()?;
            match mappings
                .iter_mut()
                .find(|(existing, _)| *existing == pattern)
            {
                Some((_, existing)) => *existing = targets,
                None => mappings.push((pattern, targets)),
            }
        }
        Ok(OrderedPaths(mappings))
    }
}
