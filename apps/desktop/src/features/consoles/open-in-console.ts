import { createStore } from "@tanstack/react-store"

/** Text to drop into a console without running it. */
export interface ConsoleTextRequest {
  id: number
  sql: string
  /** A new console rather than the active one. */
  newConsole: boolean
  /** Why this text is here, shown next to the console. Never executed. */
  notice: string | null
}

// A queue rather than a callback prop: the assistant and the inspector are
// mounted by the integrator, beside the workspace screen and not inside it.
export const consoleTextRequests = createStore<Array<ConsoleTextRequest>>([])

let next = 0

/**
 * Puts SQL in the active console of the open workspace — or a new one — and
 * focuses it. Nothing runs: the user reads it, then decides (I-07).
 */
export function openInConsole(
  sql: string,
  options: { newConsole?: boolean; notice?: string } = {}
) {
  next += 1
  consoleTextRequests.setState((requests) => [
    ...requests,
    {
      id: next,
      sql,
      newConsole: options.newConsole ?? false,
      notice: options.notice ?? null,
    },
  ])
}

export function takeConsoleTextRequests() {
  const requests = consoleTextRequests.state
  if (requests.length > 0) consoleTextRequests.setState(() => [])
  return requests
}
