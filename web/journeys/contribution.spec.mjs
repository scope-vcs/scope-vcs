import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { after, afterEach, before, test } from 'node:test'
import { chromium } from 'playwright'
import { assertNoHorizontalOverflow, baseUrl, waitForClientHydration } from '../smoke/browser-smoke.mjs'
import {
  apiFetch, cliActor, closeSession, collaborators, provisionClerkUsers, signIn, signInThroughForm,
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
  const search = maintainer.page.getByRole('combobox', { name: 'Search requests, history and files' })
  await waitForClientHydration(search)
  await search.fill(name)
  await maintainer.page.getByRole('option', { name: new RegExp(name) }).click()
  await clickAfterHydration(maintainer.page.getByRole('link', { name: 'Open request', exact: true }))
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

test('signed-in private stack navigation preserves the pin across parent extension and hides names from a contributor', async () => {
  const actor = cli.maintainer
  await syncMain(actor)
  const parentName = `journey-stack-parent-${runId}`
  const childName = `journey-stack-child-${runId}`
  const parentTitle = `Private foundation ${runId}`
  const childTitle = `Private follow-up ${runId}`
  const parentPath = `internal/${parentName}.txt`
  const childPath = `internal/${childName}.txt`
  const parent = await actor.scope('request', 'start', parentName, '--view', 'private', '--title', parentTitle)
  const parentId = parent.request.id
  let childId
  const page = web.maintainer.page
  try {
    await writeFile(join(actor.repo, parentPath), 'private foundation\n')
    await actor.commit('Add private stack foundation')
    await actor.scope('request', 'push')
    await actor.scope('request', 'submit', '--yes')
    const child = await actor.scope('request', 'start', childName, '--base', parentId, '--title', childTitle)
    childId = child.request.id
    const pin = child.request.base
    await writeFile(join(actor.repo, childPath), 'private follow-up\n')
    await actor.commit('Add private stack follow-up')
    await actor.scope('request', 'push')
    await actor.scope('request', 'submit', '--yes')

    await openRequest(page, childId)
    const dependency = page.getByRole('region', { name: 'Request dependency' })
    const parentLink = dependency.getByRole('link', { name: parentTitle, exact: true })
    const stackDetails = dependency.getByRole('dialog')
    const pinned = (head) => stackDetails.getByText(`Pinned at ${pin.head_oid.slice(0, 12)} · parent now at ${head.slice(0, 12)}`, { exact: true })
    await clickAfterHydration(dependency.locator('button[aria-haspopup="dialog"]'))
    await pinned(pin.head_oid).waitFor()
    await actor.git('switch', parentName)
    await writeFile(join(actor.repo, parentPath), 'private foundation extended\n')
    await actor.commit('Extend private stack foundation')
    const extended = await actor.scope('request', 'push')
    await pinned(extended.request.head_oid).waitFor()
    await clickAfterHydration(parentLink)
    await page.waitForURL(`**${repoPath}/requests/${parentId}`)
    await clickAfterHydration(dependency.getByRole('link', { name: childTitle, exact: true }))
    await page.waitForURL(`**${repoPath}/requests/${childId}`)
    const primary = page.getByRole('navigation', { name: 'Primary' })
    await primary.getByRole('link', { name: 'Runs', exact: true }).click()
    await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
    await primary.getByRole('link', { name: 'Requests', exact: true }).click()
    await page.waitForURL(`**${repoPath}/requests/${childId}`)
    await clickAfterHydration(dependency.locator('button[aria-haspopup="dialog"]'))
    await pinned(extended.request.head_oid).waitFor()
    await page.keyboard.press('Escape')

    await page.setViewportSize({ width: 390, height: 844 })
    await clickAfterHydration(parentLink)
    await page.waitForURL(`**${repoPath}/requests/${parentId}`)
    await clickAfterHydration(dependency.getByRole('link', { name: childTitle, exact: true }))
    await page.waitForURL(`**${repoPath}/requests/${childId}`)
    await dependency.locator('[aria-current="page"]').filter({ hasText: childTitle }).waitFor()
    await assertNoHorizontalOverflow(page)

    const contributor = web.contributor.page
    for (const id of [parentId, childId]) {
      await openRequest(contributor, id)
      await contributor.getByText('Request not found', { exact: true }).waitFor()
      assert.equal(await contributor.getByText(parentTitle, { exact: true }).count(), 0)
      assert.equal(await contributor.getByText(childTitle, { exact: true }).count(), 0)
      assert.equal(await contributor.getByRole('region', { name: 'Request dependency' }).count(), 0)
    }
    await contributor.goto(`${baseUrl}${repoPath}/requests`)
    const search = contributor.getByRole('combobox', { name: 'Search requests, history and files' })
    await waitForClientHydration(search)
    await search.click()
    await contributor.getByRole('radio', { name: 'Requests', exact: true }).click()
    await search.fill(`journey-stack`)
    await contributor.getByText('No matches.', { exact: true }).waitFor()
    assert.equal(await contributor.getByText(parentTitle, { exact: true }).count(), 0)
    assert.equal(await contributor.getByText(childTitle, { exact: true }).count(), 0)
  } finally {
    await page.setViewportSize({ width: 1280, height: 900 })
    if (childId) await actor.scope('request', 'close', '--request', childId, '--yes')
    await actor.scope('request', 'close', '--request', parentId, '--yes')
    await syncMain(actor)
  }
})

test('a maintainer closes a contributed request in the browser', async () => {
  const name = `journey-close-${runId}`
  const { id } = await submitRequest(name, `${name}.txt`, 'close this\n')
  finishedRequests.closed = id
  const { maintainer } = web
  const maintainerCli = cli.maintainer

  await openRequest(maintainer.page, id)
  await clickAfterHydration(maintainer.page.getByRole('button', { name: 'More request actions' }))
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
  const merge = page.getByRole('button', { name: 'Merge', exact: true })
  await merge.waitFor()
  const { x, y, width, height } = await merge.boundingBox()
  assert.ok(
    x >= 0 && y >= 0 && x + width <= viewport.width && y + height <= viewport.height,
    'Merge is off screen',
  )
  await assertNoHorizontalOverflow(page)
  for (const [label, requestId] of [['Merged', finishedRequests.merged], ['Closed', finishedRequests.closed]]) {
    await openRequest(page, requestId)
    await waitForLifecycleBadge(page, label)
    await assertNoHorizontalOverflow(page)
  }
  await context.close()
})
