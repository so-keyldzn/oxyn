import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowDown01Icon,
  ArrowReloadHorizontalIcon,
  FileNotFoundIcon,
  Login03Icon,
  SquareLock02Icon,
} from "@hugeicons/core-free-icons"

import { AssistantSignIn } from "@/components/oxyn/assistant-sign-in"
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import type { SignInState } from "@/features/assistant/conversation-store"
import type { FailedEntry } from "@/features/assistant/transcript"
import type { AgentExit, FailureCategory } from "@/lib/ipc/ai"

export const FAILURE_TITLES: Record<FailureCategory, string> = {
  refused: "Not sent",
  provider: "The answer stopped",
  setup: "The conversation could not start",
  agentNotFound: "The agent's program was not found",
  agentSignIn: "Sign in to the agent",
  agentIncompatible: "This agent speaks another protocol version",
  agentExited: "The agent stopped",
  agentTimedOut: "The agent took too long to start",
  agent: "The agent reported an error",
  unknown: "The conversation stopped for an unknown reason",
}

/**
 * What « ask again » does, said on the button. For an agent that stopped or
 * never started, asking again starts a new one — and the message says the
 * same, so the two never disagree.
 */
const RETRY_LABELS: Partial<Record<FailureCategory, string>> = {
  agentSignIn: "Ask again",
  agentExited: "Restart the agent and ask again",
  agentTimedOut: "Restart the agent and ask again",
  agentNotFound: "Try again",
}

export function failureIcon(category: FailureCategory) {
  switch (category) {
    case "refused":
      return SquareLock02Icon
    case "agentNotFound":
      return FileNotFoundIcon
    case "agentSignIn":
      return Login03Icon
    default:
      return Alert02Icon
  }
}

/**
 * What the agent's process said as it died, folded: the cause is usually on
 * its last lines, and a wall of it above the button would bury the button.
 *
 * The backend already replaced every value Oxyn handed the process — declared
 * variables, the tool token — by a marker. What is left is shown as text and
 * can be selected, never interpreted.
 */
export function AgentExitDetails({ exit }: { exit: AgentExit }) {
  return (
    <Collapsible data-slot="agent-exit" className="flex min-w-0 flex-col">
      <CollapsibleTrigger className="group/exit inline-flex w-fit items-center gap-1 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
        Details
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0 transition-transform group-data-[panel-open]/exit:rotate-180 motion-reduce:transition-none"
          aria-hidden
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="overflow-hidden">
        <p className="mt-1 text-xs">
          {exit.code === null
            ? "The process was stopped by a signal."
            : `The process exited with code ${exit.code}.`}
          {exit.output === "" ? " It wrote nothing on its way out." : ""}
        </p>
        {exit.output !== "" ? (
          // `tabIndex`: a region that scrolls and holds no control is
          // unreachable from the keyboard otherwise.
          <pre
            data-selectable
            tabIndex={0}
            aria-label="The agent's last output"
            className="mt-1 max-h-48 overflow-auto rounded-md bg-muted p-2 font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {exit.output}
          </pre>
        ) : null}
      </CollapsibleContent>
    </Collapsible>
  )
}

/**
 * Why a run broke off, in the terms the backend classified it.
 *
 * The message is shown whole — the public reads error messages. Asking again
 * is always a click, never automatic (I-13), and never offered after a refusal:
 * the tier would refuse again.
 */
export function AssistantFailure({
  entry,
  canRetry,
  signInStates,
  onRetry,
  onSignIn,
  onCopy,
}: {
  entry: FailedEntry
  canRetry: boolean
  signInStates: Record<string, SignInState>
  onRetry: () => void
  onSignIn: (method: string) => void
  onCopy: (text: string) => Promise<boolean> | boolean
}) {
  const signIn = entry.category === "agentSignIn"
  return (
    <Alert
      data-slot="assistant-failure"
      data-category={entry.category}
      variant={signIn ? "default" : "destructive"}
    >
      <HugeiconsIcon icon={failureIcon(entry.category)} strokeWidth={2} />
      <AlertTitle>{FAILURE_TITLES[entry.category]}</AlertTitle>
      <AlertDescription className="flex flex-col gap-2">
        <p data-selectable className="font-mono text-xs break-words">
          {entry.message}
        </p>
        {entry.foundElsewhere ? (
          <p className="text-xs">
            A program of that name exists at{" "}
            <code data-selectable className="font-mono break-all">
              {entry.foundElsewhere}
            </code>
            . Oxyn does not look there when it starts the agent: put that path
            in the agent's command, in the AI settings.
          </p>
        ) : null}
        {entry.exit ? <AgentExitDetails exit={entry.exit} /> : null}
        {entry.signIn ? (
          <AssistantSignIn
            help={entry.signIn}
            states={signInStates}
            onSignIn={onSignIn}
            onCopy={onCopy}
          />
        ) : null}
      </AlertDescription>
      {entry.retryable ? (
        <AlertAction>
          <Button
            size="xs"
            variant="outline"
            disabled={!canRetry}
            // Asks the model again. A tool call only runs again if the model
            // asks for it, through the same gate (I-13).
            onClick={onRetry}
          >
            <HugeiconsIcon
              icon={ArrowReloadHorizontalIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            {RETRY_LABELS[entry.category] ?? "Ask again"}
          </Button>
        </AlertAction>
      ) : null}
    </Alert>
  )
}
