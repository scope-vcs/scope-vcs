/**
 * Takes the place of a row that was just settled or snoozed, so undo sits
 * where the eye already is instead of in a toast across the screen.
 */
export function RequestUndoStrip({
  label,
  onUndo,
  pending,
}: {
  label: string
  onUndo: () => void
  pending: boolean
}) {
  return (
    <output className="request-workspace-undo flex items-center justify-between gap-3 bg-foreground px-4 py-2 text-[11px] font-medium text-background">
      <span className="min-w-0 truncate">{label}</span>
      <button
        className="shrink-0 underline underline-offset-2 hover:opacity-80 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-background disabled:opacity-50"
        disabled={pending}
        onClick={onUndo}
        type="button"
      >
        Undo
      </button>
    </output>
  )
}
