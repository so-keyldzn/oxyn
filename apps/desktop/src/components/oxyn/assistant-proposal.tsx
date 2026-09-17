import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, CodeIcon } from "@hugeicons/core-free-icons"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"
import type { Environment } from "@/lib/ipc/types"

export type ProposalState =
  | { status: "loading" }
  | { status: "ready"; sql: string; title: string; origin: string }
  /** Nothing this dialect can change on the target: said, not hidden. */
  | { status: "unavailable" }
  | { status: "error"; message: string }

/**
 * A schema change to review (ADR-0025): SQL composed by Oxyn, every line
 * commented, that only a console runs — after you uncomment and complete it.
 *
 * `Open in console` hands the text over and runs nothing. No agent reaches
 * this: a change an agent wants goes through a console like anyone's.
 */
export function AssistantProposal({
  relation,
  connectionName,
  environment,
  state,
  onOpenInConsole,
}: {
  relation: string
  connectionName: string
  environment: Environment
  state: ProposalState
  onOpenInConsole: (sql: string, title: string) => void
}) {
  return (
    <Card size="sm" aria-label={`Proposed change for ${relation}`}>
      <CardHeader>
        <CardTitle>Proposed change · {relation}</CardTitle>
        <CardDescription className="flex flex-wrap items-center gap-1.5">
          Nothing runs from here. Review it on
          <strong className="font-medium text-foreground">
            {connectionName}
          </strong>
          <EnvironmentBadge environment={environment} />
        </CardDescription>
        {state.status === "ready" ? (
          <CardAction>
            <Button
              size="sm"
              variant="outline"
              onClick={() => onOpenInConsole(state.sql, state.title)}
            >
              <HugeiconsIcon
                icon={CodeIcon}
                strokeWidth={2}
                data-icon="inline-start"
              />
              Open in console
            </Button>
          </CardAction>
        ) : null}
      </CardHeader>
      <CardContent>
        {state.status === "loading" ? (
          <div
            role="status"
            className="flex flex-col gap-1.5"
            aria-busy="true"
            aria-label="Composing"
          >
            <Skeleton className="h-3 w-3/4" />
            <Skeleton className="h-3 w-2/3" />
            <Skeleton className="h-3 w-1/2" />
          </div>
        ) : state.status === "ready" ? (
          <pre
            data-selectable
            tabIndex={0}
            aria-label="Proposed SQL, commented"
            className="max-h-72 overflow-auto rounded-md border bg-muted/30 p-3 font-mono text-xs leading-5 whitespace-pre outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {state.sql}
          </pre>
        ) : state.status === "unavailable" ? (
          <p className="text-sm text-muted-foreground">
            There is nothing this database can change on this object from a
            template. SQLite, for one, has no ALTER COLUMN and no DROP
            CONSTRAINT.
          </p>
        ) : (
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>The proposal could not be composed</AlertTitle>
            <AlertDescription data-selectable className="font-mono text-xs">
              {state.message}
            </AlertDescription>
          </Alert>
        )}
      </CardContent>
      {state.status === "ready" ? (
        <CardFooter className="text-xs text-muted-foreground">
          {state.origin}
        </CardFooter>
      ) : null}
    </Card>
  )
}
