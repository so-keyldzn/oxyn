export interface ConsoleDestination {
  connection: string
  workspace: string
  session: string | null
}
const owners = new Map<
  string,
  { workspace: string; current: () => string | null }
>()
/** A retained workspace publishes its own console generation, never its text. */
export function registerConsoleDestination(
  connection: string,
  workspace: string,
  current: () => string | null
) {
  const owner = { workspace, current }
  owners.set(connection, owner)
  return () => {
    if (owners.get(connection) === owner) owners.delete(connection)
  }
}
export function consoleDestination(
  connection: string
): ConsoleDestination | null {
  const owner = owners.get(connection)
  return owner
    ? { connection, workspace: owner.workspace, session: owner.current() }
    : null
}
