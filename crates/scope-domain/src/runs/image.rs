use crate::error::DomainError;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PinnedContainerImage {
    image: String,
    #[serde(skip)]
    digest_start: usize,
}

const DIGEST_SEPARATOR: &str = "@sha256:";

impl PinnedContainerImage {
    pub fn parse(image: impl Into<String>) -> Result<Self, DomainError> {
        let image = image.into();
        let Some((repository, digest)) = image.rsplit_once(DIGEST_SEPARATOR) else {
            return Err(DomainError::invalid_input(
                "pinned container image must end in an immutable sha256 digest",
            ));
        };
        if repository.is_empty()
            || repository.contains('@')
            || image.chars().any(char::is_whitespace)
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DomainError::invalid_input(
                "pinned container image is invalid",
            ));
        }
        Ok(Self {
            digest_start: repository.len() + DIGEST_SEPARATOR.len(),
            image: format!(
                "{repository}{DIGEST_SEPARATOR}{}",
                digest.to_ascii_lowercase()
            ),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.image
    }

    pub fn digest(&self) -> &str {
        &self.image[self.digest_start..]
    }
}
