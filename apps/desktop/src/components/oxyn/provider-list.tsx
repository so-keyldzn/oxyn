import * as React from "react"
import { cva } from "class-variance-authority"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowRight01Icon,
  Link01Icon,
} from "@hugeicons/core-free-icons"

import { ProviderLogo } from "@/components/oxyn/provider-logo"
import type {
  ProviderRow,
  RowLogo,
  RowStatus,
} from "@/components/oxyn/provider-list-model"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import type { AgentPresetId } from "@/lib/ipc/ai"

/** The provider's mark on its tile, decorative: the name is written beside it. */
export function ProviderLogoTile({ logo }: { logo: RowLogo }) {
  return (
    <span className="flex size-10 shrink-0 items-center justify-center rounded-lg border bg-muted/40">
      <ProviderLogo {...logo} className="size-5" />
    </span>
  )
}

// Literal class names, one per tone: Tailwind only generates what it reads.
const statusPill = cva("gap-1.5 rounded-full font-normal", {
  variants: {
    tone: {
      success: "border-success/40 bg-success/10 text-success",
      warning: "border-warning/40 bg-warning/10 text-warning",
      neutral: "border-border text-muted-foreground",
    },
  },
})

/** A dot and a word: the colour repeats the label, never replaces it. */
export function ProviderStatusPill({ status }: { status: RowStatus }) {
  return (
    <Badge variant="outline" className={statusPill({ tone: status.tone })}>
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      {status.label}
    </Badge>
  )
}

function ProviderRowButton({
  row,
  onOpen,
}: {
  row: ProviderRow
  onOpen: () => void
}) {
  const lineId = `provider-row-line-${row.id}`
  return (
    <Item
      variant="outline"
      className="flex-nowrap text-left hover:bg-muted/50"
      data-row-id={row.id}
      // A native button: Enter and Space open it, and it takes the focus.
      render={<button type="button" />}
      aria-label={`${row.title}, ${row.status.label}`}
      aria-describedby={lineId}
      onClick={onOpen}
    >
      <ItemMedia>
        <ProviderLogoTile logo={row.logo} />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle>{row.title}</ItemTitle>
        <ItemDescription
          id={lineId}
          className="flex items-center gap-1.5 truncate text-xs"
        >
          {row.endpointDependent ? (
            <HugeiconsIcon
              icon={Link01Icon}
              strokeWidth={2}
              aria-hidden
              className="size-3.5 shrink-0"
            />
          ) : null}
          <span className="truncate">{row.line}</span>
        </ItemDescription>
      </ItemContent>
      <ItemActions>
        <ProviderStatusPill status={row.status} />
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          aria-hidden
          className="size-4 text-muted-foreground"
        />
      </ItemActions>
    </Item>
  )
}

/** The two subscriptions a user most likely already pays for. */
const SIGN_IN: ReadonlyArray<{ preset: AgentPresetId; label: string }> = [
  { preset: "claude-code", label: "Sign in with Claude" },
  { preset: "codex", label: "Sign in with ChatGPT" },
]

export interface ProviderListProps {
  status: "loading" | "ready" | "error"
  loadError: string | null
  rows: ReadonlyArray<ProviderRow>
  /** The row to give the focus back to, coming back from it. */
  focusRow: string | null
  onOpen: (row: ProviderRow) => void
}

/**
 * Every destination Oxyn can declare, one row each, with what is known of it.
 *
 * Detected is not declared: an agent found on the machine is only declared
 * once the user confirms, twice (ADR-0026). The sign-in buttons therefore open
 * the agent's page, where it is declared and where its sign-in command is
 * shown: Oxyn signs no one in outside a conversation.
 */
export function ProviderList({
  status,
  loadError,
  rows,
  focusRow,
  onOpen,
}: ProviderListProps) {
  const listRef = React.useRef<HTMLUListElement>(null)
  const declared = rows.filter((row) => row.declared).length
  const signIn = SIGN_IN.flatMap((entry) => {
    const row = rows.find(
      (candidate) =>
        candidate.subject.kind === "preset" &&
        candidate.subject.preset.id === entry.preset
    )
    return row ? [{ ...entry, row }] : []
  })

  React.useEffect(() => {
    if (focusRow === null) return
    listRef.current
      ?.querySelector<HTMLElement>(`[data-row-id="${CSS.escape(focusRow)}"]`)
      ?.focus()
  }, [focusRow])

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 id="ai-providers-title" className="text-base font-medium">
          Connect your AI provider
        </h2>
        <p className="text-sm text-muted-foreground">
          Use the subscriptions you already pay for. Oxyn detects Claude Code
          and Codex when they are installed on this machine, declares them only
          once you confirm, and never sees their credentials. What each
          connection lets leave is set on the connection.
        </p>
      </div>

      {signIn.length > 0 && status === "ready" ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex flex-wrap gap-2">
            {signIn.map(({ preset, label, row }, index) => (
              <Button
                key={preset}
                variant={index === 0 ? "default" : "outline"}
                aria-describedby="provider-sign-in-help"
                onClick={() => onOpen(row)}
              >
                <ProviderLogo brand={preset} data-icon="inline-start" />
                {label}
              </Button>
            ))}
          </div>
          <p
            id="provider-sign-in-help"
            className="text-xs text-muted-foreground"
          >
            Opens the agent: declare it, then sign in with the command shown.
          </p>
        </div>
      ) : null}

      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-medium">Providers</h3>
        {status === "ready" ? (
          <span className="text-xs text-muted-foreground">
            {declared} of {rows.length} declared
          </span>
        ) : null}
      </div>

      {status === "loading" ? (
        <div
          role="status"
          className="flex flex-col gap-2"
          aria-busy="true"
          aria-label="Loading providers"
        >
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
        </div>
      ) : status === "error" ? (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>The declarations could not be read</AlertTitle>
          <AlertDescription data-selectable className="font-mono text-xs">
            {loadError}
          </AlertDescription>
        </Alert>
      ) : (
        <>
          {declared === 0 ? (
            <p className="text-sm text-muted-foreground">
              Nothing declared yet: Ask AI appears in the connection bar with
              the first one, and until then nothing is sent anywhere.
            </p>
          ) : null}
          <ul
            ref={listRef}
            aria-label="Providers"
            className="flex flex-col gap-2"
          >
            {rows.map((row) => (
              <li key={row.id}>
                <ProviderRowButton row={row} onOpen={() => onOpen(row)} />
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  )
}
