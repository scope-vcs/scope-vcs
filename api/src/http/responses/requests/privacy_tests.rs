use super::*;
use scope_domain::{
    repository::access::{RepositoryAccess, RepositoryActor},
    requests::{RequestBase, RequestEventKind, RequestEventPayload},
    views::ViewId,
};

#[test]
fn activity_preserves_visible_history_without_disclosing_canonical_landing_oids() {
    let main = "a".repeat(40);
    let head = "b".repeat(40);
    let pin = "c".repeat(40);
    let events = [
        (
            RequestEventKind::BaseRetargeted,
            RequestEventPayload::BaseRetargeted {
                old_base: RequestBase::Request {
                    request_id: "parent".into(),
                    revision_id: "parent-revision".into(),
                    head_oid: pin.clone(),
                },
                landed_head_oid: head.clone(),
                main_oid: main.clone(),
            },
        ),
        (
            RequestEventKind::Merged,
            RequestEventPayload::Merged {
                head_oid: head.clone(),
                main_oid: main.clone(),
            },
        ),
        (
            RequestEventKind::AutoMergeFulfilled,
            RequestEventPayload::AutoMergeFulfilled {
                intent_id: "intent".into(),
                revision_id: "revision".into(),
                head_oid: head.clone(),
                main_oid: main.clone(),
            },
        ),
    ];
    for (kind, payload) in events {
        let event = RequestEvent {
            id: "event".into(),
            request_id: "child".into(),
            position: 5,
            actor_user_id: None,
            kind,
            payload,
            created_at_unix: 10,
        };
        for (access, expected_main) in [
            (RepositoryAccess::public(), serde_json::Value::Null),
            (
                RepositoryAccess {
                    actor: RepositoryActor::Member,
                    view: ViewId::private(),
                    ..RepositoryAccess::public()
                },
                serde_json::Value::String(main.clone()),
            ),
        ] {
            let response =
                request_event_response(event.clone(), None, &access, &Views::builtin()).unwrap();
            let json = serde_json::to_value(response).unwrap();
            let payload = &json["payload"][format!("{kind:?}")];
            assert_eq!(payload["main_oid"], expected_main);
            assert!(payload.to_string().contains(&head));
            if !access.reads_full_view(&Views::builtin()) {
                assert!(!json.to_string().contains(&main));
            }
            if kind == RequestEventKind::BaseRetargeted {
                assert_eq!(payload["old_base"]["request_id"], "parent");
                assert_eq!(payload["old_base"]["head_oid"], pin);
            }
        }
    }
}
