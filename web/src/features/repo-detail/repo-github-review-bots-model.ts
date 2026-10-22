import { REVIEW_REQUEST_COMMENT_MAX_CHARS } from '../../api/github-inputs'

type ReviewRequestCommentDraft = {
  tooLong: boolean
  changed: boolean
}

export function reviewRequestCommentDraft(draft: string, saved: string | null): ReviewRequestCommentDraft {
  const text = draft.trim()
  return {
    tooLong: [...text].length > REVIEW_REQUEST_COMMENT_MAX_CHARS,
    changed: text !== (saved ?? ''),
  }
}

export const REVIEW_REQUEST_COMMENT_HINT = `Keep it to ${REVIEW_REQUEST_COMMENT_MAX_CHARS} characters.`
