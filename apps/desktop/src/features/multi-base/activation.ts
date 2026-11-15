import type { OpenConnection } from "@/lib/ipc/types"

interface Flight {
  command: string
  cancelled: boolean
  promise: Promise<void>
}
export interface ActivationPorts {
  command: () => string
  held: (connection: string) => boolean
  show: (connection: string) => void
  capacity: (pending: number) => boolean
  reconnect: (command: string, connection: string) => Promise<OpenConnection>
  adopt: (open: OpenConnection, foreground: boolean) => void
  discard: (open: OpenConnection) => Promise<void>
  cancel: (command: string) => Promise<unknown>
  changed: () => void
}

/** One window's explicit activation intents; no draft or credential is cached. */
export class WorkspaceActivations {
  private flights = new Map<string, Flight>()
  private latest: string | null = null
  private disposed = false
  constructor(private ports: ActivationPorts) {}
  mount() {
    this.disposed = false
  }
  intent(key: string) {
    this.latest = key
  }
  foreground(key: string) {
    return !this.disposed && this.latest === key
  }
  pending() {
    return [...this.flights.keys()]
  }
  canStart() {
    return this.ports.capacity(this.flights.size)
  }
  discard(open: OpenConnection) {
    return this.ports.discard(open)
  }
  status(key: string): "opening" | "cancelling" | null {
    const held = this.flights.get(key)
    return held ? (held.cancelled ? "cancelling" : "opening") : null
  }
  command(key: string) {
    return this.flights.get(key)?.command ?? null
  }
  run(
    key: string,
    work: (command: string, cancelled: () => boolean) => Promise<void>,
    command?: string
  ): Promise<void> {
    const held = this.flights.get(key)
    if (held) return held.promise
    if (this.disposed)
      return Promise.reject(new Error("This window is closing."))
    const flight: Flight = {
      command: command ?? this.ports.command(),
      cancelled: false,
      promise: Promise.resolve(),
    }
    this.flights.set(key, flight)
    flight.promise = Promise.resolve()
      .then(() =>
        flight.cancelled || this.disposed
          ? undefined
          : work(flight.command, () => flight.cancelled || this.disposed)
      )
      .finally(() => {
        if (this.flights.get(key) === flight) this.flights.delete(key)
        if (!this.disposed) this.ports.changed()
      })
    this.ports.changed()
    return flight.promise
  }
  activate(connection: string) {
    const key = `connection:${connection}`
    this.intent(key)
    if (this.ports.held(connection)) {
      this.ports.show(connection)
      return Promise.resolve()
    }
    const existing = this.flights.get(key)
    if (existing) return existing.promise
    if (!this.ports.capacity(this.flights.size))
      return Promise.reject(
        new Error(
          "Workspace limit reached. Disconnect a connection before opening another."
        )
      )
    return this.run(key, async (command, cancelled) => {
      const open = await this.ports.reconnect(command, connection)
      if (cancelled()) await this.ports.discard(open)
      else this.ports.adopt(open, this.foreground(key))
    })
  }
  async cancel(key: string) {
    const flight = this.flights.get(key)
    if (!flight || flight.cancelled) return
    flight.cancelled = true
    this.ports.changed()
    await this.ports.cancel(flight.command)
  }
  dispose() {
    this.disposed = true
    for (const flight of this.flights.values()) {
      flight.cancelled = true
      void this.ports.cancel(flight.command).catch(() => undefined)
    }
  }
}
