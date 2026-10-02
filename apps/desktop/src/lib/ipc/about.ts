// Mirror of `crates/oxyn-desktop/src/ipc/about.rs`, as executable schemas
// (ADR-0031). Both sides change in the same commit.

import { z } from "zod"

import { call } from "./client"

export const BuildProfile = z.enum(["development", "release"])
export type BuildProfile = z.infer<typeof BuildProfile>

/** Which build is running; `null` is what the build could not record. */
export const BuildIdentity = z.object({
  version: z.string(),
  /** The commit the sources were at, captured when the binary was built. */
  revision: z.string().nullable(),
  /** Whether tracked files differed from that commit. */
  modified: z.boolean().nullable(),
  os: z.string(),
  arch: z.string(),
  profile: BuildProfile,
})
export type BuildIdentity = z.infer<typeof BuildIdentity>

export const aboutBackend = {
  buildIdentity: () => call("build_identity", BuildIdentity),
}
