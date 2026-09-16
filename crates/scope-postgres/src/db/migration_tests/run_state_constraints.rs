use super::*;
use scope_domain::runs::{attempt::AttemptState, job::RunJobState, run::RunState, step::StepState};
use std::collections::BTreeSet;

/// The states a CHECK definition admits for the `state` column, and every
/// other state it mentions in an invariant. PostgreSQL prints the allowlist
/// and any `= ANY` implication as `(state)::text = ANY ((ARRAY[...])::text[])`;
/// other invariants use `<> ALL (ARRAY[...])` or a scalar
/// `(state)::text <> 'pending'::text`. The allowlist is the `= ANY` set that
/// contains every other state the definition mentions, so an invariant left
/// behind for a retired state fails the parity check instead of hiding in it.
struct ConstraintStates {
    allowed: BTreeSet<String>,
    referenced: BTreeSet<String>,
}

fn constraint_states(definition: &str) -> ConstraintStates {
    let mut any_sets: Vec<BTreeSet<String>> = Vec::new();
    let mut referenced = BTreeSet::new();
    for occurrence in definition.split("(state)::text").skip(1) {
        let comparison = occurrence.trim_start();
        let (literals, is_any) = match comparison.split_once("ARRAY[") {
            Some((operator, array)) => {
                let end = array.find(']').expect("unterminated state array");
                (
                    quoted_literals(&array[..end]),
                    operator.trim_start().starts_with("= ANY"),
                )
            }
            None => (
                comparison
                    .split('\'')
                    .nth(1)
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
                false,
            ),
        };
        assert!(
            !literals.is_empty(),
            "unparsed state comparison: {comparison}"
        );
        if is_any {
            any_sets.push(literals.into_iter().collect());
        } else {
            referenced.extend(literals);
        }
    }
    let allowed = any_sets
        .iter()
        .max_by_key(|set| set.len())
        .unwrap_or_else(|| panic!("no state allowlist in: {definition}"))
        .clone();
    for set in &any_sets {
        referenced.extend(set.iter().cloned());
    }
    ConstraintStates {
        allowed,
        referenced,
    }
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
        let parsed = constraint_states(&definition);
        assert_eq!(
            parsed.allowed,
            states
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>(),
            "{constraint} does not admit exactly the domain states: {definition}"
        );
        assert!(
            parsed.referenced.is_subset(&parsed.allowed),
            "{constraint} invariants mention a state it does not admit: {definition}"
        );
    }
}
