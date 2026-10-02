// The words of automatic updates (ADR-0051, docs/UX-SPEC.md « Updates »),
// written once: the menu, Settings ▸ Updates, the indicator and the toasts
// read them here, so a state never reads two ways depending on where it shows.

import type {
  DisabledReason,
  UpdateErrorKind,
  UpdateSnapshot,
  UpdateState,
} from "@/lib/ipc/updates"

/** After this many days, the indicator gains a border (UX-SPEC « Updates »). */
export const STALE_AFTER_DAYS = 7

const DAY_MS = 24 * 60 * 60 * 1000

/** « 512 bytes », « 12.4 MB »: decimal units, as a download size reads. */
export function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes < 1000) {
    const whole = Math.max(0, Math.round(Number.isFinite(bytes) ? bytes : 0))
    return `${whole.toLocaleString("en-US")} ${whole === 1 ? "byte" : "bytes"}`
  }
  const units = ["KB", "MB", "GB", "TB"]
  let value = bytes / 1000
  let unit = 0
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

const TIME = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
})
const DATE_TIME = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
})

/**
 * « today at 2:32 PM », otherwise « Oct 2, 2026, 2:32 PM », in the machine's
 * time zone. An unreadable timestamp is shown as it came rather than as
 * « Invalid Date ».
 */
export function formatCheckedAt(iso: string, now: Date = new Date()) {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate()
  return sameDay ? `today at ${TIME.format(date)}` : DATE_TIME.format(date)
}

/** Whole days since `readyAt`; `0` for a clock set back or a bad timestamp. */
export function readyDays(readyAt: string, now: Date = new Date()) {
  const at = new Date(readyAt).getTime()
  if (Number.isNaN(at)) return 0
  return Math.max(0, Math.floor((now.getTime() - at) / DAY_MS))
}

/** « Downloaded 9 days ago. » */
export function downloadedAgo(days: number) {
  return `Downloaded ${days} ${days === 1 ? "day" : "days"} ago.`
}

/**
 * Why Oxyn does not update itself here, or `null` when it can — automatic
 * updates turned off by the user leave `Check now` available.
 */
export function disabledReasonText(reason: DisabledReason): string | null {
  switch (reason) {
    case "user":
      return null
    case "admin":
      return "Updates are turned off by your administrator"
    case "packageManager":
      return "Updates are managed by your package manager"
    case "dev":
      return "Updates are turned off in development builds"
  }
}

/** A state from which no update action exists in this installation. */
export function isLocked(state: UpdateState) {
  return state.type === "disabled" && state.reason !== "user"
}

/** What `Check for updates…` does from this state. */
export function checkAvailability(
  state: UpdateState
): { kind: "check" } | { kind: "open" } | { kind: "off"; reason: string } {
  switch (state.type) {
    case "idle":
    case "upToDate":
    case "error":
      return { kind: "check" }
    case "checking":
    case "available":
    case "downloading":
    case "ready":
      // One operation at a time: the view shows the one under way.
      return { kind: "open" }
    case "disabled": {
      const reason = disabledReasonText(state.reason)
      return reason === null ? { kind: "check" } : { kind: "off", reason }
    }
  }
}

/** What the passive indicator shows, or `null` when nothing is pending. */
export type PendingUpdate =
  | {
      kind: "ready"
      version: string
      installOnQuit: boolean
      days: number
    }
  | {
      kind: "failed"
      failure: "signature" | "install"
      version: string | null
      message: string
    }

/**
 * Only a downloaded update and a failure the user must know of show outside
 * Settings: a background check that could not reach the server stays silent.
 */
export function pendingUpdate(
  state: UpdateState,
  now: Date = new Date()
): PendingUpdate | null {
  if (state.type === "ready")
    return {
      kind: "ready",
      version: state.version,
      installOnQuit: state.installOnQuit,
      days: readyDays(state.readyAt, now),
    }
  if (
    state.type === "error" &&
    (state.kind === "signature" || state.kind === "install")
  )
    return {
      kind: "failed",
      failure: state.kind,
      version: state.version,
      message: state.message,
    }
  return null
}

/** « Oxyn 1.4.0 », or « The update » before a version was announced. */
function named(version: string | null) {
  return version === null ? "The update" : `Oxyn ${version}`
}

