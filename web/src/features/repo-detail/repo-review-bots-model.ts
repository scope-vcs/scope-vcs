import type { GitHubReviewBotResponse, GitHubUnlistedReviewBotResponse } from '@/api/types.generated'

const REVIEW_BOTS_LIMIT = 20
const LOGIN_MAX_CHARS = 39
const BOT_SUFFIX = '[bot]'

export function reviewBotLogins(text: string) {
  return text.split(/[\s,]+/).filter((login) => login.length > 0)
}

export function reviewBotsText(bots: GitHubReviewBotResponse[]) {
  return bots.map((bot) => bot.login).join('\n')
}

export function sameReviewBots(logins: string[], bots: GitHubReviewBotResponse[]) {
  return logins.length === bots.length && logins.every((login, index) => login === bots[index]?.login)
}

export function reviewBotsProblem(logins: string[]) {
  if (logins.length > REVIEW_BOTS_LIMIT) {
    return `A repository can list at most ${REVIEW_BOTS_LIMIT} review bots.`
  }
  const seen = new Set<string>()
  for (const login of logins) {
    if (!login.endsWith(BOT_SUFFIX)) {
      return `"${login}" is not a bot. Review bots are GitHub bot logins ending in [bot], like greptile-apps[bot]; people who can write to the repository are imported without being listed.`
    }
    const name = login.slice(0, -BOT_SUFFIX.length)
    if (
      name.length === 0
      || name.length > LOGIN_MAX_CHARS
      || name.startsWith('-')
      || name.endsWith('-')
      || !/^[A-Za-z0-9-]+$/.test(name)
    ) {
      return `"${login}" is not a GitHub bot login. Use a name like greptile-apps[bot].`
    }
    const key = login.toLowerCase()
    if (seen.has(key)) return `${login} is listed twice.`
    seen.add(key)
  }
  return null
}

export function reviewBotsFromText(text: string, saved: GitHubReviewBotResponse[]): GitHubReviewBotResponse[] {
  const required = new Set(saved.filter((bot) => bot.required).map((bot) => bot.login.toLowerCase()))
  return reviewBotLogins(text).map((login) => ({ login, required: required.has(login.toLowerCase()) }))
}

export function withReviewBotRequired(
  bots: GitHubReviewBotResponse[],
  login: string,
  required: boolean,
): GitHubReviewBotResponse[] {
  return bots.map((bot) => bot.login === login ? { ...bot, required } : bot)
}

export function canListReviewBot(bots: GitHubReviewBotResponse[]) {
  return bots.length < REVIEW_BOTS_LIMIT
}

export function withListedReviewBot(bots: GitHubReviewBotResponse[], login: string): GitHubReviewBotResponse[] {
  return [...bots, { login, required: false }]
}

export function draftWithReviewBot(draft: string, login: string) {
  const key = login.toLowerCase()
  if (reviewBotLogins(draft).some((drafted) => drafted.toLowerCase() === key)) return draft
  const kept = draft.trimEnd()
  return kept ? `${kept}\n${login}` : login
}

export function skippedReviewsText(bot: GitHubUnlistedReviewBotResponse) {
  const reviews = bot.review_count === 1 ? '1 review' : `${bot.review_count} reviews`
  const pulls = bot.pull_request_count === 1 ? '1 pull request' : `${bot.pull_request_count} pull requests`
  return `Skipped ${reviews} on ${pulls}`
}
