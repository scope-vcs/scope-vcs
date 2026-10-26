use serde::{
    Deserializer,
    de::{IgnoredAny, MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
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

pub(crate) fn ordered_path_mappings(document: &str) -> Option<Vec<(String, Vec<String>)>> {
    let compiler_options = last_field(document, "compilerOptions")?;
    let paths = last_field(compiler_options.get(), "paths")?;
    let mut deserializer = serde_json::Deserializer::from_str(paths.get());
    deserializer.deserialize_map(OrderedPaths).ok()
}

fn last_field<'a>(object: &'a str, key: &str) -> Option<&'a RawValue> {
    let mut deserializer = serde_json::Deserializer::from_str(object);
    deserializer.deserialize_map(LastField(key)).ok()?
}

struct LastField<'k>(&'k str);

impl<'de> Visitor<'de> for LastField<'_> {
    type Value = Option<&'de RawValue>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut found = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == self.0 {
                found = Some(map.next_value::<&RawValue>()?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(found)
    }
}

struct OrderedPaths;

impl<'de> Visitor<'de> for OrderedPaths {
    type Value = Vec<(String, Vec<String>)>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a paths object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut mappings: Vec<(String, Value)> = Vec::new();
        while let Some(pattern) = map.next_key::<String>()? {
            let targets = map.next_value::<Value>()?;
            match mappings
                .iter_mut()
                .find(|(existing, _)| *existing == pattern)
            {
                Some((_, existing)) => *existing = targets,
                None => mappings.push((pattern, targets)),
            }
        }
        Ok(mappings
            .into_iter()
            .map(|(pattern, targets)| {
                let targets = targets
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                (pattern, targets)
            })
            .collect())
    }
}
