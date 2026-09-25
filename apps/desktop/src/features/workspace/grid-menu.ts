import * as React from "react"

import { PRIVACY_TIERS } from "@/components/oxyn/privacy-tier"
import type {
  GridMenuSource,
  RowsCopy,
} from "@/components/oxyn/result-grid-menu"
import type { AssistantEntry } from "@/features/assistant/availability"
import { useAssistantAvailable } from "@/features/assistant/use-assistant-available"
import { copyToClipboard } from "@/features/metadata/clipboard"
import type { GridAiLevel, GridOrigin } from "@/lib/actions/targets"
import { results } from "@/lib/ipc/results"
import type {
  CatalogAddress,
  OpenConnection,
  PrivacyTier,
} from "@/lib/ipc/types"

const NO_CONNECTION: Pick<OpenConnection, "privacyTier" | "capabilities"> = {
  privacyTier: "metadata",
  capabilities: [],
}

/**
 * The AI level `Send to assistant` reads (I-04): without a declared
 * destination the entry does not exist; under `Sampled` values may reach a
 * model through the sample approval; any other tier is named in the reason.
 * `tier` is null when the connection is not known: nothing is offered.
 * Pure, so it is tested.
 */
export function gridAiLevel(
  assistant: AssistantEntry["status"],
  tier: PrivacyTier | null
): GridAiLevel {
  if (assistant === "absent" || tier === null) return { kind: "none" }
  if (tier === "sampled") return { kind: "sampled" }
  return { kind: "withheld", label: PRIVACY_TIERS[tier].label }
}

/**
 * What the grid's context menus need from a result the backend holds.
 *
 * Every copy of rows is read and formatted in Rust (`copy_result_rows`),
 * bounded to one read, and reaches the clipboard through `copyToClipboard`;
 * nothing composes SQL here (I-10). `address` is the relation a preview reads,
 * the one `INSERT` names; a query's rows have none.
 */
export function useGridMenu({
  open,
  connection,
  result,
  origin,
  address,
  filterable = false,
  sortable = false,
  sort,
  filter,
}: {
  /** The workspace's connection, for its AI level; null when not known. */
  open: Pick<OpenConnection, "privacyTier" | "capabilities"> | null
  /** Where the result was read: the dialect of `INSERT` and `IN list`. */
  connection: string
  result: string | null
  origin: GridOrigin
  address: CatalogAddress | null
  /** What the preview declares; a query's rows are neither. */
  filterable?: boolean
  sortable?: boolean
  sort?: GridMenuSource["sort"]
  filter?: GridMenuSource["filter"]
}): GridMenuSource | undefined {
  const assistant = useAssistantAvailable(open ?? NO_CONNECTION).status
  const tier = open?.privacyTier ?? null
  return React.useMemo(() => {
    if (result === null) return undefined
    // The rows are read after the click, the clipboard written in it
    // (clipboard.ts). A refusal of the backend names its reason — a limit,
    // an expired result, a relation it cannot name — in the copy's toast.
    const copyRows = (request: RowsCopy) =>
      void copyToClipboard(
        results
          .copyResultRows({
            result,
            offset: request.offset,
            count: request.count,
            columns: request.columns,
            format: request.format,
            header: request.header,
            connection,
            address,
          })
          .then((copied) => copied.text),
        request.what
      )
    return {
      origin,
      relation: address !== null,
      ai: gridAiLevel(assistant, tier),
      copyText: (text, what) => void copyToClipboard(text, what),
      copyRows,
      filterable,
      sortable,
      sort,
      filter,
    }
  }, [
    result,
    connection,
    address,
    origin,
    assistant,
    tier,
    filterable,
    sortable,
    sort,
    filter,
  ])
}
