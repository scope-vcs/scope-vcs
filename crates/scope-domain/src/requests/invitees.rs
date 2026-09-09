use super::{Request, RequestAudience, RequestInvitee, validate_required_id};
use crate::error::DomainError;

pub const REQUEST_ACTIVE_INVITEE_LIMIT: usize = 30;

#[derive(Clone, Debug)]
pub struct AddRequestInviteeInput {
    pub actor_user_id: String,
    pub target_user_id: String,
    pub actor_can_manage_invitees: bool,
    pub target_is_maintainer: bool,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RemoveRequestInviteeInput {
    pub actor_user_id: String,
    pub target_user_id: String,
    pub actor_can_manage_invitees: bool,
}

#[derive(Clone, Debug)]
pub struct LeaveRequestInput {
    pub actor_user_id: String,
    pub actor_can_leave_request: bool,
}

pub fn add_request_invitee(
    request: &Request,
    target_is_invitee: bool,
    active_invitee_count: usize,
    input: AddRequestInviteeInput,
) -> Result<RequestInvitee, DomainError> {
    validate_invitee_request(request)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("invitee user id", &input.target_user_id)?;
    if !input.actor_can_manage_invitees {
        return Err(DomainError::forbidden(
            "request invite management access required",
        ));
    }
    if input.target_user_id == request.author_user_id {
        return Err(DomainError::conflict("request author cannot be an invitee"));
    }
    if input.target_is_maintainer {
        return Err(DomainError::conflict(
            "repo maintainers do not need request invitations",
        ));
    }
    if target_is_invitee {
        return Err(DomainError::conflict(
            "user is already invited to this request",
        ));
    }
    if active_invitee_count >= REQUEST_ACTIVE_INVITEE_LIMIT {
        return Err(DomainError::conflict(format!(
            "request cannot have more than {REQUEST_ACTIVE_INVITEE_LIMIT} active invitees"
        )));
    }
    let invitee = RequestInvitee {
        request_id: request.id.clone(),
        user_id: input.target_user_id,
        invited_by_user_id: input.actor_user_id,
        created_at_unix: input.now_unix,
    };
    Ok(invitee)
}

pub fn remove_request_invitee(
    request: &Request,
    invitee: Option<RequestInvitee>,
    input: RemoveRequestInviteeInput,
) -> Result<RequestInvitee, DomainError> {
    validate_invitee_request(request)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("invitee user id", &input.target_user_id)?;
    if !input.actor_can_manage_invitees {
        return Err(DomainError::forbidden(
            "request invite management access required",
        ));
    }
    invitee
        .filter(|invitee| {
            invitee.request_id == request.id && invitee.user_id == input.target_user_id
        })
        .ok_or_else(|| DomainError::not_found("request invitee not found"))
}

pub fn leave_request(
    request: &Request,
    invitee: Option<RequestInvitee>,
    input: LeaveRequestInput,
) -> Result<RequestInvitee, DomainError> {
    validate_invitee_request(request)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    if !input.actor_can_leave_request {
        return Err(DomainError::forbidden("request leave access required"));
    }
    invitee
        .filter(|invitee| {
            invitee.request_id == request.id && invitee.user_id == input.actor_user_id
        })
        .ok_or_else(|| DomainError::not_found("request invitee not found"))
}

fn validate_invitee_request(request: &Request) -> Result<(), DomainError> {
    if request.audience != RequestAudience::Public {
        return Err(DomainError::conflict(
            "private requests do not support invitees",
        ));
    }
    if request.is_terminal() {
        return Err(DomainError::conflict("request is closed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests_tests::open_request;

    #[test]
    fn invitee_facts_preserve_duplicate_limit_and_authorization_order() {
        let request = open_request();
        let mut input = AddRequestInviteeInput {
            actor_user_id: request.author_user_id.clone(),
            target_user_id: "guest".to_string(),
            actor_can_manage_invitees: false,
            target_is_maintainer: false,
            now_unix: 21,
        };
        assert_eq!(
            add_request_invitee(&request, true, REQUEST_ACTIVE_INVITEE_LIMIT, input.clone())
                .unwrap_err()
                .kind,
            crate::error::DomainErrorKind::Forbidden
        );
        input.actor_can_manage_invitees = true;
        assert_eq!(
            add_request_invitee(&request, true, REQUEST_ACTIVE_INVITEE_LIMIT, input.clone())
                .unwrap_err()
                .message,
            "user is already invited to this request"
        );
        assert!(
            add_request_invitee(&request, false, REQUEST_ACTIVE_INVITEE_LIMIT, input.clone())
                .is_err()
        );
        let invitee =
            add_request_invitee(&request, false, REQUEST_ACTIVE_INVITEE_LIMIT - 1, input).unwrap();
        let leave = LeaveRequestInput {
            actor_user_id: "guest".to_string(),
            actor_can_leave_request: true,
        };
        assert_eq!(
            leave_request(&request, Some(invitee.clone()), leave.clone()).unwrap(),
            invitee
        );
        assert_eq!(
            leave_request(&request, None, leave).unwrap_err().kind,
            crate::error::DomainErrorKind::NotFound
        );
    }
}
