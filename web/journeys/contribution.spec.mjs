import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { after, afterEach, before, test } from 'node:test'
import { chromium } from 'playwright'
import { baseUrl, waitForClientHydration } from '../smoke/browser-smoke.mjs'
import { apiFetch, cliActor, collaborators, provisionClerkUsers, signIn } from './actors.mjs'

// A per-run identifier, so a retry cannot pass on an earlier run's requests.
const runId = `${Date.now().toString(36)}-${process.pid}`
const repoPath = '/dev/update-demo'
const requestApi = (id) => `/v1/repos${repoPath}/requests/${id}`
let browser, workspace
const cli = {}
const web = {}

before(async () => {
  await provisionClerkUsers()
  workspace = await mkdtemp(join(tmpdir(), 'scope-journey-'))
  for (const [role, collaborator] of Object.entries(collaborators)) {
    cli[role] = await cliActor(workspace, collaborator)
  }
  assert.equal(existsSync(join(cli.contributor.repo, 'internal/notes.md')), false)
  assert.equal(existsSync(join(cli.maintainer.repo, 'internal/notes.md')), true)
  assert.equal(
    existsSync(join(cli.maintainer.repo, '.scope/runs/request.yml')),
    false,
    'setup: dev/update-demo already has a request workflow; run ./dev/scope-dev reset, then up --signed-in',
  )
  browser = await chromium.launch({ headless: true })
  for (const [role, collaborator] of Object.entries(collaborators)) {
    web[role] = await signIn(browser, collaborator)
    // A cold Vite dev server takes most of a minute to compile and hydrate the
    // first request page; warm it so the journey's own waits stay short.
    await openRequest(web[role].page, 'req_demo_ready')
    await web[role].page.waitForFunction(() => {
      const start = [...document.querySelectorAll('button')].find((button) => button.textContent.startsWith('Start a discussion'))
      return start && Object.keys(start).some((key) => key.startsWith('__reactProps$'))
    }, null, { timeout: 120_000 })
  }
})

// Screenshots and accessibility snapshots only: traces would record session
// cookies and tokens.
afterEach(async (t) => {
  if (t.passed) return
  const failures = join('.tmp/journey', t.name.replace(/\W+/g, '-'))
  await mkdir(failures, { recursive: true })
  for (const [role, { page }] of Object.entries(web)) {
    await page.screenshot({ path: join(failures, `${role}.png`), fullPage: true })
    await writeFile(join(failures, `${role}.aria.yml`), `${page.url()}\n${await page.locator('body').ariaSnapshot()}`)
  }
})

after(async () => {
  for (const { pageErrors } of Object.values(web)) assert.deepEqual(pageErrors, [])
  await browser?.close()
  if (workspace) await rm(workspace, { recursive: true, force: true })
})

async function submitRequest(name, path, content) {
  const { contributor } = cli
  const started = await contributor.scope('request', 'start', name)
  await writeFile(join(contributor.repo, path), content)
  await contributor.commit(`Add ${path}`)
  const pushed = await contributor.scope('request', 'push')
  await contributor.scope('request', 'submit', '--yes')
  return { id: started.request.id, head: pushed.request.head_oid }
}

function openRequest(page, id) {
  return page.goto(`${baseUrl}${repoPath}/requests/${id}`)
}

// Server-rendered controls ignore clicks until React hydrates them.
async function click(locator) {
  await waitForClientHydration(locator)
  await locator.click()
}

// The request header's lifecycle badge.
function state(page, label) {
  return page.locator('.request-detail-pane').getByText(label, { exact: true }).filter({ visible: true }).waitFor()
}

async function syncMain(actor) {
  await actor.git('switch', 'main')
  await actor.scope('pull')
}

function thread(page, text) {
  return page.getByRole('region', { name: 'Request discussion' }).getByRole('article').filter({ hasText: text })
}

