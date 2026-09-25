import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  FloppyDiskIcon,
  LeftToRightListBulletIcon,
  PlayIcon,
  SearchList01Icon,
  StopIcon,
} from "@hugeicons/core-free-icons"

import { ReadOnlyBadge } from "@/components/oxyn/read-only-badge"
import { TransactionBadge } from "@/components/oxyn/transaction-badge"
import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { Spinner } from "@/components/ui/spinner"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { actionKeys, actionShortcut, ariaKeys } from "@/lib/actions/manifest"
import { consoleAvailability } from "@/lib/actions/registry"
import { cn } from "@/lib/utils"
import { TextInput } from "./text-field"

/** Where the named copy stands. `conflict` keeps the local text and offers a copy. */
export type SaveState =
  | { status: "idle"; notice: string }
  | { status: "saving" }
  | { status: "conflict"; notice: string }
  | { status: "failed"; notice: string }

/** « 850 ms », « 12.4 s », « 3 min 05 s » — steady width with tabular digits. */
export function formatElapsed(ms: number) {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  const minutes = Math.floor(ms / 60_000)
  const seconds = Math.floor((ms % 60_000) / 1000)
  return `${minutes} min ${String(seconds).padStart(2, "0")} s`
}

function Shortcut({
  label,
  keys,
  children,
}: {
  label: string
  keys: Array<string>
  children: React.ReactElement
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent className="flex items-center gap-2">
        {label}
        <KbdGroup>
          {keys.map((key) => (
            <Kbd key={key}>{key}</Kbd>
          ))}
        </KbdGroup>
      </TooltipContent>
    </Tooltip>
  )
}

/**
 * The actions of one console: run what the cursor or selection targets, stop,
 * explain, bound values, name and save.
 *
 * Run, Stop and Explain are three distinct buttons at fixed places; the one
 * with nothing to do is dimmed and inert (docs/UX-SPEC.md, « Portée de Run »),
 * so a missed click never restarts what the user meant to stop. The hint says
 * what ⌘↵ will send before it is sent.
 */
