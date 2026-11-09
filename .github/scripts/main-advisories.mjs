import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export function checkMainAdvisories(run = spawnSync) {
  return ['workspace', 'cli'].map(target => {
    const result = run('./dev/checks/advisories', [target, '--format', 'json'], {
      encoding: 'utf8', maxBuffer: 32 * 1024 * 1024,
    });
    const output = `${result.stdout ?? ''}\n${result.stderr ?? ''}`;
    return { target, status: result.status, output, error: result.error?.message ?? null };
  });
}

export function advisoryIssueBody({ checks, runUrl, sha }) {
  const lines = [`The daily dependency advisory check failed on main at \`${sha}\`.`, '', `[Workflow run](${runUrl})`, ''];
  for (const check of checks) {
    lines.push(`### ${check.target}: ${check.status === 0 ? 'passed' : 'failed'}`, '');
    const findings = new Set();
    for (const line of check.output.split('\n')) {
      let diagnostic;
      try { diagnostic = JSON.parse(line); } catch { continue; }
      const fields = diagnostic.fields;
      if (diagnostic.type !== 'diagnostic' || fields?.severity !== 'error') continue;
      const ids = (fields.notes ?? []).filter(note => note.startsWith('ID: ')).map(note => note.slice(4));
      const crates = (fields.graphs ?? []).filter(graph => graph.Krate).map(graph => `${graph.Krate.name} ${graph.Krate.version}`);
      if (ids.length && crates.length) {
        for (const id of ids) for (const crate of crates) findings.add(`- ${id}: \`${crate}\``);
      } else {
        findings.add(`- ${fields.code ?? 'error'}: ${fields.message}${crates.length ? ` (${crates.join(', ')})` : ''}`);
      }
    }
    lines.push(...findings, '');
    if (check.status !== 0 && !findings.size) {
      lines.push('No advisory IDs were available. The check failed before producing advisory diagnostics.', '',
        '```text', (check.error ?? check.output).slice(-4000).replaceAll('```', "'''"), '```', '');
    }
  }
  if (!checks.length) lines.push('The workflow failed before advisory checks completed. See the run for setup failure details.', '');
  return lines.join('\n');
}

export async function reportMainAdvisories({ github, owner, repo, checks, runUrl, sha }) {
  const title = 'New dependency advisory on main';
  const body = advisoryIssueBody({ checks, runUrl, sha });
  const issues = await github.paginate(github.rest.issues.listForRepo, {
    owner, repo, state: 'open', per_page: 100,
  });
  const existing = issues.find(issue => !issue.pull_request && issue.title === title);
  if (existing) {
    return github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
  }
  return github.rest.issues.create({ owner, repo, title, body, labels: ['bug'] });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const checks = checkMainAdvisories();
  writeFileSync(process.argv[2], JSON.stringify(checks, null, 2));
  for (const check of checks) process.stderr.write(`${check.target}:\n${check.output}\n${check.error ?? ''}\n`);
  if (checks.some(check => check.status !== 0)) process.exitCode = 1;
}