test('a CLI contribution is discussed and merged in the browser', async () => {
  const path = `journey-${runId}.txt`
  const name = `journey-${runId}`
  const { id, head } = await submitRequest(name, path, 'contributed from the CLI\n')
  const { maintainer, contributor } = web

  await maintainer.page.goto(`${baseUrl}${repoPath}/requests`)
  const search = maintainer.page.getByRole('searchbox', { name: 'Search requests' })
  await waitForClientHydration(search)
  await search.fill(name)
  await click(maintainer.page.getByRole('link', { name: new RegExp(`^${name} `) }))
  await maintainer.page.waitForURL(`**${repoPath}/requests/${id}`)

  const question = `Why this file? (${runId})`
  await click(maintainer.page.getByRole('button', { name: /^Start a discussion/ }))
  await maintainer.page.getByRole('textbox', { name: 'Start a new discussion' }).fill(question)
  await maintainer.page.getByRole('button', { name: 'Start discussion' }).click()
  await thread(maintainer.page, question).getByText(collaborators.maintainer.handle).waitFor()

  const answer = `It exercises the journey. (${runId})`
  await openRequest(contributor.page, id)
  const contributorThread = thread(contributor.page, question)
  await click(contributorThread.getByRole('button', { name: 'Reply' }))
  await contributorThread.getByRole('textbox', { name: 'Reply' }).fill(answer)
  await contributorThread.getByRole('button', { name: 'Reply', exact: true }).last().click()
  await thread(contributor.page, answer).getByText(collaborators.contributor.handle).first().waitFor()
  await thread(maintainer.page, question).getByText(answer).waitFor()

  await maintainer.page.getByRole('button', { name: 'Changes', exact: true }).click()
  await maintainer.page.getByRole('dialog', { name: 'Request changes' })
    .getByRole('link', { name: new RegExp(`latest .* ${head.slice(0, 12)}$`) }).click()
  await maintainer.page.getByText(path).first().waitFor()
  await maintainer.page.goBack()
  await click(maintainer.page.getByRole('button', { name: 'Merge', exact: true }))
  await maintainer.page.getByRole('button', { name: 'Merge request' }).click()
  await state(maintainer.page, 'Merged')

  const { request } = await apiFetch(cli.maintainer.token, requestApi(id))
  assert.equal(request.state, 'Merged')
  assert.equal(request.merged_head_oid, head)
  const { discussions } = await apiFetch(cli.contributor.token, `${requestApi(id)}/timeline`)
  assert.deepEqual(
    discussions.map(({ author, body_markdown, latest_replies }) => ({
      author: author.handle,
      body: body_markdown,
      replies: latest_replies.map((reply) => [reply.author.handle, reply.body_markdown]),
    })),
    [{ author: collaborators.maintainer.handle, body: question, replies: [[collaborators.contributor.handle, answer]] }],
  )
  for (const actor of [cli.maintainer, cli.contributor]) {
    await syncMain(actor)
    assert.equal(await readFile(join(actor.repo, path), 'utf8'), 'contributed from the CLI\n')
  }
  assert.equal(await cli.maintainer.git('rev-parse', 'HEAD'), request.merged_main_oid)
  assert.equal(existsSync(join(cli.contributor.repo, 'internal/notes.md')), false)
})

test('only the maintainer approves checks, and pending checks hold the merge', async () => {
  const { maintainer: maintainerCli, contributor: contributorCli } = cli
  await syncMain(maintainerCli)
  await mkdir(join(maintainerCli.repo, '.scope/runs'), { recursive: true })
  await writeFile(join(maintainerCli.repo, '.scope/runs/request.yml'), `name: Request gate
on:
  request: true
container:
  image: ghcr.io/scope/dev-seed-ci@sha256:0000000000000000000000000000000000000000000000000000000000000000
timeout: 5m
jobs:
  verify:
    steps:
      - name: Verify
        run: 'true'
`)
  await maintainerCli.commit('Require checks on contributed revisions')
  await maintainerCli.scope('push', '--main', '--no-review', '--wait')
  await syncMain(contributorCli)

  const name = `journey-checks-${runId}`
  const { id } = await submitRequest(name, `${name}.txt`, 'needs checks\n')
  const { maintainer, contributor } = web

  await openRequest(contributor.page, id)
  await contributor.page.getByText('These checks wait for a maintainer to start them.').waitFor()
  assert.equal(await contributor.page.getByRole('button', { name: 'Approve checks' }).count(), 0)

  await openRequest(maintainer.page, id)
  await click(maintainer.page.getByRole('button', { name: 'Approve checks' }))
  await maintainer.page.getByRole('region', { name: 'Checks' }).getByRole('link', { name: 'queued' }).waitFor()
  await state(maintainer.page, 'Checks running')
  assert.equal(await maintainer.page.getByRole('button', { name: 'Merge', exact: true, disabled: false }).count(), 0)
  const checks = await apiFetch(maintainerCli.token, `${requestApi(id)}/checks`)
  assert.equal(checks.state, 'started')
  assert.equal(checks.mergeability.status, 'ChecksPending')

  await maintainer.page.getByRole('button', { name: 'More request actions' }).click()
  await maintainer.page.getByRole('button', { name: 'Close request' }).click()
  await maintainer.page.getByRole('alertdialog').getByRole('button', { name: 'Close request' }).click()
  await state(maintainer.page, 'Closed')
  const { request } = await apiFetch(maintainerCli.token, requestApi(id))
  assert.equal(request.state, 'Closed')
})