export const READY_BODY =
  "It installs when you quit Oxyn. Oxyn never restarts on its own."
export const READY_BODY_NOT_ON_QUIT =
  "Oxyn can't install it when you quit from this location. Restart now to install it; macOS may ask for an administrator password."

/** The popover's title of a failure. */
export function failedTitle(
  failure: "signature" | "install",
  version: string | null
) {
  return failure === "signature"
    ? `${named(version)} could not be verified`
    : `${named(version)} could not be installed`
}

/** The ready toast, the first time a version is downloaded. */
export function readyToastTitle(version: string) {
  return `Oxyn ${version} is ready. It installs when you quit.`
}

/** The title of a failure in Settings ▸ Updates. */
export function errorTitle(
  kind: UpdateErrorKind,
  version: string | null
): string {
  switch (kind) {
    case "offline":
      return "Oxyn could not reach the update server"
    case "server":
      return "The update server answered with an error"
    case "signature":
      return `${named(version)} failed signature verification and was discarded`
    case "install":
      return `${named(version)} could not be installed`
  }
}

/** What to do next after a failure, under the server's message. */
export function errorNextStep(kind: UpdateErrorKind, current: string) {
  switch (kind) {
    case "offline":
      return "Check your network connection."
    case "server":
      return undefined
    case "signature":
      return "Nothing was installed. Download Oxyn from the release page and check it before installing."
    case "install":
      return `Oxyn ${current} keeps working.`
  }
}

/**
 * The status line of Settings ▸ Updates. A failure is not a line: it is an
 * alert with the server's words, and this returns `null` for it.
 */
export function statusLine(
  snapshot: UpdateSnapshot,
  now: Date = new Date()
): string | null {
  const { state, currentVersion, automatic, lastCheckedAt } = snapshot
  switch (state.type) {
    case "idle":
      return lastCheckedAt === null
        ? "Not checked yet."
        : `Last checked ${formatCheckedAt(lastCheckedAt, now)}.`
    case "checking":
      return "Checking for updates…"
    case "upToDate":
      return `Oxyn ${currentVersion} is the latest version. Last checked ${formatCheckedAt(state.checkedAt, now)}.`
    case "available":
      return automatic
        ? `Oxyn ${state.version} is available. Downloading…`
        : `Oxyn ${state.version} is available.`
    case "downloading":
      return `Downloading Oxyn ${state.version} — ${downloadedBytes(state.received, state.total)}`
    case "ready":
      return state.installOnQuit
        ? `Oxyn ${state.version} is ready. It installs when you quit.`
        : `Oxyn ${state.version} is ready. Oxyn can't install it when you quit from this location: restart now to install it.`
    case "error":
      return null
    case "disabled":
      switch (state.reason) {
        case "user":
          return "Automatic updates are off. Oxyn checks only when you ask."
        case "admin":
          return "Updates are turned off by your administrator."
        case "packageManager":
          return "Updates are managed by your package manager. Update Oxyn with the tool you installed it with (apt, dnf…)."
        case "dev":
          return "Updates are turned off in development builds."
      }
  }
}

/** « 12.4 MB of 48.0 MB », or « 12.4 MB downloaded » without a total. */
export function downloadedBytes(received: number, total: number | null) {
  return total === null
    ? `${formatBytes(received)} downloaded`
    : `${formatBytes(received)} of ${formatBytes(total)}`
}

/**
 * The part of the status line the polite live region holds: the state, never
 * the bytes — a counter read aloud four times a second drowns the user.
 */
export function liveStatus(snapshot: UpdateSnapshot, now: Date = new Date()) {
  return snapshot.state.type === "downloading"
    ? `Downloading Oxyn ${snapshot.state.version}`
    : statusLine(snapshot, now)
}

/** « Restarting cancels it on the server. », per what is running. */
export function busyLines(running: number, exports: number): Array<string> {
  const lines: Array<string> = []
  if (running === 1)
    lines.push("1 query is running. Restarting cancels it on the server.")
  else if (running > 1)
    lines.push(
      `${running} queries are running. Restarting cancels them on the server.`
    )
  if (exports === 1)
    lines.push(
      "1 export is in progress. It stops, and the destination file keeps its previous content."
    )
  else if (exports > 1)
    lines.push(
      `${exports} exports are in progress. They stop, and their destination files keep their previous content.`
    )
  return lines
}
