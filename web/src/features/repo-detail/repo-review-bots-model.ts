import type { GitHubReviewBotResponse } from '@/api/types.generated'

export function reviewBotLogins(text: string) {
  return text.split(/[\s,]+/).filter((login) => login.length > 0)
}

export function reviewBotsText(bots: GitHubReviewBotResponse[]) {
  return bots.map((bot) => bot.login).join(', ')
}

export function sameReviewBots(text: string, bots: GitHubReviewBotResponse[]) {
  const logins = reviewBotLogins(text)
  return logins.length === bots.length && logins.every((login, index) => login === bots[index]?.login)
}
