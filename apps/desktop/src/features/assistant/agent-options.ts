import type { AgentOption } from "@/lib/ipc/ai"

/** The shipped identity is stable, including in pre-picker conversations. */
export const SQL_AGENT_ID = "0199a3c0-0000-7000-8000-000000000001"

export const SQL_ONLY: ReadonlyArray<AgentOption> = [
  {
    id: SQL_AGENT_ID,
    name: "SQL",
    description: "Writes, fixes and explains queries on the open connection.",
    origin: "shipped",
    error: null,
  },
]
