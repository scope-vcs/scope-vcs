import { Button } from '@/components/ui/button'
import { Calendar } from '@/components/ui/calendar'
import { Input } from '@/components/ui/input'
import { Popover } from '@/components/ui/popover'
import { Select } from '@/components/ui/select'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import * as Dialog from '@radix-ui/react-dialog'
import { CalendarIcon, Minus, Plus } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import {
  localSnoozeDate,
  localSnoozeTime,
  resolveCustomSnooze,
  type CustomSnoozeInput,
} from './request-workspace-model'

const DURATION_UNITS = ['minutes', 'hours', 'days'] as const
const LABEL_CLASS = 'flex min-w-0 flex-col gap-1.5 text-sm font-medium'
const HOUR_MS = 3_600_000

export function RequestCustomSnoozeDialog({
  onClose,
  onSnooze,
}: {
  onClose: () => void
  onSnooze: (until: number) => void
}) {
  const id = useId()
  const [initial] = useState(() => new Date(Date.now() + HOUR_MS))
  const [mode, setMode] = useState<CustomSnoozeInput['mode']>('date')
  const [date, setDate] = useState(initial)
  const [time, setTime] = useState(localSnoozeTime(initial))
  const [amount, setAmount] = useState('2')
  const [unit, setUnit] = useState<(typeof DURATION_UNITS)[number]>('hours')
  const [error, setError] = useState<string | null>(null)

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const input: CustomSnoozeInput =
      mode === 'date' ? { mode, date: localSnoozeDate(date), time } : { mode, amount, unit }
    const until = resolveCustomSnooze(input, new Date())
    if (until === null) {
      setError(mode === 'date' ? 'Choose a valid date and time in the future.' : 'Enter a positive duration.')
      return
    }
    onClose()
    onSnooze(until)
  }

  function stepAmount(step: number) {
    setAmount(String(Math.max(0, Number(amount) + step)))
    setError(null)
  }

  return (
    <Dialog.Root
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      open
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm" />
        <Dialog.Content className="fixed top-1/2 left-1/2 z-50 grid w-[calc(100%-2rem)] max-w-sm -translate-x-1/2 -translate-y-1/2 gap-5 content-start [@media(max-height:760px)]:top-4 [@media(max-height:760px)]:h-[calc(100dvh-2rem)] [@media(max-height:760px)]:translate-y-0 [@media(max-height:760px)]:overflow-y-auto rounded-lg border border-[var(--border-strong)] bg-popover p-6 text-popover-foreground shadow-[var(--shadow-pop)] outline-none">
          <div className="grid gap-1.5">
            <Dialog.Title className="text-base font-semibold leading-6">Custom snooze</Dialog.Title>
            <Dialog.Description className="text-sm leading-5 text-muted-foreground">
              Choose when this request returns to your queue.
            </Dialog.Description>
          </div>
          <form className="grid gap-5" onSubmit={submit}>
            <ToggleGroup
              aria-label="Schedule type"
              className="w-full *:flex-1"
              onValueChange={(next) => {
                if (next === 'date' || next === 'duration') setMode(next)
                setError(null)
              }}
              type="single"
              value={mode}
            >
              <ToggleGroupItem value="date">Date and time</ToggleGroupItem>
              <ToggleGroupItem value="duration">Duration</ToggleGroupItem>
            </ToggleGroup>
            {mode === 'date' ? (
              <div className="grid grid-cols-2 gap-3">
                <div className={LABEL_CLASS}>
                  <span id={`${id}-date`}>Date</span>
                  <Popover
                    align="start"
                    className="p-2"
                    label="Choose snooze date"
                    panel={(close) => (
                      <Calendar
                        defaultMonth={date}
                        disabled={{ before: new Date(new Date().setHours(0, 0, 0, 0)) }}
                        mode="single"
                        onSelect={(selected) => {
                          setDate(selected)
                          close()
                          setError(null)
                        }}
                        required
                        selected={date}
                      />
                    )}
                    trigger={(props) => (
                      <Button
                        {...props}
                        aria-labelledby={`${id}-date ${id}-date-value`}
                        className="w-full justify-between"
                        size="sm"
                        type="button"
                        variant="secondary"
                      >
                        <span id={`${id}-date-value`}>
                          {date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}
                        </span>
                        <CalendarIcon aria-hidden="true" className="text-muted-foreground" />
                      </Button>
                    )}
                  />
                </div>
                <label className={LABEL_CLASS}>
                  Time
                  <Input
                    className="h-8 rounded-md"
                    onChange={(event) => {
                      setTime(event.target.value)
                      setError(null)
                    }}
                    required
                    type="time"
                    value={time}
                  />
                </label>
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-3">
                <div className={LABEL_CLASS}>
                  <label htmlFor={`${id}-amount`}>Snooze for</label>
                  <span className="flex items-center gap-1">
                    <Button aria-label="Decrease duration" onClick={() => stepAmount(-1)} size="icon-sm" type="button" variant="ghost">
                      <Minus aria-hidden="true" />
                    </Button>
                    <Input
                      className="h-8 rounded-md px-2 text-center [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none"
                      id={`${id}-amount`}
                      min={0}
                      onChange={(event) => {
                        setAmount(event.target.value)
                        setError(null)
                      }}
                      required
                      step="any"
                      type="number"
                      value={amount}
                    />
                    <Button aria-label="Increase duration" onClick={() => stepAmount(1)} size="icon-sm" type="button" variant="ghost">
                      <Plus aria-hidden="true" />
                    </Button>
                  </span>
                </div>
                <label className={LABEL_CLASS}>
                  Unit
                  <Select
                    onChange={(event) => {
                      const next = DURATION_UNITS.find((value) => value === event.target.value)
                      if (next) setUnit(next)
                      setError(null)
                    }}
                    value={unit}
                  >
                    <option value="minutes">Minutes</option>
                    <option value="hours">Hours</option>
                    <option value="days">Days</option>
                  </Select>
                </label>
              </div>
            )}
            {error && (
              <p className="text-sm text-danger-strong" role="alert">
                {error}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button onClick={onClose} type="button" variant="secondary">
                Cancel
              </Button>
              <Button type="submit">Snooze</Button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
