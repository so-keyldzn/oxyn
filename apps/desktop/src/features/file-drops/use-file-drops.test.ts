import { describe, expect, it, vi } from "vitest"

import { DroppedFile } from "@/lib/ipc/file-drops"
import { receiveDrop } from "./use-file-drops"

function effects(connected: boolean) {
  return {
    connected: () => connected,
    openInConsole: vi.fn(),
    offerConnection: vi.fn(),
    showConnectionScreen: vi.fn(),
    warn: vi.fn(),
  }
}

describe("a dropped file", () => {
  it("opens a .sql file in a new console, and runs nothing", () => {
    const done = effects(true)
    receiveDrop(
      { type: "sql", name: "purge.sql", text: "DELETE FROM audit;" },
      done
    )
    expect(done.openInConsole).toHaveBeenCalledWith("DELETE FROM audit;", {
      newConsole: true,
      notice: "Opened from purge.sql. Nothing was executed.",
    })
  })

  it("keeps a .sql file out of a queue while no connection is open", () => {
    const done = effects(false)
    receiveDrop({ type: "sql", name: "purge.sql", text: "SELECT 1;" }, done)
    expect(done.openInConsole).not.toHaveBeenCalled()
    expect(done.warn).toHaveBeenCalledWith(
      "purge.sql was not opened",
      expect.stringMatching(/Open a connection first/)
    )
  })

  it("offers a database file as a connection, and creates none", () => {
    const done = effects(true)
    receiveDrop(
      {
        type: "database",
        name: "scratch.sqlite",
        driver: "sqlite",
        field: "path",
        path: "/Users/me/scratch.sqlite",
      },
      done
    )
    expect(done.offerConnection).toHaveBeenCalledWith({
      driver: "sqlite",
      field: "path",
      path: "/Users/me/scratch.sqlite",
    })
    expect(done.showConnectionScreen).toHaveBeenCalled()
  })

  it("says why a file was refused", () => {
    const done = effects(true)
    receiveDrop(
      { type: "refused", name: "notes.sql", reason: "It is a symbolic link." },
      done
    )
    expect(done.warn).toHaveBeenCalledWith(
      "notes.sql was not opened",
      "It is a symbolic link."
    )
  })

  it("is read by the schema Rust serializes, and nothing looser", () => {
    expect(
      DroppedFile.safeParse({ type: "sql", name: "a.sql", text: "SELECT 1" })
        .success
    ).toBe(true)
    // A path where a text is expected is not read as a text.
    expect(
      DroppedFile.safeParse({ type: "sql", name: "a.sql", path: "/etc/passwd" })
        .success
    ).toBe(false)
    expect(
      DroppedFile.safeParse({ type: "run", name: "a.sql", text: "DROP" })
        .success
    ).toBe(false)
  })
})
