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
import { cn } from "@/lib/utils"

/** What the panel may do to the field from outside it. Nothing here sends. */
export interface AssistantComposerHandle {
  /** Replaces the draft and puts the caret at its end; the user still sends. */
  fill: (text: string) => void
  focus: () => void
}

/**
 * Finds the question field `Ask AI` gives the focus to once the panel is
 * shown. The panel stays mounted while hidden, so a focus on mount would
 * happen once, and never on the next opening. A hidden workspace keeps its
 * own field in the document: only a rendered one is returned.
 */
export function visibleAssistantField(root: ParentNode = document) {
  return (
    Array.from(
      root.querySelectorAll<HTMLTextAreaElement>("[data-assistant-question]")
    ).find((element) => element.getClientRects().length > 0) ?? null
  )
}

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
  ref,
  running,
  stopRequested = false,
  disabledReason = null,
  initialValue = "",
  onSubmit,
  onStop,
}: {
  ref?: React.Ref<AssistantComposerHandle>
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
  const field = React.useRef<HTMLTextAreaElement>(null)
  // A message typed while an answer runs is queued, not refused: it is sent
  // when the run ends, and it approves nothing meanwhile.
  const blocked = sending || disabledReason !== null
  const unsendable = blocked || value.trim() === ""
  const hintId = React.useId()

  React.useImperativeHandle(
    ref,
    () => ({
      fill: (text) => {
        setValue(text)
        const element = field.current
        if (!element) return
        element.focus()
        // After React has written the value, or the caret lands before it.
        requestAnimationFrame(() =>
          element.setSelectionRange(text.length, text.length)
        )
      },
      focus: () => field.current?.focus(),
    }),
    []
  )

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
          ref={field}
          data-assistant-question=""
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
            // `aria-disabled` rather than `disabled`, for the same dimming as
            // the field; the look is changed by hand so a button that cannot
            // send does not keep the colour of one that can.
            <InputGroupButton
              type="submit"
              size="icon-sm"
              variant={unsendable ? "secondary" : "default"}
              aria-label="Send question"
              aria-disabled={unsendable || undefined}
              className={cn(
                unsendable && "cursor-not-allowed text-muted-foreground"
              )}
            >
              <HugeiconsIcon icon={ArrowUp02Icon} strokeWidth={2} />
            </InputGroupButton>
          )}
        </InputGroupAddon>
      </InputGroup>
    </form>
  )
}
