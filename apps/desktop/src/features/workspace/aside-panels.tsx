import * as React from "react"
import { SparklesIcon, Table01Icon, ViewIcon } from "@hugeicons/core-free-icons"

import type { AsideItem } from "@/components/oxyn/workspace-aside"
import { AssistantPanel } from "@/features/assistant/assistant-panel"
import { useAssistantAvailable } from "@/features/assistant/use-assistant-available"
import { openInConsole } from "@/features/consoles/open-in-console"
import {
  ObjectInspectorPanel,
  ValueInspectorPanel,
} from "@/features/metadata/inspectors"
import type { AgentProvenance } from "@/lib/ipc/ai"
import type { OpenConnection } from "@/lib/ipc/types"

/** What signs an answer, shown beside the console; never executed (ADR-0023). */
function noticeOf(provenance: AgentProvenance | null) {
  if (!provenance) return undefined
  return `Written by ${provenance.model}. Read it before running it.`
}

/**
 * The panels of the right column, composed here rather than inside the
 * workspace screen: the screen owns the column, each feature owns its panel.
 *
 * The assistant appears only when a provider or an agent is declared: without
 * one there is no AI entry anywhere, not even a greyed one
 * (docs/UX-SPEC.md, « Le workspace IA n'existe que s'il a été configuré »).
 */
export function useAsidePanels(open: OpenConnection | null): Array<AsideItem> {
  // The hook runs for every render, open connection or not: the assistant's
  // availability depends on the connection's tier (I-04).
  const assistant = useAssistantAvailable(
    open ?? { privacyTier: "metadata", capabilities: [] }
  )

  return React.useMemo(() => {
    if (!open) return []
    const items: Array<AsideItem> = [
      {
        id: "object",
        label: "Object",
        icon: Table01Icon,
        content: <ObjectInspectorPanel open={open} />,
      },
      {
        id: "value",
        label: "Value",
        icon: ViewIcon,
        content: <ValueInspectorPanel open={open} />,
      },
    ]
    if (assistant.status !== "absent") {
      items.push({
        id: "assistant",
        label: "Assistant",
        icon: SparklesIcon,
        content: (
          <AssistantPanel
            open={open}
            onOpenInConsole={(sql, provenance) =>
              openInConsole(sql, {
                newConsole: true,
                notice: noticeOf(provenance),
              })
            }
          />
        ),
      })
    }
    return items
  }, [open, assistant.status])
}
