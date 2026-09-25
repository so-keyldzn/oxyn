import * as React from "react"

import {
  Command,
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import type { PaletteEntry, PaletteSection } from "@/lib/actions/listings"
import { TEXT_FIELD_ATTRIBUTES } from "./text-field"

/**
 * The sections holding `search` in an entry's label or menu: a substring, not
 * cmdk's fuzzy default — « run » scattered across « Return to connections »
 * would rank it, and Enter would run the wrong action.
 */
function matching(sections: Array<PaletteSection>, search: string) {
  const needle = search.trim().toLocaleLowerCase()
  if (!needle) return sections
  return sections.flatMap((section) => {
    const entries = section.entries.filter((entry) =>
      [entry.label, entry.section].some((text) =>
        text.toLocaleLowerCase().includes(needle)
      )
    )
    return entries.length > 0 ? [{ ...section, entries }] : []
  })
}

function Entry({
  entry,
  onRun,
}: {
  entry: PaletteEntry
  onRun: (id: string) => void
}) {
  const reason = entry.availability === true ? null : entry.availability.reason
  return (
    <CommandItem
      value={entry.id}
      // Not cmdk's `disabled`: a disabled item is skipped by the arrows, and
      // its reason would never be read by keyboard (UX-SPEC, « Une action,
      // un libellé, un raccourci »). It is read with the entry instead, and
      // choosing it does nothing.
      data-unavailable={reason !== null || undefined}
      data-checked={entry.checked ?? undefined}
      onSelect={() => {
        if (reason === null) onRun(entry.id)
      }}
    >
      <span className="flex min-w-0 flex-col">
        <span className={reason ? "text-muted-foreground" : undefined}>
          {entry.label}
          {entry.checked ? <span className="sr-only"> (current)</span> : null}
        </span>
        {reason ? (
          <span className="text-xs text-muted-foreground">
            <span className="sr-only">Unavailable: </span>
            {reason}
          </span>
        ) : null}
      </span>
      {entry.keys.length > 0 ? (
        <KbdGroup className="ml-auto">
          {entry.keys.map((key) => (
            <Kbd key={key}>{key}</Kbd>
          ))}
        </KbdGroup>
      ) : null}
    </CommandItem>
  )
}

/**
 * The command palette, ⌘K (ADR-0041, point 7): every action of the registry
 * the context offers, by menu, with its combination. A greyed one is listed
 * with its reason — the only place where the reason of a greyed entry of the
 * native macOS bar can be read — and choosing it does nothing.
 *
 * It lists what `sections` holds and runs nothing itself: `onRun` hands the
 * id to the registry, which checks it again, and a destructive action opens
 * its review there as from any other trigger.
 */
export function CommandPalette({
  open,
  onOpenChange,
  onOpenChangeComplete,
  sections,
  onRun,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Told once the palette has finished opening or closing. */
  onOpenChangeComplete?: (open: boolean) => void
  sections: Array<PaletteSection>
  onRun: (id: string) => void
}) {
  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      onOpenChangeComplete={onOpenChangeComplete}
      title="Command palette"
      description="Search an action of Oxyn by name or menu."
      className="sm:max-w-lg"
    >
      <Search sections={sections} onRun={onRun} />
    </CommandDialog>
  )
}

/** Inside the dialog: its search starts empty at every opening. */
function Search({
  sections,
  onRun,
}: {
  sections: Array<PaletteSection>
  onRun: (id: string) => void
}) {
  const [search, setSearch] = React.useState("")
  const shown = matching(sections, search)
  return (
    <Command shouldFilter={false} label="Search actions">
      <CommandInput
        value={search}
        onValueChange={setSearch}
        placeholder="Search actions…"
        aria-label="Search actions"
        {...TEXT_FIELD_ATTRIBUTES}
      />
      {/* An empty listbox is not a list: hidden — the field still names it
          in `aria-controls` — and a status says there is none. */}
      {shown.length === 0 ? (
        <p role="status" className="py-6 text-center text-sm">
          No action matches.
        </p>
      ) : null}
      <CommandList className="max-h-96" hidden={shown.length === 0}>
        {shown.map((section) => (
          <CommandGroup key={section.title} heading={section.title}>
            {section.entries.map((entry) => (
              <Entry key={entry.id} entry={entry} onRun={onRun} />
            ))}
          </CommandGroup>
        ))}
      </CommandList>
    </Command>
  )
}
