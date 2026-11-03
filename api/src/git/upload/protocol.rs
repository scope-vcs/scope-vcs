use super::{upload_pack_oid, upload_pack_packets};
use crate::error::ApiError;
use axum::http::HeaderMap;
use scope_domain::requests::GitFetchAsk;
use std::process::Command;

const GIT_PROTOCOL_HEADER: &str = "git-protocol";
const LS_REFS_COMMAND: &[u8] = b"command=ls-refs";
const FETCH_COMMAND: &[u8] = b"command=fetch";
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
        let command = packets
            .first()
            .map(|packet| packet.payload.trim_ascii_end());
        match (self, command) {
            (Self::V0, _) | (Self::V2, Some(FETCH_COMMAND)) => {
                return Ok(GitFetchAsk::Transfer {
                    shallow: packets
                        .iter()
                        .any(|packet| packet.payload.starts_with(b"shallow ")),
                    wants: packets
                        .iter()
                        .filter_map(|packet| packet.payload.strip_prefix(b"want "))
                        .map(|object| {
                            upload_pack_oid(object)
                                .map(|oid| String::from_utf8_lossy(&oid).into_owned())
                        })
                        .collect::<Result<_, _>>()?,
                });
            }
            (Self::V2, Some(LS_REFS_COMMAND)) => {}
            (Self::V2, _) => {
                return Err(ApiError::bad_request("unsupported Git protocol v2 command"));
            }
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
    fn version_2_lists_named_refs_transfers_fetches_and_refuses_other_commands() {
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
                wants: vec![want.clone()],
                shallow: false,
            }
        );
        let object_info = request(&[
            "command=object-info",
            "0001",
            "size",
            &format!("oid {want}"),
            "0000",
        ]);
        assert!(v2.upload_pack_ask(&object_info).is_err());
        assert_eq!(
            GitProtocol::of(&HeaderMap::new())
                .upload_pack_ask(&ls_refs)
                .unwrap(),
            GitFetchAsk::Transfer {
                wants: Default::default(),
                shallow: false,
            }
        );
    }
}
