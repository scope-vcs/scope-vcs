use super::{upload_pack_packets, wanted_heads};
use crate::error::ApiError;
use axum::http::HeaderMap;
use scope_domain::requests::GitFetchAsk;
use std::process::Command;

const GIT_PROTOCOL_HEADER: &str = "git-protocol";
const LS_REFS_COMMAND: &[u8] = b"command=ls-refs";
const REF_PREFIX: &[u8] = b"ref-prefix ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitProtocol {
    V0,
    V2,
}

impl GitProtocol {
    pub(crate) fn of(headers: &HeaderMap) -> Self {
        let v2 = headers
            .get_all(GIT_PROTOCOL_HEADER)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(':'))
            .any(|parameter| parameter.trim() == "version=2");
        if v2 { Self::V2 } else { Self::V0 }
    }

    pub(crate) fn configure(self, command: &mut Command) {
        if self == Self::V2 {
            command.env("GIT_PROTOCOL", "version=2");
        }
    }

    pub(crate) fn upload_pack_ask(self, request: &[u8]) -> Result<GitFetchAsk, ApiError> {
        let packets = upload_pack_packets(request)?;
        let lists_refs = self == Self::V2
            && packets
                .first()
                .is_some_and(|packet| packet.payload.trim_ascii_end() == LS_REFS_COMMAND);
        if !lists_refs {
            return Ok(GitFetchAsk::Transfer {
                wants: wanted_heads(request)?,
            });
        }
        let named_refs = packets
            .iter()
            .filter_map(|packet| packet.payload.strip_prefix(REF_PREFIX))
            .map(|prefix| {
                std::str::from_utf8(prefix.trim_ascii_end())
                    .map(str::to_string)
                    .map_err(|_| ApiError::bad_request("malformed Git ref prefix"))
            })
            .collect::<Result<_, _>>()?;
        Ok(GitFetchAsk::Listing { named_refs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::upload::pkt_line;

    fn request(lines: &[&str]) -> Vec<u8> {
        let mut body = Vec::new();
        for line in lines {
            match *line {
                "0001" | "0000" => body.extend_from_slice(line.as_bytes()),
                line => body.extend(pkt_line(format!("{line}\n").as_bytes())),
            }
        }
        body
    }

    #[test]
    fn only_version_2_ls_refs_names_refs_and_other_bodies_transfer_their_wants() {
        let ls_refs = request(&[
            "command=ls-refs",
            "agent=git/2.55.0",
            "0001",
            "peel",
            "ref-prefix refs/heads/",
            "ref-prefix refs/heads/fix-parser",
            "0000",
        ]);
        let want = "a".repeat(40);
        let fetch = request(&[
            "command=fetch",
            "0001",
            &format!("want {want}"),
            "done",
            "0000",
        ]);
        let mut headers = HeaderMap::new();
        headers.insert(GIT_PROTOCOL_HEADER, "version=2".parse().unwrap());
        let v2 = GitProtocol::of(&headers);
        assert_eq!(v2, GitProtocol::V2);
        assert_eq!(
            v2.upload_pack_ask(&ls_refs).unwrap(),
            GitFetchAsk::Listing {
                named_refs: [
                    "refs/heads/".to_string(),
                    "refs/heads/fix-parser".to_string()
                ]
                .into()
            }
        );
        assert_eq!(
            v2.upload_pack_ask(&fetch).unwrap(),
            GitFetchAsk::Transfer {
                wants: vec![want.clone()]
            }
        );
        assert_eq!(
            GitProtocol::of(&HeaderMap::new())
                .upload_pack_ask(&ls_refs)
                .unwrap(),
            GitFetchAsk::Transfer {
                wants: Default::default()
            }
        );
    }
}
