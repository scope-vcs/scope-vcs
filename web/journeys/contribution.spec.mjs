import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { after, afterEach, before, test } from 'node:test'
import { chromium } from 'playwright'
import { assertNoHorizontalOverflow, baseUrl, waitForClientHydration } from '../smoke/browser-smoke.mjs'
import {
  apiFetch, apiUrl, cliActor, closeSession, collaborators, devSessionToken, provisionClerkUsers, signIn, signInThroughForm,
} from './actors.mjs'

const runId = `${Date.now().toString(36)}-${process.pid}`
const repoPath = '/dev/update-demo'
const requestApi = (id) => `/v1/repos${repoPath}/requests/${id}`
let browser, workspace
const finishedRequests = {}
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
    await warmColdDevServer(web[role].page)
  }
})

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
  for (const session of Object.values(web)) {
    assert.deepEqual(session.pageErrors, [])
    await closeSession(session)
  }
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

async function warmColdDevServer(page) {
  await openRequest(page, 'req_demo_ready')
  await page.waitForFunction(() => {
    const start = [...document.querySelectorAll('button')].find((button) => button.textContent.startsWith('Start a discussion'))
    return start && Object.keys(start).some((key) => key.startsWith('__reactProps$'))
  }, null, { timeout: 120_000 })
}

function openRequest(page, id) {
  return page.goto(`${baseUrl}${repoPath}/requests/${id}`)
}

async function clickAfterHydration(locator) {
  await waitForClientHydration(locator)
  await locator.click()
}

function waitForLifecycleBadge(page, label) {
  return page.locator('.request-detail-pane').getByText(label, { exact: true }).filter({ visible: true }).waitFor()
}

async function syncMain(actor) {
  await actor.git('switch', 'main')
  await actor.scope('pull')
}

function holdLiveUpdates(page) {
  return page.route('**/v1/repos/*/*/events', () => new Promise(() => {}))
}

function thread(page, text) {
  return page.getByRole('region', { name: 'Request discussion' }).getByRole('article').filter({ hasText: text })
}

