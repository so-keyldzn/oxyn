import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { PencilEdit02Icon } from "@hugeicons/core-free-icons"

import { AssistantCopyButton } from "@/components/oxyn/assistant-copy-button"
import { AssistantVersions } from "@/components/oxyn/assistant-versions"
import { Bubble, BubbleContent } from "@/components/ui/bubble"
import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import { Message, MessageContent } from "@/components/ui/message"
import { Textarea } from "@/components/ui/textarea"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { Versions } from "@/features/assistant/thread"

/**
 * A question the user asked, with what can be done with it.
 *
 * Editing sends the new text as another version of this exchange: what
 * followed stays with the previous version, one click away. Nothing is edited
 * while an answer runs — it would fork a conversation mid-sentence.
 */
export function AssistantQuestion({
  text,
  versions,
  busy,
  onEdit,
  onSelectVersion,
  onCopy,
}: {
  text: string
  versions: Versions
  /** An answer is running: editing and switching wait. */
  busy: boolean
  /** Resolves to whether the edit was accepted; the editor stays open if not. */
  onEdit: (text: string) => Promise<boolean> | boolean
  onSelectVersion: (node: number) => void
  onCopy: (text: string) => Promise<boolean> | boolean
}) {
  const [editing, setEditing] = React.useState(false)
  const [draft, setDraft] = React.useState(text)
  const [submitting, setSubmitting] = React.useState(false)
  const field = React.useRef<HTMLTextAreaElement>(null)
  const editButton = React.useRef<HTMLButtonElement>(null)

  React.useEffect(() => {
    if (editing) field.current?.focus()
  }, [editing])

  const close = () => {
    setEditing(false)
    setDraft(text)
    // Focus returns where it came from, not to the top of the page.
    requestAnimationFrame(() => editButton.current?.focus())
  }

  const submit = async () => {
    const trimmed = draft.trim()
    if (trimmed === "" || submitting) return
    if (trimmed === text.trim()) {
      close()
      return
    }
    setSubmitting(true)
    try {
      if (await onEdit(trimmed)) setEditing(false)
    } finally {
      setSubmitting(false)
    }
  }

  if (editing) {
    return (
      <Message align="end">
        <MessageContent className="w-full max-w-full">
          <form
            data-slot="assistant-question-editor"
            className="flex w-full flex-col gap-2 rounded-lg border bg-card p-2"
            onSubmit={(event) => {
              event.preventDefault()
              void submit()
            }}
          >
            <Textarea
              ref={field}
              aria-label="Edit your question"
              dir="auto"
              value={draft}
              readOnly={submitting}
              className="max-h-60 min-h-16 resize-none border-0 bg-transparent shadow-none focus-visible:ring-0"
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.nativeEvent.isComposing) return
                if (event.key === "Escape") {
                  event.preventDefault()
                  close()
                } else if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault()
                  void submit()
                }
              }}
            />
            <div className="flex flex-wrap items-center justify-end gap-2">
              <p className="mr-auto text-xs text-muted-foreground">
                Sent as a new version. What followed stays with the old one.
              </p>
              <Button type="button" size="sm" variant="ghost" onClick={close}>
                Cancel <Kbd>Esc</Kbd>
              </Button>
              <Button
                type="submit"
                size="sm"
                disabled={draft.trim() === "" || submitting}
              >
                Send
              </Button>
            </div>
          </form>
        </MessageContent>
      </Message>
    )
  }

  return (
    <Message align="end" className="group/question">
      <MessageContent className="items-end">
        <Bubble variant="secondary" align="end">
          <BubbleContent
            className="whitespace-pre-wrap"
            dir="auto"
            data-selectable
          >
            {text}
          </BubbleContent>
        </Bubble>
        <div className="flex items-center gap-0.5">
          <AssistantVersions
            versions={versions}
            disabled={busy}
            onSelect={onSelectVersion}
          />
          <AssistantCopyButton
            text={text}
            label="Copy question"
            onCopy={onCopy}
          />
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  ref={editButton}
                  type="button"
                  size="icon-xs"
                  variant="ghost"
                  aria-label="Edit question"
                  disabled={busy}
                  onClick={() => {
                    setDraft(text)
                    setEditing(true)
                  }}
                />
              }
            >
              <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
            </TooltipTrigger>
            <TooltipContent>Edit and send again</TooltipContent>
          </Tooltip>
        </div>
      </MessageContent>
    </Message>
  )
}
