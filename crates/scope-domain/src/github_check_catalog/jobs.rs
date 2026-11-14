use super::{GitHubWorkflowFile, WorkflowValue};

const REUSABLE_WORKFLOW_DEPTH_LIMIT: usize = 4;
const GITHUB_MATRIX_JOB_LIMIT: usize = 256;
const CATALOG_CHECK_LIMIT: usize = 10_000;
const CATALOG_NAME_BYTE_LIMIT: usize = 1024 * 1024;
const EXPRESSION_OPEN: &str = "${{";
const EXPRESSION_CLOSE: &str = "}}";

type Combination = Vec<(String, String)>;

pub(super) struct JobCheck {
    pub(super) name: String,
    pub(super) conditional: bool,
}

pub(super) struct WorkflowJobs {
    pub(super) checks: Vec<JobCheck>,
    pub(super) runtime_named: Vec<String>,
}

pub(super) struct CheckBudget {
    checks: usize,
    name_bytes: usize,
}

impl Default for CheckBudget {
    fn default() -> Self {
        Self {
            checks: CATALOG_CHECK_LIMIT,
            name_bytes: CATALOG_NAME_BYTE_LIMIT,
        }
    }
}

impl CheckBudget {
    fn spend_checks(&mut self, checks: usize) -> Option<()> {
        self.checks = self.checks.checked_sub(checks)?;
        Some(())
    }

    fn spend_name(&mut self, bytes: usize) -> Option<()> {
        self.name_bytes = self.name_bytes.checked_sub(bytes)?;
        Some(())
    }
}

pub(super) fn workflow_jobs(
    document: &WorkflowValue,
    path: &str,
    files: &[GitHubWorkflowFile],
    budget: &mut CheckBudget,
) -> WorkflowJobs {
    let mut result = WorkflowJobs {
        checks: Vec::new(),
        runtime_named: Vec::new(),
    };
    for (id, job) in jobs(document) {
        match job_checks(id, job, files, &mut vec![path], budget) {
            Some(checks) => result.checks.extend(checks),
            None => result.runtime_named.push(id.clone()),
        }
    }
    result
}

fn jobs(document: &WorkflowValue) -> &[(String, WorkflowValue)] {
    document
        .get("jobs")
        .and_then(WorkflowValue::entries)
        .unwrap_or_default()
}

fn job_checks<'a>(
    id: &str,
    job: &WorkflowValue,
    files: &'a [GitHubWorkflowFile],
    callers: &mut Vec<&'a str>,
    budget: &mut CheckBudget,
) -> Option<Vec<JobCheck>> {
    let display = job
        .get("name")
        .and_then(WorkflowValue::scalar_text)
        .unwrap_or_else(|| id.to_string());
    let conditional = job.get("if").is_some();
    let matrix = job
        .get("strategy")
        .and_then(|strategy| strategy.get("matrix"));
    if let Some(uses) = job.get("uses") {
        if matrix.is_some() || display.contains(EXPRESSION_OPEN) {
            return None;
        }
        let called = called_checks(uses, files, callers, budget)?;
        return called
            .into_iter()
            .map(|check| {
                budget.spend_name(display.len() + " / ".len() + check.name.len())?;
                Some(JobCheck {
                    name: format!("{display} / {}", check.name),
                    conditional: conditional || check.conditional,
                })
            })
            .collect();
    }
    let combinations = match matrix {
        Some(matrix) => combinations(matrix)?,
        None => vec![Vec::new()],
    };
    budget.spend_checks(combinations.len())?;
    combinations
        .iter()
        .map(|combination| {
            let name = check_name(&display, combination)?;
            budget.spend_name(name.len())?;
            Some(JobCheck { name, conditional })
        })
        .collect()
}

fn called_checks<'a>(
    uses: &WorkflowValue,
    files: &'a [GitHubWorkflowFile],
    callers: &mut Vec<&'a str>,
    budget: &mut CheckBudget,
) -> Option<Vec<JobCheck>> {
    let WorkflowValue::String(uses) = uses else {
        return None;
    };
    let path = uses.strip_prefix("./")?;
    if callers.len() > REUSABLE_WORKFLOW_DEPTH_LIMIT {
        return None;
    }
    let file = files.iter().find(|file| file.path == path)?;
    if callers.contains(&file.path.as_str()) {
        return None;
    }
    let document = file.document.as_ref().ok()?;
    callers.push(&file.path);
    let mut checks = Vec::new();
    let mut complete = true;
    for (id, job) in jobs(document) {
        match job_checks(id, job, files, callers, budget) {
            Some(called) => checks.extend(called),
            None => {
                complete = false;
                break;
            }
        }
    }
    callers.pop();
    complete.then_some(checks)
}

