import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { LATE_COPY_REFUSED, writeClipboard } from "./clipboard"

// jsdom 30.0.1 has neither the Clipboard API nor `ClipboardItem`: both are
// stubbed here, the item keeping what it was given.
class FakeClipboardItem {
  constructor(readonly items: Record<string, Promise<Blob>>) {}
}

let write: ReturnType<typeof vi.fn>
let writeText: ReturnType<typeof vi.fn>

beforeEach(() => {
  write = vi.fn(async (items: Array<FakeClipboardItem>) => {
    for (const item of items) await item.items["text/plain"]
  })
  writeText = vi.fn(async () => undefined)
  Object.defineProperty(navigator, "clipboard", {
    value: { write, writeText },
    configurable: true,
  })
  vi.stubGlobal("ClipboardItem", FakeClipboardItem)
})

afterEach(() => {
  Reflect.deleteProperty(navigator, "clipboard")
  vi.unstubAllGlobals()
})

async function textOf(item: FakeClipboardItem) {
  const blob = await item.items["text/plain"]
  return blob?.text()
}

describe("writeClipboard", () => {
  it("writes in the gesture a text that arrives afterwards", async () => {
    let resolve: (text: string) => void = () => undefined
    const text = new Promise<string>((done) => (resolve = done))

    const written = writeClipboard(text)

    // WebKit refuses a write that waits for an IPC round trip: the write is
    // asked now, before the text exists.
    expect(write).toHaveBeenCalledTimes(1)
    resolve('INSERT INTO "public"."orders" ("id") VALUES (?)')
    await written
    const [items] = write.mock.calls[0] as [Array<FakeClipboardItem>]
    expect(items).toHaveLength(1)
    await expect(textOf(items[0] as FakeClipboardItem)).resolves.toBe(
      'INSERT INTO "public"."orders" ("id") VALUES (?)'
    )
    expect(writeText).not.toHaveBeenCalled()
  })

  it("says why the text could not be read, not that the clipboard refused", async () => {
    write.mockRejectedValue(
      new DOMException("The request is not allowed", "NotAllowedError")
    )
    const text = Promise.reject(new Error("This result is no longer held"))

    await expect(writeClipboard(text)).rejects.toThrow(
      "This result is no longer held"
    )
  })

  it("writes a text already known as text", async () => {
    await writeClipboard('"public"."customers"')

    expect(writeText).toHaveBeenCalledWith('"public"."customers"')
    expect(write).not.toHaveBeenCalled()
  })

  it("says plainly that a late copy was refused where no item can wait", async () => {
    vi.stubGlobal("ClipboardItem", undefined)
    writeText.mockRejectedValue(
      new DOMException(
        "The request is not allowed by the user agent or the platform in the current context, possibly because the user denied permission.",
        "NotAllowedError"
      )
    )

    await expect(writeClipboard(Promise.resolve("SELECT 1"))).rejects.toThrow(
      LATE_COPY_REFUSED
    )
  })
})
