use super::{ChangePairing, change_paths, parse_numstat_entry};
use crate::{error::ApiError, runtime_budgets::RuntimeBudgets};
use scope_domain::requests::FileLineChanges;
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout, truncated_stderr};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::Path as FsPath,
    process::Command,
};

pub(crate) fn stream_request_diff<T: Send + 'static>(
    raw_repo: &FsPath,
    args: &[&str],
    input: Option<Vec<u8>>,
    action: &str,
    consume: impl FnOnce(BufReader<Box<dyn Read + Send>>) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(raw_repo).args(args);
    let git_span = crate::git::command::prepare_git_subprocess(&mut command)?;
    let _entered = git_span.enter();
    let output = run_with_stdout(
        &mut command,
        input,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        action,
        move |stdout, _cancellation| consume(BufReader::new(stdout)),
    )
    .map_err(|error| match error {
        StreamingProcessError::Process(error) => {
            ApiError::infrastructure_unavailable(error.to_string())
        }
        StreamingProcessError::Consumer(error) => error,
    })?;
    crate::git::command::record_git_exit(&git_span, output.status);
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "{action}: {}",
            truncated_stderr(&output.stderr, scope_git_process::STDERR_DIAGNOSTIC_BYTES).trim()
        )));
    }
    Ok(output.value)
}

pub(crate) struct WholeFileLines(BTreeMap<Vec<u8>, FileLineChanges>);

impl WholeFileLines {
    pub(crate) fn take(&mut self, path: &[u8]) -> Result<FileLineChanges, ApiError> {
        self.0
            .remove(path)
            .ok_or_else(|| ApiError::internal_message("renamed file line counts are missing"))
    }
}

pub(crate) fn whole_file_line_counts(
    raw_repo: &FsPath,
    revisions: &[&str],
    paths: BTreeSet<Vec<u8>>,
) -> Result<WholeFileLines, ApiError> {
    if paths.is_empty() {
        return Ok(WholeFileLines(BTreeMap::new()));
    }
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "--numstat",
        "-z",
        ChangePairing::PerPath.git_arg(),
    ];
    args.extend(revisions);
    args.push("--");
    stream_request_diff(
        raw_repo,
        &args,
        None,
        "counting renamed file lines",
        move |mut reader| {
            let mut counts = BTreeMap::new();
            while let Some(field) = read_nul_field_bounded(&mut reader)? {
                let (lines, path) = parse_numstat_entry(&field.bytes)?;
                if !field.truncated && paths.contains(path) {
                    counts.insert(path.to_vec(), lines);
                }
            }
            Ok(WholeFileLines(counts))
        },
    )
}

pub(crate) fn read_numstat_record(
    field: BoundedNulField,
    reader: &mut impl BufRead,
) -> Result<(FileLineChanges, Option<BoundedNulField>, BoundedNulField), ApiError> {
    let (lines, path) = parse_numstat_entry(&field.bytes)?;
    if path.is_empty() {
        let (old_path, path) = change_paths(true, || read_nul_field_bounded(reader))?;
        return Ok((lines, old_path, path));
    }
    let path = BoundedNulField {
        bytes: path.to_vec(),
        truncated: field.truncated,
    };
    Ok((lines, None, path))
}

const MAX_REQUEST_DIFF_FIELD_BYTES: usize = 64 * 1024;

pub(crate) struct BoundedNulField {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

pub(crate) fn read_nul_field_bounded(
    reader: &mut impl BufRead,
) -> Result<Option<BoundedNulField>, ApiError> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().map_err(|error| {
            ApiError::infrastructure_unavailable(format!(
                "reading request identity diff failed: {error}"
            ))
        })?;
        if available.is_empty() {
            if bytes.is_empty() && !truncated {
                return Ok(None);
            }
            return Err(ApiError::internal_message(
                "request identity diff ended inside a field",
            ));
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == 0)
            .map_or(available.len(), |position| position + 1);
        let content =
            &available[..consumed.saturating_sub(usize::from(available[consumed - 1] == 0))];
        let remaining = MAX_REQUEST_DIFF_FIELD_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&content[..content.len().min(remaining)]);
        truncated |= content.len() > remaining;
        let terminated = available[consumed - 1] == 0;
        reader.consume(consumed);
        if terminated {
            return Ok(Some(BoundedNulField { bytes, truncated }));
        }
    }
}
