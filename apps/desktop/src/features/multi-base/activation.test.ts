import { describe, expect, it, vi } from "vitest"
import type { OpenConnection } from "@/lib/ipc/types"
import { WorkspaceActivations } from "./activation"

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((a, b) => {
    resolve = a
    reject = b
  })
  return { promise, resolve, reject }
}
const open = (connection: string, session = connection) =>
  ({
    connection,
    session,
    console: { session: `${session}-console` },
  }) as OpenConnection
function setup() {
  const ports = {
    command: vi.fn(() => "command"),
    held: vi.fn((_id: string) => false),
    show: vi.fn(),
    capacity: vi.fn(() => true),
    reconnect: vi.fn(async (_id: string, connection: string) =>
      open(connection)
    ),
    adopt: vi.fn(),
    discard: vi.fn(async (_open: OpenConnection) => {}),
    cancel: vi.fn(async (_id: string) => {}),
    changed: vi.fn(),
  }
  return { ports, flight: new WorkspaceActivations(ports) }
}
describe("window workspace activations", () => {
  it("shares one opening for repeated activations", async () => {
    const { ports, flight } = setup()
    const result = deferred<OpenConnection>()
    ports.reconnect.mockImplementation(() => result.promise)
    const first = flight.activate("a")
    const repeated = flight.activate("a")
    expect(repeated).toBe(first)
    await Promise.resolve()
    expect(ports.reconnect).toHaveBeenCalledTimes(1)
    expect(ports.command).toHaveBeenCalledTimes(1)
    result.resolve(open("a"))
    await first
    expect(ports.adopt).toHaveBeenCalledExactlyOnceWith(open("a"), true)
  })
  it("keeps a later explicit target in front when an older response arrives", async () => {
    const { ports, flight } = setup()
    const a = deferred<OpenConnection>()
    const b = deferred<OpenConnection>()
    ports.reconnect.mockImplementation((_id, id) =>
      id === "a" ? a.promise : b.promise
    )
    const first = flight.activate("a")
    const second = flight.activate("b")
    a.resolve(open("a"))
    await first
    expect(ports.adopt).toHaveBeenLastCalledWith(open("a"), false)
    b.resolve(open("b"))
    await second
    expect(ports.adopt).toHaveBeenLastCalledWith(open("b"), true)
  })
  it("A to B to A reveals the retained identity without reconnecting", async () => {
    const { ports, flight } = setup()
    ports.held.mockImplementation((id) => id === "a")
    await flight.activate("a")
    await flight.activate("b")
    await flight.activate("a")
    expect(ports.show).toHaveBeenCalledTimes(2)
    expect(ports.reconnect).toHaveBeenCalledTimes(1)
  })
  it("cancellation discards only the returned opening and never adopts it", async () => {
    const { ports, flight } = setup()
    const late = deferred<OpenConnection>()
    ports.reconnect.mockImplementation(() => late.promise)
    const waiting = flight.activate("a")
    await Promise.resolve()
    await flight.cancel("connection:a")
    await flight.cancel("connection:a")
    late.resolve(open("a", "late"))
    await waiting
    expect(ports.cancel).toHaveBeenCalledTimes(1)
    expect(ports.discard).toHaveBeenCalledExactlyOnceWith(open("a", "late"))
    expect(ports.adopt).not.toHaveBeenCalled()
  })
  it("a pre-start cancellation sends no opening", async () => {
    const { ports, flight } = setup()
    const waiting = flight.activate("a")
    await flight.cancel("connection:a")
    await waiting
    expect(ports.reconnect).not.toHaveBeenCalled()
  })
  it("capacity rejects without evicting or sending anything", async () => {
    const { ports, flight } = setup()
    ports.capacity.mockReturnValue(false)
    await expect(flight.activate("ninth")).rejects.toThrow("limit")
    expect(ports.reconnect).not.toHaveBeenCalled()
    expect(ports.discard).not.toHaveBeenCalled()
  })
  it("same database labels in distinct profiles/accounts remain distinct intents", async () => {
    const { flight } = setup()
    const first = vi.fn(async () => {})
    const second = vi.fn(async () => {})
    await Promise.all([
      flight.run(JSON.stringify(["server-a", "orders", "account-a"]), first),
      flight.run(JSON.stringify(["server-b", "orders", "account-b"]), second),
    ])
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(1)
  })
  it("a failed call is not retried but can be requested explicitly again", async () => {
    const { ports, flight } = setup()
    ports.reconnect.mockRejectedValueOnce(new Error("refused"))
    await expect(flight.activate("a")).rejects.toThrow("refused")
    expect(ports.reconnect).toHaveBeenCalledTimes(1)
    await flight.activate("a")
    expect(ports.reconnect).toHaveBeenCalledTimes(2)
  })
})
