import { z } from "zod"

/**
 * The licenses of the Rust and npm packages the application ships, written at
 * build time by `script/licences-tierces` (ADR-0044). One notice per distinct
 * license text, with the packages it covers.
 */
export const ThirdPartyNotice = z.object({
  /** An SPDX expression, e.g. `MIT` or `(MPL-2.0 OR Apache-2.0)`. */
  license: z.string(),
  name: z.string(),
  text: z.string(),
  packages: z.array(
    z.object({
      ecosystem: z.enum(["cargo", "npm"]),
      name: z.string(),
      version: z.string(),
    })
  ),
})
export type ThirdPartyNotice = z.infer<typeof ThirdPartyNotice>

const ThirdPartyLicenses = z.object({
  version: z.literal(1),
  notices: z.array(ThirdPartyNotice),
})

// A glob rather than an import: the file is generated outside git, and a
// build without `cargo-about` has none. The glob then matches nothing, where
// an import would fail the build. It is also a separate chunk, loaded only
// when the About section opens: over a megabyte of license text.
const generated = import.meta.glob<unknown>(
  "../generated/third-party-licenses.json",
  { import: "default" }
)

/** The notices of this build, or `null` if the build was made without them. */
export async function loadThirdPartyLicenses(): Promise<Array<ThirdPartyNotice> | null> {
  const load = Object.values(generated)[0]
  if (!load) return null
  return ThirdPartyLicenses.parse(await load()).notices
}
