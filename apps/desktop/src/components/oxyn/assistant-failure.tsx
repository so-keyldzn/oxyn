import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
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
import type { SignInState } from "@/features/assistant/conversation-store"
import type { FailedEntry } from "@/features/assistant/transcript"
import type { FailureCategory } from "@/lib/ipc/ai"

const TITLES: Record<FailureCategory, string> = {
  refused: "Not sent",
  provider: "The answer stopped",
  setup: "The conversation could not start",
  agentNotFound: "The agent's program was not found",
  agentSignIn: "Sign in to the agent",
  agentIncompatible: "This agent speaks another protocol version",
  agentExited: "The agent stopped",
  agent: "The agent reported an error",
  unknown: "The conversation stopped for an unknown reason",
}

const RETRY_LABELS: Partial<Record<FailureCategory, string>> = {
  agentSignIn: "Ask again",
  agentExited: "Restart the agent",
  agentNotFound: "Try again",
}

function icon(category: FailureCategory) {
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
      <HugeiconsIcon icon={icon(entry.category)} strokeWidth={2} />
      <AlertTitle>{TITLES[entry.category]}</AlertTitle>
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
        {entry.category === "agentIncompatible" ? (
          <p className="text-xs">
            Update the agent or its adapter, then declare it again.
          </p>
        ) : null}
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
