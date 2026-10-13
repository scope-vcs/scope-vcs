use scope_domain::account::UserAccount;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevSeedUser {
    pub(crate) email: String,
    pub(crate) handle: String,
}

pub(crate) const DEV_SEED_USER_ID: &str = "scope_usr_dev_seed";

pub(crate) fn seed_user_account(seed_user: DevSeedUser) -> UserAccount {
    UserAccount {
        id: DEV_SEED_USER_ID.to_string(),
        handle: seed_user.handle,
        email: seed_user.email,
        email_verified: true,
    }
}
