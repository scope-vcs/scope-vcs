use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    landing_file::RepositoryLandingFile,
    repo_actions::RepoStorageCleanup,
    repository::{
        CatalogError, Repository,
        git::{GitPoolPack, GitSegmentUpload},
        repo_id,
    },
    requests::{
        Request, RequestDiscussion, RequestDiscussionReadState, RequestDiscussionReply,
        RequestEvent, RequestRevision,
    },
    views::ViewId,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct CatalogFixture {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, Repository>,
    pub repository_landing_files: BTreeMap<String, RepositoryLandingFile>,
    pub git_segment_uploads: Vec<GitSegmentUpload>,
    pub git_pool_packs: BTreeMap<String, Vec<GitPoolPack>>,
    pub requests: BTreeMap<String, Request>,
    pub request_revisions: BTreeMap<String, RequestRevision>,
    pub request_discussions: BTreeMap<String, RequestDiscussion>,
    pub request_discussion_replies: BTreeMap<String, RequestDiscussionReply>,
    pub request_discussion_read_states: BTreeMap<String, RequestDiscussionReadState>,
    pub request_events: BTreeMap<String, RequestEvent>,
    pub pending_repo_storage_deletions: Vec<RepoStorageCleanup>,
    pub pending_source_blob_deletions: Vec<SourceBlob>,
}

impl CatalogFixture {
    pub fn repository(&self, owner: &str, name: &str) -> Option<&Repository> {
        self.repositories.get(&repo_id(owner, name))
    }

    pub fn create_repository(
        &mut self,
        owner: &UserAccount,
        name: &str,
        default_view: ViewId,
    ) -> Result<&Repository, CatalogError> {
        let repository = Repository::new(
            owner,
            name,
            default_view,
            format!(
                "repoi_fixture_{}",
                scope_domain::repository::repo_id(&owner.handle, name)
            ),
        )?;
        Ok(self
            .repositories
            .entry(repository.record.id.clone())
            .insert_entry(repository)
            .into_mut())
    }
}
