import { cn } from "cn"

import type { Cell } from "@/lib/ipc/types"

/** The text a cell copies to the clipboard: what is shown, as shown. */
export function cellText(cell: Cell): string {
  if (cell === null) return ""
  if (typeof cell === "string") return cell
  if ("text" in cell) return cell.text
  return ""
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/**
 * One formatted cell. Rendered as text only — never as HTML — whatever the
 * server sent: a cell value is the first place an XSS in the webview would
 * come from (docs/adr/0029-interface-tauri-shadcn.md).
 *
 * An absent value is `∅ NULL` in its own token, never the word alone: a text
 * column may literally contain « NULL ». A value Rust cut keeps its text and a
 * size marker; the CSS ellipsis is then the only ellipsis drawn.
 */
export function CellValue({
  cell,
  className,
  title,
}: {
  cell: Cell
  className?: string
  /** The full text, when the caller knows the column is too narrow for it. */
  title?: string
}) {
  if (cell === null) {
    return (
      <span
        className={cn("shrink-0 text-null select-none", className)}
        data-null
      >
        ∅ NULL
      </span>
    )
  }
  if (typeof cell === "string") {
    return (
      <span className={cn("min-w-0 truncate", className)} title={title}>
        {cell}
      </span>
    )
  }
  if ("text" in cell) {
    const full = `Truncated · ${formatBytes(cell.fullBytes)} in full`
    return (
      <span
        className={cn("flex min-w-0 items-center gap-1", className)}
        title={full}
        data-truncated
      >
        <span className="min-w-0 truncate">{cell.text}</span>
        <span className="shrink-0 rounded-sm bg-muted px-1 font-sans text-[length:var(--reading-caption)] leading-4 text-muted-foreground tabular-nums">
          {formatBytes(cell.fullBytes)}
        </span>
      </span>
    )
  }
  return (
    <span
      className={cn("min-w-0 truncate text-warning", className)}
      title={cell.unrenderable}
      data-unrenderable
    >
      ⟨unrenderable⟩
    </span>
  )
}
