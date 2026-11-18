export async function reportIssue({ github, owner, repo, title, body }) {
  const issues = await github.paginate(github.rest.issues.listForRepo, { owner, repo, state: 'open', per_page: 100 });
  const existing = issues.find(issue => !issue.pull_request && issue.title === title);
  if (existing) return github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
  return github.rest.issues.create({ owner, repo, title, body, labels: ['bug'] });
}
