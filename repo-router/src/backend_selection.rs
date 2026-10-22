use axum::http::{Method, Uri};
use std::sync::atomic::{AtomicUsize, Ordering};

const UPLOAD_PACK: &str = "git-upload-pack";
const RECEIVE_PACK: &str = "git-receive-pack";
const UPLOAD_PACK_PATH_SUFFIX: &str = "/git-upload-pack";
const RECEIVE_PACK_PATH_SUFFIX: &str = "/git-receive-pack";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitRequestKind {
    UploadPackRead,
    ReceivePack,
    PrimaryOnly,
}

impl GitRequestKind {
    pub(crate) fn classify(method: &Method, uri: &Uri) -> Self {
        match *method {
            Method::POST if uri.path().ends_with(UPLOAD_PACK_PATH_SUFFIX) => Self::UploadPackRead,
            Method::POST if uri.path().ends_with(RECEIVE_PACK_PATH_SUFFIX) => Self::ReceivePack,
            Method::GET
                if uri.path().ends_with("/info/refs") && has_one_service(uri, UPLOAD_PACK) =>
            {
                Self::UploadPackRead
            }
            Method::GET
                if uri.path().ends_with("/info/refs") && has_one_service(uri, RECEIVE_PACK) =>
            {
                Self::ReceivePack
            }
            _ => Self::PrimaryOnly,
        }
    }
}

fn has_one_service(uri: &Uri, expected: &str) -> bool {
    let mut services = uri.query().into_iter().flat_map(|query| {
        query.split('&').filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == "service").then_some(value)
        })
    });
    matches!(services.next(), Some(value) if value == expected) && services.next().is_none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitRequestPhase {
    Discovery,
    Transfer,
}

impl GitRequestPhase {
    pub(crate) fn of(method: &Method) -> Self {
        if *method == Method::GET {
            Self::Discovery
        } else {
            Self::Transfer
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Discovery => 0,
            Self::Transfer => 1,
        }
    }
}

pub(crate) struct BackendSelector {
    read_replicas: usize,
    write_replicas: usize,
    next_read: [AtomicUsize; 2],
    next_write: [AtomicUsize; 2],
}

impl BackendSelector {
    pub(crate) fn new(read_replicas: usize, write_replicas: usize) -> Self {
        assert!(read_replicas > 0, "read replica count must be positive");
        assert!(write_replicas > 0, "write replica count must be positive");
        Self {
            read_replicas,
            write_replicas,
            next_read: Default::default(),
            next_write: Default::default(),
        }
    }

    pub(crate) fn candidate_indices(
        &self,
        kind: GitRequestKind,
        phase: GitRequestPhase,
        backend_count: usize,
    ) -> Vec<usize> {
        if backend_count == 0 {
            return Vec::new();
        }
        match kind {
            GitRequestKind::PrimaryOnly => return vec![0],
            GitRequestKind::ReceivePack => {
                let eligible = self.write_replicas.min(backend_count);
                return vec![
                    self.next_write[phase.index()].fetch_add(1, Ordering::Relaxed) % eligible,
                ];
            }
            GitRequestKind::UploadPackRead => {}
        }

        let eligible = self.read_replicas.min(backend_count);
        let start = self.next_read[phase.index()].fetch_add(1, Ordering::Relaxed) % eligible;
        (0..eligible)
            .map(|offset| (start + offset) % eligible)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(value: &str) -> Uri {
        value.parse().unwrap()
    }

    #[test]
    fn classifies_upload_pack_reads_and_receive_pack_writes() {
        for (method, uri) in [
            (
                Method::GET,
                uri("/git/public/scope/router/info/refs?service=git-upload-pack"),
            ),
            (
                Method::GET,
                uri("/git/public/scope/router/info/refs?version=2&service=git-upload-pack"),
            ),
            (
                Method::POST,
                uri("/git/permissioned/scope/router/git-upload-pack"),
            ),
        ] {
            assert_eq!(
                GitRequestKind::classify(&method, &uri),
                GitRequestKind::UploadPackRead
            );
        }

        for (method, uri) in [
            (
                Method::GET,
                uri("/git/permissioned/scope/router/info/refs?service=git-receive-pack"),
            ),
            (
                Method::POST,
                uri("/git/permissioned/scope/router/git-receive-pack"),
            ),
        ] {
            assert_eq!(
                GitRequestKind::classify(&method, &uri),
                GitRequestKind::ReceivePack
            );
        }

        for (method, uri) in [
            (
                Method::GET,
                uri(
                    "/git/public/scope/router/info/refs?service=git-upload-pack&service=git-receive-pack",
                ),
            ),
            (Method::GET, uri("/git/public/scope/router/info/refs")),
        ] {
            assert_eq!(
                GitRequestKind::classify(&method, &uri),
                GitRequestKind::PrimaryOnly
            );
        }
    }

    #[test]
    fn sequential_pushes_and_fetches_rotate_their_transfers_across_every_replica() {
        let selector = BackendSelector::new(2, 2);
        let mut push_transfers = Vec::new();
        let mut fetch_transfers = Vec::new();
        for _ in 0..4 {
            selector.candidate_indices(GitRequestKind::ReceivePack, GitRequestPhase::Discovery, 6);
            push_transfers.push(
                selector.candidate_indices(
                    GitRequestKind::ReceivePack,
                    GitRequestPhase::Transfer,
                    6,
                )[0],
            );
            selector.candidate_indices(
                GitRequestKind::UploadPackRead,
                GitRequestPhase::Discovery,
                6,
            );
            fetch_transfers.push(
                selector.candidate_indices(
                    GitRequestKind::UploadPackRead,
                    GitRequestPhase::Transfer,
                    6,
                )[0],
            );
        }
        assert_eq!(push_transfers, vec![0, 1, 0, 1]);
        assert_eq!(fetch_transfers, vec![0, 1, 0, 1]);
    }

    #[test]
    fn receive_pack_rotates_across_write_replicas_and_stays_primary_by_default() {
        let primary = BackendSelector::new(3, 1);
        for _ in 0..4 {
            assert_eq!(
                primary.candidate_indices(
                    GitRequestKind::ReceivePack,
                    GitRequestPhase::Transfer,
                    6
                ),
                vec![0]
            );
        }

        let spread = BackendSelector::new(1, 3);
        let chosen = (0..6)
            .map(|_| {
                spread.candidate_indices(GitRequestKind::ReceivePack, GitRequestPhase::Transfer, 6)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chosen,
            vec![vec![0], vec![1], vec![2], vec![0], vec![1], vec![2]]
        );
        assert_eq!(
            spread.candidate_indices(GitRequestKind::ReceivePack, GitRequestPhase::Transfer, 2),
            vec![0]
        );
        assert_eq!(
            spread.candidate_indices(GitRequestKind::PrimaryOnly, GitRequestPhase::Transfer, 6),
            vec![0]
        );
    }
}
