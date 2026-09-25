import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { useStore } from "@tanstack/react-store"

import { CommandPalette } from "@/components/oxyn/command-palette"
import { QuickOpen } from "@/components/oxyn/quick-open"
import type { QuickOpenResults } from "@/components/oxyn/quick-open"
import { ShortcutSheet } from "@/components/oxyn/shortcut-sheet"
import {
  NO_FOCUS,
  actionSources,
  currentContext,
  useActionSource,
} from "@/lib/actions/context"
import type { FocusState } from "@/lib/actions/context"
import { paletteSections, shortcutSheet } from "@/lib/actions/listings"
import type { PaletteSection } from "@/lib/actions/listings"
import { manifest } from "@/lib/actions/manifest"
import { platform } from "@/lib/actions/platform"
import { invoke } from "@/lib/actions/registry"
import { BackendError } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"

const SHEET = shortcutSheet(manifest, platform)

/**
 * The palette (⌘K), the quick open (⌘P) and the shortcut sheet (⌘/), once
 * for the window (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md,
 * point 7). They are opened through the registry like any action, and list
 * what the registry and the manifest hold.
 */
export function ActionOverlays() {
  const [palette, setPalette] = React.useState<{
    open: boolean
    sections: Array<PaletteSection>
    /** Where the focus was when it opened: what the entries were read for. */
    focus: FocusState
  }>({ open: false, sections: [], focus: NO_FOCUS })
  // Run once the palette has closed and handed the focus back: an action
  // that moves the focus — Go to editor — must not have it taken back.
  const chosen = React.useRef<string | null>(null)
  const [quickOpen, setQuickOpen] = React.useState(false)
  const [query, setQuery] = React.useState("")
  const [sheet, setSheet] = React.useState(false)
  const catalog = useStore(actionSources, (state) => state.sources.catalog)
  const connection = catalog?.state.connection ?? null

  useActionSource(
    "overlays",
    {},
    {
      // The context of the moment the palette opens: the focus is still
      // where the user was, and the entries say what can run there.
      openPalette: () => {
        const context = currentContext()
        setPalette({
          open: true,
          sections: paletteSections(manifest, platform, context),
          focus: {
            zones: context.zones,
            modal: false,
            focused: context.focused,
          },
        })
      },
      openQuickOpen: () => {
        setQuery("")
        setQuickOpen(true)
      },
      openShortcuts: () => setSheet(true),
    }
  )

  // The search of the catalog sidebar, under its key: the loaded catalog
  // only, bounded by the backend, never a read of the server (I-06).
  const search = query.trim()
  const hits = useQuery({
    queryKey: ["catalog-search", connection, search],
    queryFn: () => metadata.searchCatalog(connection ?? "", search),
    enabled: quickOpen && connection !== null && search !== "",
  })

  let results: QuickOpenResults = { status: "idle" }
  if (search !== "") {
    if (hits.error)
      results = {
        status: "error",
        error:
          hits.error instanceof BackendError
            ? { message: hits.error.message, retryable: hits.error.retryable }
            : { message: String(hits.error), retryable: false },
      }
    else if (hits.data) results = { status: "done", hits: hits.data }
    else results = { status: "searching" }
  }

  return (
    <>
      <CommandPalette
        open={palette.open}
        onOpenChange={(open) => setPalette((current) => ({ ...current, open }))}
        onOpenChangeComplete={(open) => {
          const id = chosen.current
          chosen.current = null
          if (open || !id) return
          // The zone the entry was chosen for, and the sources as they are
          // now: `invoke` checks the action again before running it.
          const focus = palette.focus
          requestAnimationFrame(() =>
            invoke(id, "palette", {
              ...focus,
              platform,
              sources: actionSources.state.sources,
            })
          )
        }}
        sections={palette.sections}
        onRun={(id) => {
          chosen.current = id
          setPalette((current) => ({ ...current, open: false }))
        }}
      />
      <QuickOpen
        open={quickOpen}
        onOpenChange={setQuickOpen}
        query={query}
        onQueryChange={setQuery}
        results={results}
        onRetry={() => void hits.refetch()}
        onOpenObject={(hit) => {
          setQuickOpen(false)
          catalog?.actions.openObject({
            address: hit.address,
            name: hit.name,
            kind: hit.kind,
            holdsRecords: hit.holdsRecords,
          })
        }}
      />
      <ShortcutSheet open={sheet} onOpenChange={setSheet} sections={SHEET} />
    </>
  )
}
