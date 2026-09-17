import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  BubbleChatAddIcon,
  Delete02Icon,
  MessageMultiple01Icon,
  PencilEdit02Icon,
} from "@hugeicons/core-free-icons"
import { cn } from "cn"

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import type { HistoryState } from "@/features/assistant/conversation-store"
import type { ThreadSummary } from "@/lib/ipc/ai"

const UNTITLED = "Untitled conversation"

function when(ms: number) {
  if (!Number.isFinite(ms) || ms <= 0) return ""
  return new Date(ms).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })
}

function Row({
  item,
  current,
  onOpen,
  onRename,
  onDelete,
}: {
  item: ThreadSummary
  current: boolean
  onOpen: () => void
  onRename: (title: string) => Promise<void>
  onDelete: () => void
}) {
  const [renaming, setRenaming] = React.useState(false)
  const [title, setTitle] = React.useState(item.title)
  const [saving, setSaving] = React.useState(false)
  const [error, setError] = React.useState<string | null>(null)
  const label = item.title || UNTITLED

  if (renaming) {
    return (
      <li className="flex flex-col gap-1 rounded-md border p-2">
        <form
          className="flex items-center gap-1"
          onSubmit={async (event) => {
            event.preventDefault()
            if (title.trim() === "") return
            setSaving(true)
            try {
              await onRename(title)
              setRenaming(false)
              setError(null)
            } catch (caught) {
              setError(
                caught instanceof Error ? caught.message : String(caught)
              )
            } finally {
              setSaving(false)
            }
          }}
        >
          <Input
            autoFocus
            dir="auto"
            aria-label={`New title for ${label}`}
            aria-invalid={error !== null || undefined}
            value={title}
            maxLength={200}
            className="h-7 text-sm"
            onChange={(event) => setTitle(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault()
                setRenaming(false)
                setTitle(item.title)
                setError(null)
              }
            }}
          />
          <Button
            type="submit"
            size="sm"
            disabled={saving || title.trim() === ""}
          >
            {saving ? <Spinner data-icon="inline-start" /> : null}
            Save
          </Button>
        </form>
        {error ? (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </li>
    )
  }

  return (
    <li
      className={cn(
        "group/row flex min-w-0 items-center gap-1 rounded-md pr-1 focus-within:bg-muted/60 hover:bg-muted/60",
        current && "bg-muted"
      )}
    >
      <button
        type="button"
        aria-current={current ? "true" : undefined}
        className="flex min-w-0 flex-1 flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onOpen}
      >
        <span className="flex w-full min-w-0 items-center gap-1.5">
          <span className="min-w-0 truncate text-sm" dir="auto" title={label}>
            {label}
          </span>
          {item.running ? (
            <Badge variant="outline" className="shrink-0">
              <Spinner data-icon="inline-start" />
              Answering
            </Badge>
          ) : null}
        </span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {when(item.updatedAtMs)} · {item.exchanges} exchange(s)
        </span>
      </button>
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label={`Rename ${label}`}
        onClick={() => {
          setTitle(item.title)
          setRenaming(true)
        }}
      >
        <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
      </Button>
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label={`Delete ${label}`}
        onClick={onDelete}
      >
        <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
      </Button>
    </li>
  )
}

/**
 * The conversations of this connection, most recent first.
 *
 * Kept while Oxyn runs, not after: nothing in the store describes a
 * conversation yet, and a file invented for it would be a closed format
 * (I-11). The list says so rather than letting a restart surprise anyone.
 */
export function AssistantHistory({
  history,
  currentId,
  onNew,
  onOpen,
  onRename,
  onDelete,
  onReload,
}: {
  history: HistoryState
  currentId: string | null
  onNew: () => void
  onOpen: (id: string) => void
  onRename: (id: string, title: string) => Promise<void>
  onDelete: (id: string) => Promise<void>
  onReload: () => void
}) {
  const [removing, setRemoving] = React.useState<ThreadSummary | null>(null)
  const [removalOpen, setRemovalOpen] = React.useState(false)
  const [deleting, setDeleting] = React.useState(false)
  const [deleteError, setDeleteError] = React.useState<string | null>(null)
  const items = "items" in history ? history.items : []

  return (
    <section
      data-slot="assistant-history"
      aria-label="Conversations"
      className="flex min-h-0 flex-1 flex-col gap-2 p-3"
    >
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-medium">Conversations</h2>
        {history.status === "loading" && items.length > 0 ? (
          <Spinner aria-label="Refreshing" />
        ) : null}
        <Button size="sm" variant="outline" className="ml-auto" onClick={onNew}>
          <HugeiconsIcon
            icon={BubbleChatAddIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Start a new conversation
        </Button>
      </div>

      {history.status === "error" ? (
        <Alert variant="destructive">
          <AlertTitle>The conversations could not be listed</AlertTitle>
          <AlertDescription data-selectable>{history.message}</AlertDescription>
          <AlertAction>
            <Button size="xs" variant="outline" onClick={onReload}>
              Try again
            </Button>
          </AlertAction>
        </Alert>
      ) : null}

      {(history.status === "loading" || history.status === "idle") &&
      items.length === 0 ? (
        <div
          role="status"
          aria-label="Loading conversations"
          className="flex flex-col gap-2"
        >
          {[0, 1, 2].map((row) => (
            <Skeleton key={row} className="h-10 w-full" />
          ))}
        </div>
      ) : null}

      {history.status === "ready" && items.length === 0 ? (
        <Empty className="border-0">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={MessageMultiple01Icon} strokeWidth={2} />
            </EmptyMedia>
            <EmptyTitle>No conversation yet</EmptyTitle>
            <EmptyDescription>
              Your conversations about this connection appear here while Oxyn
              runs.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : null}

      {items.length > 0 ? (
        <ul className="-mx-1 flex min-h-0 flex-col gap-0.5 overflow-y-auto px-1">
          {items.map((item) => (
            <Row
              key={item.id}
              item={item}
              current={item.id === currentId}
              onOpen={() => onOpen(item.id)}
              onRename={(title) => onRename(item.id, title)}
              onDelete={() => {
                setDeleteError(null)
                setRemoving(item)
                setRemovalOpen(true)
              }}
            />
          ))}
        </ul>
      ) : null}

      <p className="mt-auto text-xs text-muted-foreground">
        Conversations are kept until Oxyn quits. They are not saved to disk.
      </p>

      <AlertDialog open={removalOpen} onOpenChange={setRemovalOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Delete “{removing?.title || UNTITLED}”?
            </AlertDialogTitle>
            <AlertDialogDescription>
              Every version of every exchange goes, and an answer still running
              stops. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {deleteError ? (
            <p
              role="alert"
              className="text-sm text-destructive"
              data-selectable
            >
              {deleteError}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button
              variant="destructive"
              disabled={deleting}
              onClick={async () => {
                if (!removing) return
                setDeleting(true)
                try {
                  await onDelete(removing.id)
                  setRemovalOpen(false)
                } catch (caught) {
                  setDeleteError(
                    caught instanceof Error ? caught.message : String(caught)
                  )
                } finally {
                  setDeleting(false)
                }
              }}
            >
              {deleting ? <Spinner data-icon="inline-start" /> : null}
              Delete conversation
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
