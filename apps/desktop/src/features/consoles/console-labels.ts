import { createStore } from "@tanstack/react-store"

// The title of each console, by its session id. The backend knows sessions,
// not documents: the exit dialog names the consoles from here (ADR-0043).
// Only a label — the list itself always comes from the backend.

export const consoleLabels = createStore<Record<string, string>>({})

/** The consoles of one connection, as they stand; replaces what it owned. */
export function publishConsoleLabels(
  owned: ReadonlyArray<string>,
  labels: Record<string, string>
) {
  consoleLabels.setState((all) => {
    const next = { ...all }
    for (const session of owned) delete next[session]
    return { ...next, ...labels }
  })
}
