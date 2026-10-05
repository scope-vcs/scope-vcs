mod endpoint;
mod error;

pub use endpoint::{EndpointError, ServiceEndpoint};
pub use error::{ErrorKind, INTERNAL_MESSAGE, ServiceError};

#[cfg(feature = "postgres")]
pub use error::postgres_error_kind;
