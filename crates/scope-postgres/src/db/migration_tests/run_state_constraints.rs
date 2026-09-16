use super::*;
use scope_domain::runs::{attempt::AttemptState, job::RunJobState, run::RunState, step::StepState};
use std::collections::BTreeSet;

/// Every state string a CHECK definition compares against the `state` column.
/// PostgreSQL prints either `(state)::text = ANY (ARRAY[...])`, its `<> ALL`
/// form, or a scalar comparison such as `(state)::text <> 'pending'::text`.
fn constraint_state_literals(definition: &str) -> BTreeSet<String> {
    let mut states = BTreeSet::new();
    for occurrence in definition.split("(state)::text").skip(1) {
        let comparison = occurrence
            .trim_start()
            .trim_start_matches(['=', '<', '>'])
            .trim_start();
        let literals = match comparison.split_once("ARRAY[") {
            Some((operator, array))
                if operator.starts_with("ANY (") || operator.starts_with("ALL (") =>
            {
                let end = array.find(']').expect("unterminated state array");
                quoted_literals(&array[..end])
            }
            _ => comparison
                .split('\'')
                .nth(1)
                .map(str::to_string)
                .into_iter()
                .collect(),
        };
        assert!(
            !literals.is_empty(),
            "unparsed state comparison: {comparison}"
        );
        states.extend(literals);
    }
    states
}

fn quoted_literals(text: &str) -> Vec<String> {
    text.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

async fn constraint_definition(db: &DatabaseConnection, name: &str) -> String {
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_get_constraintdef(c.oid) AS definition
         FROM pg_constraint c
         JOIN pg_class r ON r.oid = c.conrelid
         JOIN pg_namespace n ON n.oid = r.relnamespace
         WHERE n.nspname = current_schema() AND c.conname = $1",
        [name.into()],
    ))
    .await
    .unwrap()
    .unwrap_or_else(|| panic!("{name} is missing from the migrated schema"))
    .try_get::<String>("", "definition")
    .unwrap()
}

/// The schema and the domain must enumerate the same run states: a new enum
/// variant needs a migration, and a migrated state needs a variant.
#[tokio::test]
async fn run_state_check_constraints_allow_exactly_the_domain_states() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();

    for (constraint, states) in [
        (
            "scope_runs_values",
            RunState::ALL.map(RunState::as_str).to_vec(),
        ),
        (
            "scope_run_jobs_values",
            RunJobState::ALL.map(RunJobState::as_str).to_vec(),
        ),
        (
            "scope_run_attempts_values",
            AttemptState::ALL.map(AttemptState::as_str).to_vec(),
        ),
        (
            "scope_run_attempt_steps_values",
            StepState::ALL.map(StepState::as_str).to_vec(),
        ),
    ] {
        let definition = constraint_definition(&db, constraint).await;
        assert_eq!(
            constraint_state_literals(&definition),
            states
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>(),
            "{constraint} does not match the domain states: {definition}"
        );
    }
}
