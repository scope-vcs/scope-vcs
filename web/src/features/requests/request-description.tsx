import { Button } from '@/components/ui/button'
import { Check, Pencil } from 'lucide-react'
import { useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { RequestAttachmentEditor } from './request-attachment-editor'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'

/**
 * Edits in place: the rendered text becomes the editor at the same height, and
 * the edit control and the editor's actions render into `actionsSlot`, which
 * the page places at the end of the request tab row.
 */
export function RequestDescription({
  actionsSlot,
  canEdit,
  description,
  onSave,
}: {
  actionsSlot: HTMLElement | null
  canEdit: boolean
  description: string
  onSave: (description: string, expectedDescription: string) => Promise<boolean>
}) {
  const [editorMinHeight, setEditorMinHeight] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const renderedRef = useRef<HTMLDivElement>(null)
  const editing = editorMinHeight !== null

  return (
    <section className="min-w-0 px-5 pb-5 lg:px-7">
      {canEdit && !editing && actionsSlot ? createPortal(
        <Button
          aria-label="Edit description"
          className="text-muted-foreground"
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
        </Button>,
        actionsSlot,
      ) : null}

      {editing ? (
        <>
          <RequestAttachmentEditor
            actionsSlot={actionsSlot}
            autoFocus
            enterSubmits={false}
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
          {error ? <p className="mt-2 text-sm text-destructive" role="alert">{error}</p> : null}
        </>
      ) : (
        <div ref={renderedRef}>
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
