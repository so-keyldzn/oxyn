// Server navigation, explicit child openings and scoped cleanup through the bus.
import { z } from "zod"

import { call, Nothing } from "./client"
import { ConnectResponse } from "./types"
import type { OpenConnection, ConnectionDraft } from "./types"

export const ServerProfile = z.object({
  id: z.string().uuid(),
  name: z.string(),
  driver: z.string(),
  params: z.record(z.string(), z.string()),
})
export type ServerProfile = z.infer<typeof ServerProfile>

export const ServerProfileSnapshot = z.object({
  profile: ServerProfile,
  revision: z.number().int().positive(),
  connections: z.array(z.string().uuid()),
})
export type ServerProfileSnapshot = z.infer<typeof ServerProfileSnapshot>

export const DiscoveredDatabase = z.object({
  name: z.string(),
  can_connect: z.boolean(),
})
export type DiscoveredDatabase = z.infer<typeof DiscoveredDatabase>

export const serverProfiles = {
  disconnectReady: (commandId: string, connection: string) =>
    call("connection_disconnect_ready", Nothing, { commandId, connection }),
  discard: (open: OpenConnection) =>
    call("discard_database_opening", Nothing, {
      connection: open.connection,
      catalog: open.session,
      console: open.console.session,
    }),
  discover: (commandId: string, connection: string) =>
    call("discover_databases", z.array(DiscoveredDatabase), {
      commandId,
      connection,
    }),
  open: (
    commandId: string,
    profile: string,
    seed: string,
    draft: ConnectionDraft
  ) =>
    call("open_database", ConnectResponse, { commandId, profile, seed, draft }),
  list: (commandId: string) =>
    call("list_server_profiles", z.array(ServerProfileSnapshot), { commandId }),
  save: (
    commandId: string,
    profile: ServerProfile,
    expectedRevision: number | null
  ) =>
    call("save_server_profile", ServerProfileSnapshot, {
      commandId,
      profile,
      expectedRevision,
    }),
  remove: (commandId: string, profile: string, expectedRevision: number) =>
    call("remove_server_profile", Nothing, {
      commandId,
      profile,
      expectedRevision,
    }),
  setMembership: (
    commandId: string,
    connection: string,
    profile: string | null,
    expectedProfile: string | null
  ) =>
    call("set_server_profile_membership", Nothing, {
      commandId,
      connection,
      profile,
      expectedProfile,
    }),
}
