import { HugeiconsIcon } from "@hugeicons/react"
import {
  ComputerTerminal01Icon,
  Login03Icon,
  Tick02Icon,
} from "@hugeicons/core-free-icons"

import { AssistantCopyButton } from "@/components/oxyn/assistant-copy-button"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import type { SignInState } from "@/features/assistant/conversation-store"
import type { SignInHelp } from "@/lib/ipc/ai"

/**
 * How to sign in with an external agent that asked for it.
 *
 * The agent does the signing in — usually in a browser — or the user does it
 * in a terminal. Oxyn neither reads nor keeps a token: the subscription stays
 * between the user and the agent's vendor. Asking again is a separate click,
 * once the user is done.
 */
export function AssistantSignIn({
  help,
  states,
  onSignIn,
  onCopy,
}: {
  help: SignInHelp
  states: Record<string, SignInState>
  onSignIn: (method: string) => void
  onCopy: (text: string) => Promise<boolean> | boolean
}) {
  const agentMethods = help.methods.filter((method) => method.kind === "agent")
  const terminalMethods = help.methods.filter(
    (method) => method.kind === "terminal" && method.command !== null
  )
  const nothingToOffer =
    agentMethods.length === 0 &&
    terminalMethods.length === 0 &&
    help.terminalCommand === null

  return (
    <div data-slot="assistant-sign-in" className="flex flex-col gap-3 text-xs">
      {agentMethods.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {agentMethods.map((method) => {
            const state = states[method.id]
            return (
              <div key={method.id} className="flex flex-col gap-1">
                <Button
                  size="sm"
                  variant={state?.status === "done" ? "outline" : "default"}
                  disabled={state?.status === "pending"}
                  aria-describedby={
                    method.description ? `sign-in-${method.id}` : undefined
                  }
                  onClick={() => onSignIn(method.id)}
                >
                  {state?.status === "pending" ? (
                    <Spinner data-icon="inline-start" />
                  ) : (
                    <HugeiconsIcon
                      icon={state?.status === "done" ? Tick02Icon : Login03Icon}
                      strokeWidth={2}
                      data-icon="inline-start"
                    />
                  )}
                  {state?.status === "done"
                    ? `Signed in with ${method.name}`
                    : `Sign in with ${method.name}`}
                </Button>
                {method.description ? (
                  <p
                    id={`sign-in-${method.id}`}
                    className="text-muted-foreground"
                  >
                    {method.description}
                  </p>
                ) : null}
                {state?.status === "error" ? (
                  <p role="alert" className="text-destructive" data-selectable>
                    {state.message}
                  </p>
                ) : null}
              </div>
            )
          })}
        </div>
      ) : null}

      {[
        ...terminalMethods.map((method) => ({
          key: method.id,
          name: method.name,
          command: method.command ?? "",
        })),
        ...(help.terminalCommand && terminalMethods.length === 0
          ? [
              {
                key: "documented",
                name: `Sign in to ${help.agent}`,
                command: help.terminalCommand,
              },
            ]
          : []),
      ].map((entry) => (
        <div key={entry.key} className="flex flex-col gap-1">
          <p className="flex items-center gap-1.5 text-muted-foreground">
            <HugeiconsIcon
              icon={ComputerTerminal01Icon}
              strokeWidth={2}
              className="size-3.5"
              aria-hidden
            />
            {entry.name} · run this in a terminal, then ask again
          </p>
          <div className="flex min-w-0 items-center gap-1 rounded-md border bg-muted/40 pl-2">
            <code
              data-selectable
              tabIndex={0}
              role="region"
              aria-label={`Command to sign in: ${entry.name}`}
              className="min-w-0 flex-1 overflow-x-auto py-1.5 font-mono whitespace-nowrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {entry.command}
            </code>
            <AssistantCopyButton
              text={entry.command}
              label="Copy command"
              onCopy={onCopy}
            />
          </div>
        </div>
      ))}

      {nothingToOffer ? (
        <p className="text-muted-foreground">
          {help.agent} gave no way to sign in from here. Sign in with its own
          command-line tool, then ask again.
        </p>
      ) : null}
      <p className="text-muted-foreground">
        Oxyn never sees nor keeps the agent's credentials.
      </p>
    </div>
  )
}