export function ConsoleToolbar({
  running,
  cancelling = false,
  elapsedMs = null,
  canRun,
  readOnly,
  target,
  onRun,
  onRunAll,
  onExplain,
  onCancel,
  parameterCount,
  parametersOpen,
  onToggleParameters,
  title,
  onTitleChange,
  titleError,
  save,
  onSave,
  onSaveAsNew,
  closing = false,
  onCancelWrite,
  context,
  transaction = null,
}: {
  running: boolean
  /** Stop was pressed; the server has not answered yet. */
  cancelling?: boolean
  /** Time since the run started, while it runs. */
  elapsedMs?: number | null
  /** The session declares `SQL`, and the console is not closing. */
  canRun: boolean
  readOnly: boolean
  target: "selection" | "statement"
  onRun: () => void
  onRunAll: () => void
  onExplain: () => void
  onCancel: () => void
  parameterCount: number
  parametersOpen: boolean
  onToggleParameters: () => void
  title: string
  onTitleChange: (title: string) => void
  titleError: string | null
  save: SaveState
  onSave: () => void
  onSaveAsNew: () => void
  /**
   * The console is closing its document: the name stays readable but no
   * longer editable (docs/UX-SPEC.md, « Sauvegarde d'une console »).
   */
  closing?: boolean
  /** Cancels the named save or close under way. */
  onCancelWrite?: () => void
  /** The session context picker, when the session declares it. */
  context?: React.ReactNode
  /**
   * What the session last reported about its transaction, when there is
   * something to say (ADR-0039 §5): drawn beside the context.
   */
  transaction?: "open" | "unknown" | null
}) {
  const titleId = React.useId()
  const noticeId = React.useId()
  const writing = save.status === "saving" || closing
  // The registry's conditions, the same for these buttons as for the menu
  // bar and the keyboard (UX-SPEC, « Barre de menus »).
  const state = { canRun, running, cancelling, writing }
  const idle = consoleAvailability("run", state) === true
  const stoppable = consoleAvailability("cancel", state) === true
  const savable = consoleAvailability("save", state) === true
  const notice =
    titleError ??
    (save.status === "saving" ? "Saving named query…" : save.notice)
  const noticeWarns =
    titleError !== null ||
    save.status === "conflict" ||
    save.status === "failed"
  return (
    <div
      data-slot="console-toolbar"
      className="flex shrink-0 flex-col gap-1.5 border-b px-3 py-2"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <ButtonGroup aria-label="Execution">
          <Shortcut
            label={target === "selection" ? "Run selection" : "Run statement"}
            keys={actionKeys("console.run")}
          >
            <Button
              size="sm"
              onClick={onRun}
              disabled={!idle}
              aria-keyshortcuts={ariaKeys("console.run")}
            >
              <HugeiconsIcon
                icon={PlayIcon}
                strokeWidth={2}
                data-icon="inline-start"
              />
              Run
            </Button>
          </Shortcut>
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  size="icon-sm"
                  aria-label="More run options"
                  disabled={!idle}
                />
              }
            >
              <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuGroup>
                <DropdownMenuItem onClick={onRunAll}>
                  Run all
                  <DropdownMenuShortcut>
                    {actionShortcut("console.runAll")}
                  </DropdownMenuShortcut>
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </ButtonGroup>
        <Button
          size="sm"
          variant="outline"
          onClick={onCancel}
          disabled={!stoppable}
          aria-keyshortcuts={ariaKeys("console.cancel")}
        >
          {cancelling ? (
            <Spinner data-icon="inline-start" />
          ) : (
            <HugeiconsIcon
              icon={StopIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
          )}
          {cancelling ? "Cancelling…" : "Stop"}
          {cancelling ? null : (
            <Kbd className="ml-1">{actionShortcut("console.cancel")}</Kbd>
          )}
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={onExplain}
          disabled={!idle}
        >
          <HugeiconsIcon
            icon={SearchList01Icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Explain
        </Button>
        <span className="text-xs text-muted-foreground tabular-nums">
          {cancelling
            ? "Cancellation requested — waiting for the server"
            : running
              ? `Running${elapsedMs === null ? "" : ` · ${formatElapsed(elapsedMs)}`} — Esc cancels`
              : `${actionShortcut("console.run")} executes: ${target === "selection" ? "selection" : "current statement"}`}
        </span>
        <Button
          size="sm"
          variant={parametersOpen ? "secondary" : "ghost"}
          aria-pressed={parametersOpen}
          onClick={onToggleParameters}
        >
          <HugeiconsIcon
            icon={LeftToRightListBulletIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Parameters
          <span className="tabular-nums">· {parameterCount}</span>
        </Button>
        {readOnly ? <ReadOnlyBadge /> : null}
        <div className="ml-auto flex min-w-0 items-center gap-2">
          {transaction ? <TransactionBadge state={transaction} /> : null}
          {context}
        </div>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <label htmlFor={titleId} className="text-xs text-muted-foreground">
          Query name
        </label>
        <TextInput
          id={titleId}
          value={title}
          dir="auto"
          readOnly={closing}
          onChange={(event) => onTitleChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !closing) onSave()
          }}
          aria-invalid={titleError !== null}
          // The refusal is read with the field, not only beside it: a name too
          // long is announced as « invalid » and nothing else otherwise.
          aria-describedby={noticeId}
          className="h-7 w-60 min-w-0"
        />
        {save.status === "conflict" ? (
          <Button size="sm" variant="outline" onClick={onSaveAsNew}>
            <HugeiconsIcon
              icon={FloppyDiskIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Save as new query
          </Button>
        ) : (
          <Shortcut label="Save query" keys={actionKeys("console.save")}>
            <Button
              size="sm"
              variant="outline"
              disabled={!savable}
              onClick={onSave}
              aria-keyshortcuts={ariaKeys("console.save")}
            >
              {save.status === "saving" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon
                  icon={FloppyDiskIcon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
              )}
              Save query
            </Button>
          </Shortcut>
        )}
        {writing && onCancelWrite ? (
          <Button size="sm" variant="ghost" onClick={onCancelWrite}>
            {closing ? "Cancel close" : "Cancel save"}
          </Button>
        ) : null}
        <span
          id={noticeId}
          role="status"
          // Truncated on a narrow window, so the whole sentence stays reachable
          // by pointer as well as by the field that describes it.
          title={notice}
          className={cn(
            "min-w-0 truncate text-xs",
            noticeWarns ? "text-warning" : "text-muted-foreground"
          )}
        >
          {notice}
        </span>
      </div>
    </div>
  )
}
