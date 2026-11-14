import { consoleDestination } from "./text-destination"
import { createStore } from "@tanstack/react-store"

/** Text to drop into a console without running it. */
export interface ConsoleTextRequest {
  connection: string
  workspace: string | null
  session: string | null
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
  connection: string,
  sql: string,
  options: {
    newConsole?: boolean
    notice?: string
    workspace?: string
    session?: string | null
  } = {}
) {
  const destination = consoleDestination(connection)
  if (!options.workspace && !destination) return
  const targetSession =
    options.session === undefined
      ? (destination?.session ?? null)
      : options.session
  next += 1
  consoleTextRequests.setState((requests) => [
    ...requests,
    {
      id: next,
      connection,
      workspace: options.workspace ?? destination?.workspace ?? null,
      session: options.newConsole ? null : targetSession,
      sql,
      newConsole: options.newConsole ?? targetSession === null,
      notice: options.notice ?? null,
    },
  ])
}

export function takeConsoleTextRequests(connection?: string) {
  const requests = consoleTextRequests.state.filter(
    (request) => connection === undefined || request.connection === connection
  )
  if (requests.length > 0)
    consoleTextRequests.setState((all) =>
      all.filter((request) => !requests.includes(request))
    )
  return requests
}
