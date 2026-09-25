import { Button } from '@/components/ui/button'
import { Check, Pencil } from 'lucide-react'
import { useRef, useState } from 'react'
import { RequestAttachmentEditor } from './request-attachment-editor'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'

/**
 * Edits in place: the rendered text becomes the editor at the same height,
 * with its actions below it. The edit control sits at the text's top right.
 */
export function RequestDescription({
  canEdit,
  description,
  onSave,
}: {
  canEdit: boolean
  description: string
  onSave: (description: string, expectedDescription: string) => Promise<boolean>
}) {
  const [editorMinHeight, setEditorMinHeight] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const renderedRef = useRef<HTMLDivElement>(null)
  const editing = editorMinHeight !== null

  return (
    <section className="relative min-w-0 border-b border-border px-5 pb-5 lg:px-7">
      {canEdit && !editing ? (
        <Button
          aria-label="Edit description"
          className="absolute top-0 right-3 text-muted-foreground lg:right-5"
          onClick={() => {
            setError(null)
            setEditorMinHeight(renderedRef.current?.offsetHeight ?? 0)
          }}
          size="icon-sm"
          title="Edit description"
          type="button"
          variant="ghost"
        >
          <Pencil />
        </Button>
      ) : null}

      {editing ? (
        <RequestAttachmentEditor
          autoFocus
          enterSubmits={false}
          error={error}
          initialText={description}
          label="Request description"
          minHeight={editorMinHeight}
          onCancel={() => setEditorMinHeight(null)}
          onSubmit={async (markdown, baseText) => {
            setError(null)
            try {
              if (await onSave(markdown, baseText ?? description)) {
                setEditorMinHeight(null)
                return true
              }
              setError('The request description could not be saved.')
            } catch (saveError) {
              setError(saveError instanceof Error ? saveError.message : 'The request description could not be saved.')
            }
            return false
          }}
          placeholder="Explain the intent, approach, and how this request was tested."
          submitIcon={<Check className="size-3.5" />}
          submitLabel="Save"
          target="description"
        />
      ) : (
        <div className={canEdit ? 'pr-9' : undefined} ref={renderedRef}>
          {description ? (
            <RequestDiscussionMarkdown source={description} />
          ) : (
            <p className="text-sm leading-6 text-muted-foreground">
              No description yet.
            </p>
          )}
        </div>
      )}
    </section>
  )
}