test('a CLI contribution is discussed and merged in the browser', async () => {
  const path = `journey-${runId}.txt`
  const name = `journey-${runId}`
  const { id, head } = await submitRequest(name, path, 'contributed from the CLI\n')
  finishedRequests.merged = id
  const { maintainer, contributor } = web

  await maintainer.page.goto(`${baseUrl}${repoPath}/requests`)
  const search = maintainer.page.getByRole('searchbox', { name: 'Search requests' })
  await waitForClientHydration(search)
  await search.fill(name)
  await clickAfterHydration(maintainer.page.getByRole('link', { name: new RegExp(`^${name} `) }))
  await maintainer.page.waitForURL(`**${repoPath}/requests/${id}`)

  const question = `Why this file? (${runId})`
  await clickAfterHydration(maintainer.page.getByRole('button', { name: /^Start a discussion/ }))
  await maintainer.page.getByRole('textbox', { name: 'Start a new discussion' }).fill(question)
  await maintainer.page.getByRole('button', { name: 'Start discussion' }).click()
  await thread(maintainer.page, question).getByText(collaborators.maintainer.handle).waitFor()

  const answer = `It exercises the journey. (${runId})`
  await openRequest(contributor.page, id)
  const contributorThread = thread(contributor.page, question)
  await clickAfterHydration(contributorThread.getByRole('button', { name: 'Reply' }))
  await contributorThread.getByRole('textbox', { name: 'Reply' }).fill(answer)
  await contributorThread.getByRole('button', { name: 'Reply', exact: true }).last().click()
  await thread(contributor.page, answer).getByText(collaborators.contributor.handle).first().waitFor()
  await thread(maintainer.page, question).getByText(answer).waitFor()

  await maintainer.page.getByRole('button', { name: 'Changes', exact: true }).click()
  await maintainer.page.getByRole('dialog', { name: 'Request changes' })
    .getByRole('link', { name: new RegExp(`latest .* ${head.slice(0, 12)}$`) }).click()
  await maintainer.page.getByText(path).first().waitFor()
  await maintainer.page.goBack()
  await clickAfterHydration(maintainer.page.getByRole('button', { name: 'Merge', exact: true }))
  await maintainer.page.getByRole('button', { name: 'Merge request' }).click()
  await waitForLifecycleBadge(maintainer.page, 'Merged')

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

test('a merge of a head that moved is refused and the page shows the new head', async () => {
  const name = `journey-stale-${runId}`
  const { id, head } = await submitRequest(name, `${name}.txt`, 'first revision\n')
  const { page } = web.maintainer
  await openRequest(page, id)
  const merge = page.getByRole('button', { name: 'Merge', exact: true })
  await clickAfterHydration(merge)
  const dialog = page.getByRole('alertdialog')
  await dialog.getByText(`${head.slice(0, 12)} → main`).waitFor()

  await writeFile(join(cli.contributor.repo, `${name}.txt`), 'second revision\n')
  await cli.contributor.commit('Revise while the maintainer reviews')
  const { request: { head_oid: newHead } } = await cli.contributor.scope('request', 'push')

  await dialog.getByRole('button', { name: 'Merge request' }).click()
  await page.getByRole('alert').filter({ hasText: 'request has a new revision; review it before merging' }).waitFor()
  await merge.click()
  await dialog.getByText(`${newHead.slice(0, 12)} → main`).waitFor()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  const { request } = await apiFetch(cli.maintainer.token, requestApi(id))
  assert.equal(request.state, 'Open')
  assert.equal(request.head_oid, newHead)
})

test('only the maintainer allows CI, and pending CI holds the merge', async () => {
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
  const { id, head } = await submitRequest(name, `${name}.txt`, 'needs checks\n')
  finishedRequests.closed = id
  const { maintainer, contributor } = web

  await openRequest(contributor.page, id)
  await contributor.page.getByText('A maintainer must allow CI to run for this revision.').waitFor()
  assert.equal(await contributor.page.getByRole('button', { name: 'Allow CI to run' }).count(), 0)

  await openRequest(maintainer.page, id)
  await clickAfterHydration(maintainer.page.getByRole('button', { name: 'Allow CI to run' }))
  const confirmation = maintainer.page.getByRole('alertdialog', { name: 'Allow CI to run?' })
  await confirmation.getByText(head, { exact: true }).waitFor()
  const beforeApproval = await apiFetch(maintainerCli.token, `${requestApi(id)}/checks`)
  assert.equal(beforeApproval.state, 'awaiting-approval')
  await confirmation.getByRole('button', { name: 'Allow CI to run' }).click()
  await maintainer.page.getByRole('region', { name: 'CI' }).getByRole('link', { name: /queued/ }).waitFor()
  await waitForLifecycleBadge(maintainer.page, 'Waiting for CI')
  assert.equal(await maintainer.page.getByRole('button', { name: 'Merge', exact: true, disabled: false }).count(), 0)
  const checks = await apiFetch(maintainerCli.token, `${requestApi(id)}/checks`)
  assert.equal(checks.state, 'started')
  assert.equal(checks.mergeability.status, 'ChecksPending')

  await maintainer.page.getByRole('button', { name: 'More request actions' }).click()
  await maintainer.page.getByRole('button', { name: 'Close request' }).click()
  await maintainer.page.getByRole('alertdialog').getByRole('button', { name: 'Close request' }).click()
  await waitForLifecycleBadge(maintainer.page, 'Closed')
  const { request } = await apiFetch(maintainerCli.token, requestApi(id))
  assert.equal(request.state, 'Closed')
})

test('the sign-in form signs a collaborator in with a Clerk test code', async () => {
  const session = await signInThroughForm(browser, collaborators.contributor)
  assert.deepEqual(session.pageErrors, [])
  await closeSession(session)
})

test('review controls and completion states fit a narrow screen', async () => {
  const { id } = await submitRequest(`journey-narrow-${runId}`, `journey-narrow-${runId}.txt`, 'narrow\n')
  const viewport = { width: 390, height: 844 }
  const context = await browser.newContext({ storageState: await web.maintainer.context.storageState(), viewport })
  const page = await context.newPage()
  await openRequest(page, id)
  for (const control of [
    page.getByRole('button', { name: 'Allow CI to run' }),
    page.getByRole('button', { name: /^Merge/ }).first(),
  ]) {
    await control.waitFor()
    const { x, y, width, height } = await control.boundingBox()
    assert.ok(
      x >= 0 && y >= 0 && x + width <= viewport.width && y + height <= viewport.height,
      `${await control.textContent()} is off screen`,
    )
  }
  await assertNoHorizontalOverflow(page)
  for (const [label, requestId] of [['Merged', finishedRequests.merged], ['Closed', finishedRequests.closed]]) {
    await openRequest(page, requestId)
    await waitForLifecycleBadge(page, label)
    await assertNoHorizontalOverflow(page)
  }
  await context.close()
})

test('a revoked maintainer cannot allow CI from an open confirmation', async () => {
  const { id } = await submitRequest(`journey-revoked-${runId}`, `journey-revoked-${runId}.txt`, 'revoked\n')
  const { page } = web.maintainer
  await holdLiveUpdates(page)
  await openRequest(page, id)
  const approve = page.getByRole('button', { name: 'Allow CI to run' })
  await clickAfterHydration(approve)
  const confirmation = page.getByRole('alertdialog', { name: 'Allow CI to run?' })
  await confirmation.waitFor()

  const owner = await devSessionToken('dev')
  const revoked = await fetch(`${apiUrl}/v1/repos${repoPath}/members/scope_usr_dev_maintainer`, {
    method: 'DELETE',
    headers: { authorization: `Bearer ${owner}`, 'x-scope-cli-protocol': '1' },
  })
  assert.equal(revoked.ok, true, `member removal returned ${revoked.status}`)

  await confirmation.getByRole('button', { name: 'Allow CI to run' }).click()
  await confirmation.getByRole('alert').filter({ hasText: 'repo maintainer required' }).waitFor()
  const checks = await apiFetch(owner, `${requestApi(id)}/checks`)
  assert.equal(checks.state, 'awaiting-approval')
})
