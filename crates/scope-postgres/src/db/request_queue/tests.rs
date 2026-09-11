use super::*;
use crate::db::requests::tests::postgres_store;

const NOW: u64 = 100;
const VIEWER: &str = "viewer";
const OTHER: &str = "other";

#[tokio::test]
async fn queue_placement_sql_matches_domain_for_fact_combinations() {
    let states = [
        RequestState::Draft,
        RequestState::Open,
        RequestState::Closed,
        RequestState::Merged,
    ];
    let attention = [
        None,
        Some((RequestAttentionState::Active, None)),
        Some((RequestAttentionState::Waiting, None)),
        Some((RequestAttentionState::Settled, None)),
        Some((RequestAttentionState::Snoozed, None)),
        Some((RequestAttentionState::Snoozed, Some(NOW - 1))),
        Some((RequestAttentionState::Snoozed, Some(NOW))),
        Some((RequestAttentionState::Snoozed, Some(NOW + 1))),
    ];
    let claimers = [None, Some(VIEWER), Some(OTHER)];
    let mut rows = String::new();
    let mut expected = Vec::new();

    for state in states {
        for viewer_is_maintainer in [false, true] {
            for viewer_is_author in [false, true] {
                for viewer_is_invitee in [false, true] {
                    for attention_facts in attention {
                        let (attention_state, snoozed_until_unix) = attention_facts
                            .map_or((None, None), |(state, until)| (Some(state), until));
                        for claimer in claimers {
                            let id = expected.len();
                            let author = if viewer_is_author { VIEWER } else { OTHER };
                            let attention = attention_state.map(|state| RequestAttention {
                                request_id: "request".into(),
                                user_id: VIEWER.into(),
                                state,
                                reason: RequestAttentionReason::NewActivity,
                                through_activity_version: 0,
                                snoozed_until_unix,
                                updated_at_unix: 0,
                            });
                            let claim = claimer.map(|claimer| RequestClaim {
                                request_id: "request".into(),
                                claimer_user_id: claimer.into(),
                                claimed_at_unix: 0,
                                updated_at_unix: 0,
                            });
                            expected.push(
                                classify_request_queue_item(RequestQueueFacts {
                                    request_state: state,
                                    request_activity_version: 0,
                                    request_author_user_id: author,
                                    viewer_user_id: Some(VIEWER),
                                    viewer_is_maintainer,
                                    viewer_is_invitee,
                                    attention: attention.as_ref(),
                                    claim: claim.as_ref(),
                                    now_unix: NOW,
                                })
                                .section,
                            );

                            if id > 0 {
                                rows.push(',');
                            }
                            let (submitted, closed, merged) = state_columns(state);
                            write!(
                                rows,
                                "({id}, {submitted}, {closed}, {merged}, '{author}', '{VIEWER}', \
                                 {viewer_is_maintainer}, {viewer_is_invitee}, {attention}, \
                                 {snooze}, {claimer}, {NOW})",
                                attention = sql_text(attention_state.map(attention_state_name)),
                                snooze = sql_u64(snoozed_until_unix),
                                claimer = sql_text(claimer),
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
    }

    let placement = queue_placement_sql()
        .replace("$2", "viewer_user_id")
        .replace("$3", "viewer_is_maintainer")
        .replace("$6", "now_unix");
    let sql = format!(
        "WITH facts(case_id, submitted_at_unix, closed_at_unix, merged_at_unix, \
         author_user_id, viewer_user_id, viewer_is_maintainer, viewer_is_invitee, \
         attention_state, snoozed_until_unix, claimer_user_id, now_unix) AS (VALUES {rows}) \
         SELECT case_id, {placement} AS queue_section FROM facts ORDER BY case_id"
    );
    let actual = postgres_store()
        .db
        .query_all(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap();

    assert_eq!(actual.len(), expected.len());
    for (id, row) in actual.into_iter().enumerate() {
        let actual: String = row.try_get("", "queue_section").unwrap();
        assert_eq!(actual, expected[id].as_str(), "fact combination {id}");
    }
}

fn state_columns(state: RequestState) -> (&'static str, &'static str, &'static str) {
    match state {
        RequestState::Draft => ("NULL", "NULL", "NULL"),
        RequestState::Open => ("1", "NULL", "NULL"),
        RequestState::Closed => ("1", "2", "NULL"),
        RequestState::Merged => ("1", "NULL", "2"),
    }
}

fn attention_state_name(state: RequestAttentionState) -> &'static str {
    match state {
        RequestAttentionState::Active => "active",
        RequestAttentionState::Waiting => "waiting",
        RequestAttentionState::Snoozed => "snoozed",
        RequestAttentionState::Settled => "settled",
    }
}

fn sql_text(value: Option<&str>) -> String {
    value.map_or_else(|| "NULL".into(), |value| format!("'{value}'"))
}

fn sql_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "NULL".into(), |value| value.to_string())
}
