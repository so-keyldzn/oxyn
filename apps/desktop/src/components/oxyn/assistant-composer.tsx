import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowUp02Icon, StopIcon } from "@hugeicons/core-free-icons"
import { LexicalComposer } from "@lexical/react/LexicalComposer"
import { useLexicalComposerContext } from "@lexical/react/LexicalComposerContext"
import { ContentEditable } from "@lexical/react/LexicalContentEditable"
import { LexicalErrorBoundary } from "@lexical/react/LexicalErrorBoundary"
import { HistoryPlugin } from "@lexical/react/LexicalHistoryPlugin"
import { PlainTextPlugin } from "@lexical/react/LexicalPlainTextPlugin"
import {
  $createLineBreakNode,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $isElementNode,
  COMMAND_PRIORITY_LOW,
  KEY_ENTER_COMMAND,
  KEY_ESCAPE_COMMAND,
} from "lexical"
import type { EditorState, LexicalEditor, LexicalNode } from "lexical"

import { MentionMenuPlugin } from "./assistant-mention-menu"
import type { MentionResults } from "./assistant-mention-menu"
import { $isMentionNode, MentionNode } from "./assistant-mention-node"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupText,
} from "@/components/ui/input-group"
import { Kbd } from "@/components/ui/kbd"
import type { Mention } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

export type { MentionResults } from "./assistant-mention-menu"
export type { MentionChoice, MentionKind } from "./assistant-mention-chip"

/** What the panel may do to the field from outside it. Nothing here sends. */
export interface AssistantComposerHandle {
  /** Replaces the draft and puts the caret at its end; the user still sends. */
  fill: (text: string) => void
  focus: () => void
}

/** The `@` list, when the panel can search this connection's objects. */
export interface MentionSource {
  results: MentionResults
  /** The text after `@` as it is typed; `null` once the list is closed. */
  onQuery: (query: string | null) => void
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
      root.querySelectorAll<HTMLElement>("[data-assistant-question]")
    ).find((element) => element.getClientRects().length > 0) ?? null
  )
}

/** The question as the model will read it, and what it names. */
interface Draft {
  text: string
  mentions: Array<Mention>
}

function readDraft(state: EditorState): Draft {
  return state.read(() => {
    const mentions: Array<Mention> = []
    const visit = (node: LexicalNode) => {
      if ($isMentionNode(node)) mentions.push(node.getMention())
      if ($isElementNode(node)) node.getChildren().forEach(visit)
    }
    visit($getRoot())
    return { text: $getRoot().getTextContent(), mentions }
  })
}

/** Replaces the whole draft with plain text, the caret at its end. */
function $writeDraft(text: string) {
  const paragraph = $createParagraphNode()
  text.split("\n").forEach((line, index) => {
    if (index > 0) paragraph.append($createLineBreakNode())
    if (line !== "") paragraph.append($createTextNode(line))
  })
  $getRoot().clear().append(paragraph)
  paragraph.selectEnd()
}

const THEME = { paragraph: "m-0" }

/**
 * The question field.
 *
 * Enter sends, Shift+Enter breaks the line, and nothing is sent while a
 * question runs — the draft stays, to send once the answer is done. Escape
 * stops a running conversation from the field, where the user is standing.
 * `onSubmit` resolves `false` when the backend refused before starting: the
 * draft is kept, the refusal shown by the caller.
 *
 * Typing `@` opens a list of this connection's objects (`mentions`); the one
 * chosen becomes a chip, and its address goes with the question. While the
 * list is open, Enter chooses and Escape closes it — neither sends nor stops.
 */
export function AssistantComposer({
  ref,
  running,
  stopRequested = false,
  disabledReason = null,
  initialValue = "",
  mentions = null,
  onSubmit,
  onStop,
}: {
  ref?: React.Ref<AssistantComposerHandle>
  running: boolean
  stopRequested?: boolean
  /** Why nothing can be asked right now; the field stays readable. */
  disabledReason?: string | null
  initialValue?: string
  /** Without it, `@` is a character like another. */
  mentions?: MentionSource | null
  onSubmit: (
    question: string,
    mentions: Array<Mention>
  ) => Promise<boolean> | boolean
  onStop: () => void
}) {
  // Read once: the editor is created with the first props, and later ones
  // reach it through its API.
  const [initialConfig] = React.useState(() => ({
    namespace: "oxyn-assistant-question",
    nodes: [MentionNode],
    theme: THEME,
    editable: disabledReason === null,
    editorState: () => $writeDraft(initialValue),
    // A failure inside the editor is a bug of ours, not the user's input:
    // shown by the error boundary, never swallowed.
    onError: (error: Error) => {
      throw error
    },
  }))

  return (
    <LexicalComposer initialConfig={initialConfig}>
      <ComposerBody
        ref={ref}
        running={running}
        stopRequested={stopRequested}
        disabledReason={disabledReason}
        initialValue={initialValue}
        mentions={mentions}
        onSubmit={onSubmit}
        onStop={onStop}
      />
    </LexicalComposer>
  )
}

