import type { BuildIdentity } from "@/lib/ipc/about"

/** What the screen and the copied text say for what the build did not record. */
export const UNKNOWN = "Unknown"

/**
 * The lines of Settings → About's identity block, and of the text its
 * `Copy diagnostic info` writes: the same lines, so what a bug report quotes
 * is what the user saw.
 *
 * Each line names its field: the text goes into a public issue, and a
 * connection, a path or a variable must not reach it (I-03) — a field added
 * to `BuildIdentity` reaches neither the screen nor the clipboard until a
 * line here names it.
 */
export function diagnosticRows(
  identity: BuildIdentity | null
): Array<[label: string, value: string]> {
  const modified = identity?.modified
  return [
    ["Version", identity?.version ?? UNKNOWN],
    ["Source revision", identity?.revision ?? UNKNOWN],
    [
      "Modified sources",
      modified === true ? "Yes" : modified === false ? "No" : UNKNOWN,
    ],
    ["Platform", identity ? `${identity.os} ${identity.arch}` : UNKNOWN],
    [
      "Build",
      identity
        ? identity.profile === "release"
          ? "Release"
          : "Development"
        : UNKNOWN,
    ],
  ]
}

/** Plain text for a GitHub issue; composed here, without any request. */
export function diagnosticText(identity: BuildIdentity | null): string {
  return [
    "Oxyn",
    ...diagnosticRows(identity).map(([label, value]) => `${label}: ${value}`),
  ].join("\n")
}
