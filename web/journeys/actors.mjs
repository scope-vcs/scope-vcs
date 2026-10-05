import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { promisify } from 'node:util'
import { clerk, clerkSetup, setupClerkTestingToken } from '@clerk/testing/playwright'
import { baseUrl } from '../smoke/browser-smoke.mjs'

const run = promisify(execFile)

export const apiUrl = required('SCOPE_API_URL').replace(/\/$/, '')
const scopeCli = required('SCOPE_CLI')
const clerkApi = 'https://api.clerk.com/v1'
const webDefaultTokenTemplate = 'scope_api'
const COLD_DEV_SERVER_TIMEOUT_MS = 120_000
const CLERK_TEST_EMAIL_VERIFICATION_CODE = '424242'

export const collaborators = {
  contributor: { handle: 'river-contributor', email: 'river.contributor+clerk_test@example.com' },
  maintainer: { handle: 'maya-maintainer', email: 'maya.maintainer+clerk_test@example.com' },
}

function required(name) {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required for the contribution journey`)
  return value
}

export async function provisionClerkUsers() {
  process.loadEnvFile('.env.local')
  const secretKey = process.env.CLERK_SECRET_KEY
  if (!secretKey?.startsWith('sk_test_')) {
    throw new Error('setup: CLERK_SECRET_KEY must be a Clerk development secret key (sk_test_*)')
  }
  await clerkSetup({ dotenv: false })
  const backend = async (path, init = {}) => {
    const response = await fetch(`${clerkApi}${path}`, {
      ...init,
      headers: { authorization: `Bearer ${secretKey}`, 'content-type': 'application/json' },
    })
    const body = await response.json()
    if (!response.ok) {
      throw new Error(`setup: Clerk ${init.method ?? 'GET'} ${path} returned ${response.status}: ${JSON.stringify(body.errors ?? body)}`)
    }
    return body
  }

  const templates = await backend('/jwt_templates')
  const template = templates.find(({ name }) => name === webDefaultTokenTemplate)
  if (!template || !('email' in template.claims) || !('email_verified' in template.claims)) {
    throw new Error(`setup: the Clerk instance needs a ${webDefaultTokenTemplate} JWT template with email and email_verified claims`)
  }

  for (const { email } of Object.values(collaborators)) {
    const existing = await backend(`/users?email_address=${encodeURIComponent(email)}`)
    if (existing.length > 0) continue
    await backend('/users', {
      method: 'POST',
      body: JSON.stringify({
        email_address: [email],
        skip_password_requirement: true,
        skip_legal_checks: true,
      }),
    })
  }
}

async function newSession(browser, contextOptions) {
  const context = await browser.newContext(contextOptions)
  await setupClerkTestingToken({ context })
  const page = await context.newPage()
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))
  return { context, page, pageErrors }
}

export async function closeSession({ context }) {
  await context.unrouteAll({ behavior: 'ignoreErrors' })
  await context.close()
}

async function expectSignedIn(page, { handle, email }) {
  await page.waitForFunction(() => window.Clerk?.user)
  const token = await page.evaluate((template) => window.Clerk.session.getToken({ template }), webDefaultTokenTemplate)
  const session = await apiFetch(token, '/v1/session')
  assert.equal(session.user?.handle, handle, `${email} did not resolve to ${handle}`)
}

export async function signIn(browser, collaborator) {
  const session = await newSession(browser)
  await session.page.goto(`${baseUrl}/`, { timeout: COLD_DEV_SERVER_TIMEOUT_MS })
  await session.page.waitForFunction(() => window.Clerk?.loaded, null, { timeout: COLD_DEV_SERVER_TIMEOUT_MS })
  const failures = []
  session.page.on('response', (response) => {
    if (response.url().includes('clerk') && response.status() >= 400) {
      failures.push(`${response.status()} ${new URL(response.url()).pathname}`)
    }
  })
  try {
    await clerk.signIn({ page: session.page, emailAddress: collaborator.email })
  } catch (error) {
    throw new Error(`${error.message} at ${session.page.url()}; failed Clerk calls: ${failures.join(', ') || 'none'}`)
  }
  await expectSignedIn(session.page, collaborator)
  return session
}

export async function signInThroughForm(browser, collaborator, contextOptions) {
  const session = await newSession(browser, contextOptions)
  const { page } = session
  await page.goto(`${baseUrl}/sign-in`)
  await page.getByRole('textbox', { name: 'Email address', exact: true }).fill(collaborator.email)
  const codeSent = page.waitForResponse((response) => response.url().includes('/prepare_first_factor') && response.ok())
  await page.getByRole('button', { name: 'Continue', exact: true }).click()
  await codeSent
  await page.getByRole('textbox', { name: 'Enter verification code' }).fill(CLERK_TEST_EMAIL_VERIFICATION_CODE)
  await expectSignedIn(page, collaborator)
  return session
}

export async function apiFetch(token, path) {
  const response = await fetch(`${apiUrl}${path}`, { headers: { authorization: `Bearer ${token}` } })
  assert.equal(response.status, 200, `GET ${path} returned ${response.status}: ${await response.clone().text()}`)
  return response.json()
}

export async function devSessionToken(handle) {
  const response = await fetch(`${apiUrl}/v1/dev/cli-session/${handle}`, { method: 'POST' })
  assert.equal(response.status, 200, `setup: dev CLI session for ${handle} returned ${response.status}`)
  return (await response.json()).session_token
}

export async function cliActor(workspace, { handle }) {
  const token = await devSessionToken(handle)
  const config = join(workspace, handle, 'config')
  const sessions = join(config, 'scope/sessions')
  await mkdir(sessions, { recursive: true })
  await writeFile(join(sessions, `cli-session-${Buffer.from(apiUrl).toString('hex')}`), token)
  const repo = join(workspace, handle, 'repo')
  const env = {
    ...process.env,
    SCOPE_API_URL: apiUrl,
    XDG_CONFIG_HOME: config,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: join(config, 'gitconfig'),
    PATH: `${dirname(scopeCli)}:${process.env.PATH}`,
  }
  const actor = {
    repo,
    token,
    async scope(...args) {
      const { stdout } = await run(scopeCli, ['--json', ...args], { cwd: repo, env })
      return JSON.parse(stdout).result
    },
    async git(...args) {
      const { stdout } = await run('git', args, { cwd: repo, env })
      return stdout.trim()
    },
    async commit(message) {
      await actor.git('add', '--all')
      await actor.git('-c', 'user.email=scope@example.test', '-c', 'user.name=Scope Journey', 'commit', '-m', message)
    },
  }
  await run(scopeCli, ['--json', 'clone', 'dev/update-demo', repo], { cwd: dirname(repo), env })
  return actor
}