function ComposerBody({
  ref,
  running,
  stopRequested,
  disabledReason,
  initialValue,
  mentions,
  onSubmit,
  onStop,
}: {
  ref?: React.Ref<AssistantComposerHandle>
  running: boolean
  stopRequested: boolean
  disabledReason: string | null
  initialValue: string
  mentions: MentionSource | null
  onSubmit: (
    question: string,
    mentions: Array<Mention>
  ) => Promise<boolean> | boolean
  onStop: () => void
}) {
  const [editor] = useLexicalComposerContext()
  const [empty, setEmpty] = React.useState(initialValue.trim() === "")
  const [sending, setSending] = React.useState(false)
  // A message typed while an answer runs is queued, not refused: it is sent
  // when the run ends, and it approves nothing meanwhile.
  const blocked = sending || disabledReason !== null
  const unsendable = blocked || empty
  const hintId = React.useId()
  const sendingRef = React.useRef(false)

  React.useEffect(() => {
    editor.setEditable(disabledReason === null)
  }, [editor, disabledReason])

  React.useEffect(
    () =>
      editor.registerUpdateListener(({ editorState }) => {
        setEmpty(readDraft(editorState).text.trim() === "")
      }),
    [editor]
  )

  React.useImperativeHandle(
    ref,
    () => ({
      fill: (text) => {
        editor.update(() => $writeDraft(text))
        editor.focus()
      },
      focus: () => editor.focus(),
    }),
    [editor]
  )

  const send = React.useCallback(async () => {
    const draft = readDraft(editor.getEditorState())
    const question = draft.text.trim()
    // The ref, not the state: two Enters in one tick read the same render.
    if (sendingRef.current || disabledReason !== null || question === "") return
    sendingRef.current = true
    setSending(true)
    try {
      if (await onSubmit(question, draft.mentions))
        editor.update(() => $writeDraft(""))
    } finally {
      sendingRef.current = false
      setSending(false)
    }
  }, [editor, disabledReason, onSubmit])

  return (
    <form
      className="flex flex-col gap-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        void send()
      }}
    >
      <KeyboardPlugin
        editor={editor}
        running={running}
        onSend={() => void send()}
        onStop={onStop}
      />
      <HistoryPlugin />
      {mentions ? (
        <MentionMenuPlugin
          results={mentions.results}
          onQuery={mentions.onQuery}
        />
      ) : null}
      <InputGroup>
        <div className="relative w-full min-w-0">
          <PlainTextPlugin
            contentEditable={
              <ContentEditable
                data-assistant-question=""
                data-slot="input-group-control"
                aria-label="Question for the assistant"
                aria-describedby={hintId}
                aria-multiline
                aria-autocomplete={mentions ? "list" : undefined}
                // Readable, and reachable, while nothing can be asked.
                tabIndex={0}
                aria-disabled={disabledReason !== null || undefined}
                className={cn(
                  "max-h-40 min-h-14 w-full overflow-y-auto px-2.5 py-2 text-base break-words whitespace-pre-wrap outline-none md:text-sm",
                  disabledReason !== null && "cursor-default"
                )}
              />
            }
            // None while nothing can be asked: an invitation to type into a
            // field that takes nothing would contradict the reason below it.
            placeholder={
              disabledReason === null ? (
                <div
                  aria-hidden
                  className="pointer-events-none absolute top-2 left-2.5 text-base text-muted-foreground select-none md:text-sm"
                >
                  {mentions
                    ? "Ask about this connection… @ names an object"
                    : "Ask about this connection…"}
                </div>
              ) : null
            }
            ErrorBoundary={LexicalErrorBoundary}
          />
        </div>
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

/**
 * Enter and Escape, below the `@` list's priority: while the list is open, it
 * takes them first, and they never reach here.
 */
function KeyboardPlugin({
  editor,
  running,
  onSend,
  onStop,
}: {
  editor: LexicalEditor
  running: boolean
  onSend: () => void
  onStop: () => void
}) {
  React.useEffect(
    () =>
      editor.registerCommand(
        KEY_ENTER_COMMAND,
        (event) => {
          if (event?.shiftKey) return false
          // An input method commits its text with Enter: nothing is sent.
          if (event?.isComposing || editor.isComposing()) return true
          event?.preventDefault()
          onSend()
          return true
        },
        COMMAND_PRIORITY_LOW
      ),
    [editor, onSend]
  )
  React.useEffect(
    () =>
      editor.registerCommand(
        KEY_ESCAPE_COMMAND,
        (event) => {
          if (!running) return false
          event.preventDefault()
          onStop()
          return true
        },
        COMMAND_PRIORITY_LOW
      ),
    [editor, running, onStop]
  )
  return null
}
