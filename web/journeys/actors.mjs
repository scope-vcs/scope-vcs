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
// Must match the web's default Clerk API token template.
const tokenTemplate = 'scope_api'

// The seeded collaborators on dev/update-demo. Scope links a Clerk session to
// an existing user by verified email, so these Clerk users become those users.
export const collaborators = {
  contributor: { handle: 'river-contributor', email: 'river.contributor+clerk_test@example.com' },
  maintainer: { handle: 'maya-maintainer', email: 'maya.maintainer+clerk_test@example.com' },
}

function required(name) {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required for the contribution journey`)
  return value
}

/**
 * Creates or reuses the Clerk development users behind the seeded
 * collaborators. Loads web/.env.local and fetches a Clerk testing token.
 */
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
  const template = templates.find(({ name }) => name === tokenTemplate)
  if (!template || !('email' in template.claims) || !('email_verified' in template.claims)) {
    throw new Error(`setup: the Clerk instance needs a ${tokenTemplate} JWT template with email and email_verified claims`)
  }

  for (const { email } of Object.values(collaborators)) {
    const existing = await backend(`/users?email_address=${encodeURIComponent(email)}`)
    if (existing.length > 0) continue
    // Backend-created email addresses are verified.
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

// Clerk's testing-token route logs a warning for each request a closing context cancels.
export async function closeSession({ context }) {
  await context.unrouteAll({ behavior: 'ignoreErrors' })
  await context.close()
}

async function expectSignedIn(page, { handle, email }) {
  await page.waitForFunction(() => window.Clerk?.user)
  const token = await page.evaluate((template) => window.Clerk.session.getToken({ template }), tokenTemplate)
  const session = await apiFetch(token, '/v1/session')
  assert.equal(session.user?.handle, handle, `${email} did not resolve to ${handle}`)
}

/** Signs a collaborator into its own browser context through Clerk. */
export async function signIn(browser, collaborator) {
  const session = await newSession(browser)
  // A cold Vite dev server can take most of a minute to serve Clerk, beyond
  // the sign-in helper's own wait.
  await session.page.goto(`${baseUrl}/`, { timeout: 120_000 })
  await session.page.waitForFunction(() => window.Clerk?.loaded, null, { timeout: 120_000 })
  // The helper's error omits Clerk's response, so record failed Clerk calls.
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

/** Signs in through the real sign-in form with Clerk's fixed test code. */
export async function signInThroughForm(browser, collaborator, contextOptions) {
  const session = await newSession(browser, contextOptions)
  const { page } = session
  await page.goto(`${baseUrl}/sign-in`)
  await page.getByRole('textbox', { name: 'Email address', exact: true }).fill(collaborator.email)
  // The code field renders before Clerk has sent the code, and an early code is refused.
  const codeSent = page.waitForResponse((response) => response.url().includes('/prepare_first_factor') && response.ok())
  await page.getByRole('button', { name: 'Continue', exact: true }).click()
  await codeSent
  // +clerk_test addresses accept this code and never receive mail.
  await page.getByRole('textbox', { name: 'Enter verification code' }).fill('424242')
  await expectSignedIn(page, collaborator)
  return session
}

export async function apiFetch(token, path) {
  const response = await fetch(`${apiUrl}${path}`, { headers: { authorization: `Bearer ${token}` } })
  assert.equal(response.status, 200, `GET ${path} returned ${response.status}: ${await response.clone().text()}`)
  return response.json()
}

/** A CLI session from the local-only dev endpoint. */
export async function devSessionToken(handle) {
  const response = await fetch(`${apiUrl}/v1/dev/cli-session/${handle}`, { method: 'POST' })
  assert.equal(response.status, 200, `setup: dev CLI session for ${handle} returned ${response.status}`)
  return (await response.json()).session_token
}

/**
 * A collaborator driving the real CLI with a session from the local-only dev
 * endpoint, as cli/tests/contribution_flow.rs does.
 */
export async function cliActor(workspace, { handle }) {
  const token = await devSessionToken(handle)
  const config = join(workspace, handle, 'config')
  const sessions = join(config, 'scope/sessions')
  await mkdir(sessions, { recursive: true })
  // Named like scope_cli::auth::session_storage_key.
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
