const PREVIEW_NAME = /^pr-([1-9][0-9]{0,8})$/;

export function previewPullRequestNumber(name) {
  const match = typeof name === "string" ? PREVIEW_NAME.exec(name) : null;
  return match ? Number(match[1]) : null;
}

export function previewEnvironmentName(pullRequest) {
  const name = `pr-${pullRequest}`;
  if (previewPullRequestNumber(name) === null) throw new Error('A positive pull request number is required.');
  return name;
}

