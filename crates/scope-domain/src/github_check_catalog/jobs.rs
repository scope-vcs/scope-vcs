use super::{GitHubWorkflowFile, WorkflowValue};

const REUSABLE_WORKFLOW_DEPTH_LIMIT: usize = 4;
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

pub(super) fn workflow_jobs(
    document: &WorkflowValue,
    path: &str,
    files: &[GitHubWorkflowFile],
) -> WorkflowJobs {
    let mut result = WorkflowJobs {
        checks: Vec::new(),
        runtime_named: Vec::new(),
    };
    for (id, job) in jobs(document) {
        match job_checks(id, job, files, &mut vec![path]) {
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
        let called = called_checks(uses, files, callers)?;
        return Some(
            called
                .into_iter()
                .map(|check| JobCheck {
                    name: format!("{display} / {}", check.name),
                    conditional: conditional || check.conditional,
                })
                .collect(),
        );
    }
    let combinations = match matrix {
        Some(matrix) => combinations(matrix)?,
        None => vec![Vec::new()],
    };
    combinations
        .iter()
        .map(|combination| {
            check_name(&display, combination).map(|name| JobCheck { name, conditional })
        })
        .collect()
}

fn called_checks<'a>(
    uses: &WorkflowValue,
    files: &'a [GitHubWorkflowFile],
    callers: &mut Vec<&'a str>,
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
    let checks = jobs(document)
        .iter()
        .map(|(id, job)| job_checks(id, job, files, callers))
        .collect::<Option<Vec<_>>>();
    callers.pop();
    Some(checks?.into_iter().flatten().collect())
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
        (true, Some(include)) => entry_list(include),
        (true, None) => Some(vec![Vec::new()]),
        (false, None) => {
            let excluded = match exclude {
                Some(exclude) => entry_list(exclude)?,
                None => Vec::new(),
            };
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

fn scalar_list(value: &WorkflowValue) -> Option<Vec<String>> {
    value
        .items()?
        .iter()
        .map(WorkflowValue::scalar_text)
        .collect()
}

fn entry_list(value: &WorkflowValue) -> Option<Vec<Combination>> {
    value
        .items()?
        .iter()
        .map(|item| {
            item.entries()?
                .iter()
                .map(|(key, value)| value.scalar_text().map(|text| (key.clone(), text)))
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
