import { copyToClipboard } from "@/features/metadata/clipboard"
import type { ConnectionSummary } from "@/lib/ipc/settings"

/**
 * What `Copy connection` puts on the clipboard: what the list already shows,
 * read field by field.
 *
 * Never a spread of the summary: a field added to it later — a secret's
 * reference, the connection's id — would reach the clipboard without anyone
 * deciding it should (I-03). The location is the summary's own, built by the
 * backend from declared non-secret values only.
 */
export function connectionText(connection: ConnectionSummary): string {
  const lines = [
    `Name: ${connection.name}`,
    `Driver: ${connection.driverName}`,
    `Environment: ${connection.environment}`,
  ]
  if (connection.location) lines.push(`Location: ${connection.location}`)
  if (connection.readOnly) lines.push("Read only: yes")
  return lines.join("\n")
}

/** `Copy connection` of the saved connections' context menu. */
export function copyConnection(connection: ConnectionSummary) {
  return copyToClipboard(connectionText(connection), "Connection")
}
