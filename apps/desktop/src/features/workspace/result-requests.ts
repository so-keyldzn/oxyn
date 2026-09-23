import { createStore } from "@tanstack/react-store"

/** A retained result to open in a workspace's tab. Nothing runs. */
export interface ResultOpenRequest {
  id: number
  /** The workspace it belongs to: another connection's screen ignores it. */
  connection: string
  result: string
  /** The statement that produced it, for « open the statement in a console ». */
  statement: string | null
}

// A queue rather than a callback prop, like `openObject`: the assistant is
// mounted in the right column, beside the workspace screen that owns the tabs.
export const resultOpenRequests = createStore<Array<ResultOpenRequest>>([])

let next = 0

/**
 * Opens a result an agent's query left, in the workspace of `connection`: the
 * retained rows, read by pages. The query is never run again.
 */
export function openAgentResult(
  connection: string,
  result: string,
  statement: string | null
) {
  next += 1
  resultOpenRequests.setState((requests) => [
    ...requests,
    { id: next, connection, result, statement },
  ])
}

/** The requests for `connection`, taken out of the queue; the rest stay. */
export function takeResultOpenRequests(connection: string) {
  const mine = resultOpenRequests.state.filter(
    (request) => request.connection === connection
  )
  if (mine.length > 0)
    resultOpenRequests.setState((requests) =>
      requests.filter((request) => request.connection !== connection)
    )
  return mine
}
