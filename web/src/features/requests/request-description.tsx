import { Button } from '@/components/ui/button'
import { Check, Pencil } from 'lucide-react'
import { useState } from 'react'
import { RequestAttachmentEditor } from './request-attachment-editor'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import { REQUEST_DESCRIPTION_CONTENT_CLASS } from './request-content-layout'

export function RequestDescription({
  canEdit,
  description,
  onSave,
}: {
  canEdit: boolean
  description: string
  onSave: (description: string, expectedDescription: string) => Promise<boolean>
}) {
  const [editing, setEditing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  return (
    <section className="min-w-0 px-5 pb-5 lg:px-7">
      {canEdit && !editing ? (
        <div className="flex justify-end">
          <Button
            onClick={() => {
              setError(null)
              setEditing(true)
            }}
            size="sm"
            type="button"
            variant="ghost"
          >
            <Pencil className="size-3.5" />
            Edit
          </Button>
        </div>
      ) : null}

      {editing ? (
        <>
          <RequestAttachmentEditor
            enterSubmits={false}
            initialText={description}
            label="Request description"
            onCancel={() => setEditing(false)}
            onSubmit={async (markdown, baseText) => {
              setError(null)
              if (await onSave(markdown, baseText ?? description)) {
                setEditing(false)
                return true
              }
              setError('The request description could not be saved.')
              return false
            }}
            placeholder="Explain the intent, approach, and how this request was tested."
            submitIcon={<Check className="size-3.5" />}
            submitLabel="Save description"
            target="description"
          />
          {error ? <p className="mt-2 text-sm text-destructive" role="alert">{error}</p> : null}
        </>
      ) : description ? (
        <RequestDiscussionMarkdown
          className={REQUEST_DESCRIPTION_CONTENT_CLASS}
          source={description}
        />
      ) : (
        <p className="text-sm leading-6 text-muted-foreground">
          No description yet.
        </p>
      )}
    </section>
  )
}
