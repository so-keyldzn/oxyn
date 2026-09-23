import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowReloadHorizontalIcon } from "@hugeicons/core-free-icons"

import {
  AgentExitDetails,
  FAILURE_TITLES,
  failureIcon,
} from "@/components/oxyn/assistant-failure"
import { AssistantSignIn } from "@/components/oxyn/assistant-sign-in"
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import type { AgentStartup } from "@/features/assistant/agent-startup"
import type { SignInState } from "@/features/assistant/conversation-store"

/** The start of an external agent before its first question, and its controls. */
export interface AgentStartupControls {
  startup: AgentStartup
  signInStates: Record<string, SignInState>
  /** Stops the start. The agent is not launched again until asked. */
  onCancel: () => void
  /** Starts it again — a click, never automatic (I-13). */
  onStart: () => void
  onSignIn: (method: string) => void
  onCopy: (text: string) => Promise<boolean> | boolean
}

/**
 * Where the start of an external agent stands, before any question.
 *
 * Drawn only while it says something: a start that is ready, or that answered
 * too fast to be seen, draws nothing — the agent's settings and version then
 * speak for it. A failure is said in the backend's words, the process's last
 * output folded under « Details ».
 */
export function AssistantAgentStartup({
  startup,
  signInStates,
  onCancel,
  onStart,
  onSignIn,
  onCopy,
}: AgentStartupControls) {
  switch (startup.status) {
    case "idle":
    case "ready":
      return null
    case "starting":
      if (!startup.shown) return null
      return (
        <div
          data-slot="agent-startup"
          data-state="starting"
          className="flex min-w-0 items-center gap-2 text-xs"
        >
          <Spinner aria-hidden className="size-3.5 shrink-0" role={undefined} />
          <div role="status" className="flex min-w-0 flex-col">
            <span className="font-medium">Starting {startup.label}…</span>
            <span className="text-muted-foreground">
              A first start downloads the agent and can take a minute.
            </span>
          </div>
          <Button
            size="xs"
            variant="outline"
            className="ml-auto shrink-0"
            onClick={onCancel}
          >
            Cancel
          </Button>
        </div>
      )
    case "stopped":
      return (
        <div
          data-slot="agent-startup"
          data-state="stopped"
          className="flex min-w-0 items-center gap-2 text-xs"
        >
          <p className="min-w-0 text-muted-foreground">
            {startup.label} was not started. Your first question starts it.
          </p>
          <Button
            size="xs"
            variant="outline"
            className="ml-auto shrink-0"
            onClick={onStart}
          >
            Start now
          </Button>
        </div>
      )
    case "failed": {
      const { failure } = startup
      const signIn = failure.signIn !== null
      return (
        <Alert
          data-slot="agent-startup"
          data-state="failed"
          data-category={failure.category}
          variant={signIn ? "default" : "destructive"}
        >
          <HugeiconsIcon icon={failureIcon(failure.category)} strokeWidth={2} />
          <AlertTitle>
            {signIn
              ? FAILURE_TITLES.agentSignIn
              : `${startup.label} could not start`}
          </AlertTitle>
          <AlertDescription className="flex flex-col gap-2">
            {!signIn ? (
              <p className="text-xs">{FAILURE_TITLES[failure.category]}.</p>
            ) : null}
            <p data-selectable className="font-mono text-xs break-words">
              {failure.message}
            </p>
            {failure.foundElsewhere ? (
              <p className="text-xs">
                A program of that name exists at{" "}
                <code data-selectable className="font-mono break-all">
                  {failure.foundElsewhere}
                </code>
                . Oxyn does not look there when it starts the agent: put that
                path in the agent's command, in the AI settings.
              </p>
            ) : null}
            {failure.exit ? <AgentExitDetails exit={failure.exit} /> : null}
            {failure.signIn ? (
              <AssistantSignIn
                help={failure.signIn}
                states={signInStates}
                onSignIn={onSignIn}
                onCopy={onCopy}
              />
            ) : null}
          </AlertDescription>
          {/* A refusal would be refused again: nothing to start. */}
          {failure.category !== "refused" ? (
            <AlertAction>
              <Button size="xs" variant="outline" onClick={onStart}>
                <HugeiconsIcon
                  icon={ArrowReloadHorizontalIcon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Start again
              </Button>
            </AlertAction>
          ) : null}
        </Alert>
      )
    }
  }
}
