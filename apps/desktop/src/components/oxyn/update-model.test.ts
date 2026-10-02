import { describe, expect, it } from "vitest"

import {
  busyLines,
  checkAvailability,
  disabledReasonText,
  formatBytes,
  formatCheckedAt,
  pendingUpdate,
  readyDays,
  statusLine,
} from "./update-model"
import type { UpdateSnapshot, UpdateState } from "@/lib/ipc/updates"

// Built from local components: the formatter reads the machine's time zone,
// and the test must not depend on the one CI runs in.
const NOW = new Date(2026, 9, 2, 16, 0)
const at = (...parts: [number, number, number, number, number]) =>
  new Date(...parts).toISOString()

function snapshot(
  state: UpdateState,
  extra: Partial<UpdateSnapshot> = {}
): UpdateSnapshot {
  return {
    state,
    currentVersion: "1.3.2",
    automatic: true,
    lastCheckedAt: null,
    ...extra,
  }
}

describe("formatBytes", () => {
  it("counts small sizes in bytes", () => {
    expect(formatBytes(0)).toBe("0 bytes")
    expect(formatBytes(1)).toBe("1 byte")
    expect(formatBytes(999)).toBe("999 bytes")
  })

  it("uses decimal units with one digit", () => {
    expect(formatBytes(12_400_000)).toBe("12.4 MB")
    expect(formatBytes(48_000_000)).toBe("48.0 MB")
    expect(formatBytes(1_500)).toBe("1.5 KB")
    expect(formatBytes(2_100_000_000)).toBe("2.1 GB")
  })

  it("does not print NaN for a size the server got wrong", () => {
    expect(formatBytes(Number.NaN)).toBe("0 bytes")
  })
})

describe("formatCheckedAt", () => {
  it("says today with the time only", () => {
    expect(formatCheckedAt(at(2026, 9, 2, 14, 32), NOW)).toBe(
      "today at 2:32 PM"
    )
  })

  it("dates another day in full", () => {
    expect(formatCheckedAt(at(2026, 8, 30, 9, 5), NOW)).toBe(
      "Sep 30, 2026, 9:05 AM"
    )
  })

  it("shows an unreadable timestamp as it came", () => {
    expect(formatCheckedAt("yesterday", NOW)).toBe("yesterday")
  })
})

describe("readyDays", () => {
  it("counts whole days", () => {
    expect(readyDays(at(2026, 8, 25, 16, 0), NOW)).toBe(7)
    expect(readyDays(at(2026, 8, 25, 16, 1), NOW)).toBe(6)
  })

  it("never goes negative when the clock moved back", () => {
    expect(readyDays(at(2026, 9, 5, 0, 0), NOW)).toBe(0)
  })
})

describe("the status line", () => {
  it("says a first check has not happened", () => {
    expect(statusLine(snapshot({ type: "idle" }), NOW)).toBe("Not checked yet.")
  })

  it("dates the last check", () => {
    expect(
      statusLine(
        snapshot({ type: "idle" }, { lastCheckedAt: at(2026, 9, 2, 14, 32) }),
        NOW
      )
    ).toBe("Last checked today at 2:32 PM.")
  })

  it("says the download starts on its own only when automatic", () => {
    const available: UpdateState = { type: "available", version: "1.4.0" }
    expect(statusLine(snapshot(available), NOW)).toBe(
      "Oxyn 1.4.0 is available. Downloading…"
    )
    expect(statusLine(snapshot(available, { automatic: false }), NOW)).toBe(
      "Oxyn 1.4.0 is available."
    )
  })

  it("reads the size of a download, known or not", () => {
    expect(
      statusLine(
        snapshot({
          type: "downloading",
          version: "1.4.0",
          received: 12_400_000,
          total: 48_000_000,
        })
      )
    ).toBe("Downloading Oxyn 1.4.0 — 12.4 MB of 48.0 MB")
    expect(
      statusLine(
        snapshot({
          type: "downloading",
          version: "1.4.0",
          received: 12_400_000,
          total: null,
        })
      )
    ).toBe("Downloading Oxyn 1.4.0 — 12.4 MB downloaded")
  })

  it("leaves a failure to the alert", () => {
    expect(
      statusLine(
        snapshot({
          type: "error",
          kind: "offline",
          message: "dns error",
          retryable: true,
          version: null,
        })
      )
    ).toBeNull()
  })
})

describe("what the indicator shows", () => {
  it("shows a downloaded update with its age", () => {
    expect(
      pendingUpdate(
        {
          type: "ready",
          version: "1.4.0",
          notes: null,
          date: null,
          readyAt: at(2026, 8, 23, 12, 0),
          installOnQuit: true,
        },
        NOW
      )
    ).toEqual({ kind: "ready", version: "1.4.0", installOnQuit: true, days: 9 })
  })

  it("stays silent on a background check that failed", () => {
    for (const kind of ["offline", "server"] as const) {
      expect(
        pendingUpdate({
          type: "error",
          kind,
          message: "timed out",
          retryable: true,
          version: null,
        })
      ).toBeNull()
    }
  })

  it("always shows a failed signature or installation", () => {
    expect(
      pendingUpdate({
        type: "error",
        kind: "signature",
        message: "bad signature",
        retryable: true,
        version: "1.4.0",
      })
    ).toEqual({
      kind: "failed",
      failure: "signature",
      version: "1.4.0",
      message: "bad signature",
    })
  })
})

describe("Check for updates…", () => {
  it("checks from a resting state and only opens during an operation", () => {
    expect(checkAvailability({ type: "idle" })).toEqual({ kind: "check" })
    expect(checkAvailability({ type: "disabled", reason: "user" })).toEqual({
      kind: "check",
    })
    expect(checkAvailability({ type: "checking" })).toEqual({ kind: "open" })
  })

  it("is greyed with the reason where Oxyn does not update itself", () => {
    expect(checkAvailability({ type: "disabled", reason: "dev" })).toEqual({
      kind: "off",
      reason: "Updates are turned off in development builds",
    })
    expect(disabledReasonText("packageManager")).toBe(
      "Updates are managed by your package manager"
    )
    expect(disabledReasonText("user")).toBeNull()
  })
})

describe("the restart confirmation", () => {
  it("names each kind of work, in the singular and the plural", () => {
    expect(busyLines(1, 0)).toEqual([
      "1 query is running. Restarting cancels it on the server.",
    ])
    expect(busyLines(3, 2)).toEqual([
      "3 queries are running. Restarting cancels them on the server.",
      "2 exports are in progress. They stop, and their destination files keep their previous content.",
    ])
    expect(busyLines(0, 0)).toEqual([])
  })
})