fn combinations(matrix: &WorkflowValue) -> Option<Vec<Combination>> {
    let WorkflowValue::Mapping(entries) = matrix else {
        return None;
    };
    let mut include = None;
    let mut exclude = None;
    let mut dimensions = Vec::new();
    for (key, value) in entries {
        match key.as_str() {
            "include" => include = Some(value),
            "exclude" => exclude = Some(value),
            _ => dimensions.push((key, scalar_list(value)?)),
        }
    }
    match (dimensions.is_empty(), include) {
        (false, Some(_)) => None,
        (true, Some(include)) => {
            entry_list(include).filter(|entries| entries.len() <= GITHUB_MATRIX_JOB_LIMIT)
        }
        (true, None) => Some(vec![Vec::new()]),
        (false, None) => {
            let excluded = match exclude {
                Some(exclude) => entry_list(exclude)?,
                None => Vec::new(),
            };
            let size = dimensions
                .iter()
                .try_fold(1_usize, |size, (_, values)| size.checked_mul(values.len()))
                .filter(|size| *size <= GITHUB_MATRIX_JOB_LIMIT)?;
            let combination_bytes = dimensions
                .iter()
                .map(|(key, values)| key.len() + values.iter().map(String::len).max().unwrap_or(0))
                .sum::<usize>();
            size.checked_mul(combination_bytes)
                .filter(|bytes| *bytes <= CATALOG_NAME_BYTE_LIMIT)?;
            let product =
                dimensions
                    .iter()
                    .fold(vec![Vec::new()], |combinations, (key, values)| {
                        combinations
                            .iter()
                            .flat_map(|combination: &Combination| {
                                values.iter().map(move |value| {
                                    let mut next = combination.clone();
                                    next.push(((*key).clone(), value.clone()));
                                    next
                                })
                            })
                            .collect::<Vec<_>>()
                    });
            Some(
                product
                    .into_iter()
                    .filter(|combination| {
                        !excluded.iter().any(|entry| {
                            entry
                                .iter()
                                .all(|pair| combination.iter().any(|candidate| candidate == pair))
                        })
                    })
                    .collect(),
            )
        }
    }
}

fn literal_text(value: &WorkflowValue) -> Option<String> {
    value
        .scalar_text()
        .filter(|text| !text.contains(EXPRESSION_OPEN))
}

fn scalar_list(value: &WorkflowValue) -> Option<Vec<String>> {
    value.items()?.iter().map(literal_text).collect()
}

fn entry_list(value: &WorkflowValue) -> Option<Vec<Combination>> {
    value
        .items()?
        .iter()
        .map(|item| {
            item.entries()?
                .iter()
                .map(|(key, value)| literal_text(value).map(|text| (key.clone(), text)))
                .collect()
        })
        .collect()
}

fn check_name(display: &str, combination: &[(String, String)]) -> Option<String> {
    if !display.contains(EXPRESSION_OPEN) {
        if combination.is_empty() {
            return Some(display.to_string());
        }
        let values = combination
            .iter()
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>();
        return Some(format!("{display} ({})", values.join(", ")));
    }
    let mut rendered = String::new();
    let mut rest = display;
    while let Some(start) = rest.find(EXPRESSION_OPEN) {
        rendered.push_str(&rest[..start]);
        let expression = &rest[start + EXPRESSION_OPEN.len()..];
        let end = expression.find(EXPRESSION_CLOSE)?;
        let key = expression[..end].trim().strip_prefix("matrix.")?;
        let (_, value) = combination.iter().find(|(candidate, _)| candidate == key)?;
        rendered.push_str(value);
        rest = &expression[end + EXPRESSION_CLOSE.len()..];
    }
    rendered.push_str(rest);
    Some(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(values: &[&str]) -> WorkflowValue {
        WorkflowValue::Sequence(
            values
                .iter()
                .map(|value| WorkflowValue::String((*value).to_string()))
                .collect(),
        )
    }

    #[test]
    fn a_matrix_is_expanded_only_within_the_name_budget() {
        let value = "v".repeat(16 * 1024);
        let shards = (0..256).map(|shard| shard.to_string()).collect::<Vec<_>>();
        let shards = shards.iter().map(String::as_str).collect::<Vec<_>>();
        let large = (0..15)
            .map(|axis_index| (format!("axis{axis_index}"), axis(&[value.as_str()])))
            .chain([("shard".to_string(), axis(&shards))])
            .collect::<Vec<_>>();
        assert!(combinations(&WorkflowValue::Mapping(large)).is_none());

        let small = vec![
            ("os".to_string(), axis(&["linux", "macos"])),
            ("shard".to_string(), axis(&shards[..100])),
        ];
        assert_eq!(
            combinations(&WorkflowValue::Mapping(small)).map(|all| all.len()),
            Some(200)
        );
    }
}
