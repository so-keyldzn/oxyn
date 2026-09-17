import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowUp02Icon, StopIcon } from "@hugeicons/core-free-icons"

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupText,
  InputGroupTextarea,
} from "@/components/ui/input-group"
import { Kbd } from "@/components/ui/kbd"

/**
 * The question field.
 *
 * Enter sends, Shift+Enter breaks the line, and nothing is sent while a
 * question runs — the draft stays, to send once the answer is done. Escape
 * stops a running conversation from the field, where the user is standing.
 * `onSubmit` resolves `false` when the backend refused before starting: the
 * draft is kept, the refusal shown by the caller.
 */
export function AssistantComposer({
  running,
  stopRequested = false,
  disabledReason = null,
  initialValue = "",
  onSubmit,
  onStop,
}: {
  running: boolean
  stopRequested?: boolean
  /** Why nothing can be asked right now; the field stays readable. */
  disabledReason?: string | null
  initialValue?: string
  onSubmit: (question: string) => Promise<boolean> | boolean
  onStop: () => void
}) {
  const [value, setValue] = React.useState(initialValue)
  const [sending, setSending] = React.useState(false)
  // A message typed while an answer runs is queued, not refused: it is sent
  // when the run ends, and it approves nothing meanwhile.
  const blocked = sending || disabledReason !== null
  const hintId = React.useId()

  const send = async () => {
    const question = value.trim()
    if (blocked || question === "") return
    setSending(true)
    try {
      if (await onSubmit(question)) setValue("")
    } finally {
      setSending(false)
    }
  }

  return (
    <form
      className="flex flex-col gap-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        void send()
      }}
    >
      <InputGroup>
        <InputGroupTextarea
          aria-label="Question for the assistant"
          aria-describedby={hintId}
          placeholder="Ask about this connection…"
          rows={2}
          value={value}
          // Not `disabled`: the input group dims every child of a group
          // holding a disabled control, below readable contrast.
          readOnly={disabledReason !== null}
          aria-disabled={disabledReason !== null || undefined}
          className="max-h-40 min-h-14"
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && running) {
              event.preventDefault()
              onStop()
              return
            }
            if (
              event.key === "Enter" &&
              !event.shiftKey &&
              !event.nativeEvent.isComposing
            ) {
              event.preventDefault()
              void send()
            }
          }}
        />
        <InputGroupAddon align="block-end" className="justify-between">
          <InputGroupText id={hintId} className="text-xs">
            {disabledReason ??
              (running ? (
                stopRequested ? (
                  "Stop requested. It ends as soon as the provider speaks again."
                ) : (
                  <>
                    Working · <Kbd>↵</Kbd> queues · <Kbd>Esc</Kbd> stops
                  </>
                )
              ) : (
                <>
                  <Kbd>↵</Kbd> send · <Kbd>⇧↵</Kbd> new line
                </>
              ))}
          </InputGroupText>
          {running ? (
            // Stays enabled once pressed: the request is taken at once, its
            // effect is the provider's (docs/UX-SPEC.md).
            <InputGroupButton
              size="icon-sm"
              variant="outline"
              aria-label="Stop the assistant"
              onClick={onStop}
            >
              <HugeiconsIcon icon={StopIcon} strokeWidth={2} />
            </InputGroupButton>
          ) : (
            <InputGroupButton
              type="submit"
              size="icon-sm"
              variant="default"
              aria-label="Send question"
              aria-disabled={blocked || value.trim() === "" || undefined}
            >
              <HugeiconsIcon icon={ArrowUp02Icon} strokeWidth={2} />
            </InputGroupButton>
          )}
        </InputGroupAddon>
      </InputGroup>
    </form>
  )
}
