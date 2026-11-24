import type { LinkedRequestResponse } from '@/api/types.generated'
import { Input } from '@/components/ui/input'
import { useId, type ReactNode } from 'react'
import { ANOTHER_REQUEST, NO_REQUEST, type RequestCloseDraft } from './request-close-model'
import {
  REQUEST_CLOSE_REASONS,
  REQUEST_LINK_WORDS,
  requestCloseReasonLabel,
  requestStatusPhrase,
} from './request-labels'

const NO_REQUESTS: readonly LinkedRequestResponse[] = []
const CHOICE_CLASS = 'flex items-baseline gap-2 text-foreground'
const RADIO_CLASS = 'translate-y-px accent-foreground'

export function RequestCloseReasonField({
  draft,
  legend,
  onChange,
  resolvedBy = NO_REQUESTS,
}: {
  draft: RequestCloseDraft
  legend: string
  onChange: (draft: RequestCloseDraft) => void
  resolvedBy?: readonly LinkedRequestResponse[]
}) {
  const reference = (label: string) => (
    <RequestReferenceInput
      label={label}
      onChange={(value) => onChange({ ...draft, reference: value })}
      value={draft.reference}
    />
  )
  const fixedBy = (value: string, children: ReactNode) => (
    <label className={CHOICE_CLASS} key={value}>
      <input
        checked={draft.fixedBy === value}
        className={RADIO_CLASS}
        name="close-fixed-by"
        onChange={() => onChange({ ...draft, fixedBy: value })}
        type="radio"
        value={value}
      />
      {children}
    </label>
  )

  return (
    <>
      <fieldset className="grid gap-1.5">
        <legend className="mb-1.5 text-foreground">{legend}</legend>
        {REQUEST_CLOSE_REASONS.map((option) => (
          <label className={CHOICE_CLASS} key={option}>
            <input
              checked={draft.reason === option}
              className={RADIO_CLASS}
              name="close-reason"
              onChange={() => onChange({ ...draft, reason: option })}
              type="radio"
              value={option}
            />
            <span className="first-letter:uppercase">{requestCloseReasonLabel(option)}</span>
          </label>
        ))}
      </fieldset>
      {draft.reason === 'duplicate' ? reference('Duplicate of which request?') : null}
      {draft.reason === 'completed' && resolvedBy.length === 0
        ? reference('Fixed by which request? You can leave this empty.')
        : null}
      {draft.reason === 'completed' && resolvedBy.length > 0 ? (
        <fieldset className="grid gap-1.5">
          <legend className="mb-1.5 text-foreground first-letter:uppercase">{REQUEST_LINK_WORDS.fixedBy}</legend>
          {resolvedBy.map((request) => fixedBy(request.id, (
            <span className="min-w-0 [overflow-wrap:anywhere]">
              {request.title}{' '}
              <span className="text-muted-foreground">{requestStatusPhrase(request.status)}</span>
            </span>
          )))}
          {fixedBy(ANOTHER_REQUEST, 'Another request')}
          {draft.fixedBy === ANOTHER_REQUEST ? reference('Fixed by which request?') : null}
          {fixedBy(NO_REQUEST, 'No request')}
        </fieldset>
      ) : null}
    </>
  )
}

function RequestReferenceInput({
  label,
  onChange,
  value,
}: {
  label: string
  onChange: (value: string) => void
  value: string
}) {
  const id = useId()
  return (
    <div className="grid gap-1.5">
      <label className="text-foreground" htmlFor={id}>{label}</label>
      <Input
        autoComplete="off"
        className="text-foreground"
        id={id}
        onChange={(event) => onChange(event.target.value)}
        placeholder="gh-142, #142, or a request link"
        spellCheck={false}
        value={value}
      />
    </div>
  )
}
