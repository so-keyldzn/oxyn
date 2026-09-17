import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { cn } from "cn"
import {
  ArrowDown01Icon,
  FloppyDiskIcon,
  LeftToRightListBulletIcon,
  PlayIcon,
  SearchList01Icon,
  StopIcon,
} from "@hugeicons/core-free-icons"

import { ReadOnlyBadge } from "@/components/oxyn/read-only-badge"
import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { Spinner } from "@/components/ui/spinner"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

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
  context,
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
  /** The session context picker, when the session declares it. */
  context?: React.ReactNode
}) {
  const titleId = React.useId()
  const noticeId = React.useId()
  const idle = canRun && !running
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
            keys={["⌘", "↵"]}
          >
            <Button
              size="sm"
              onClick={onRun}
              disabled={!idle}
              aria-keyshortcuts="Meta+Enter"
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
              <DropdownMenuItem onClick={onRunAll}>
                Run all
                <DropdownMenuShortcut>⌘⇧↵</DropdownMenuShortcut>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </ButtonGroup>
        <Button
          size="sm"
          variant="outline"
          onClick={onCancel}
          disabled={!running || cancelling}
          aria-keyshortcuts="Escape"
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
          {cancelling ? null : <Kbd className="ml-1">Esc</Kbd>}
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
              : `⌘Enter executes: ${target === "selection" ? "selection" : "current statement"}`}
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
        <div className="ml-auto min-w-0">{context}</div>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <label htmlFor={titleId} className="text-xs text-muted-foreground">
          Query name
        </label>
        <Input
          id={titleId}
          value={title}
          dir="auto"
          onChange={(event) => onTitleChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") onSave()
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
          <Shortcut label="Save query" keys={["⌘", "S"]}>
            <Button
              size="sm"
              variant="outline"
              disabled={save.status === "saving"}
              onClick={onSave}
              aria-keyshortcuts="Meta+S"
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
