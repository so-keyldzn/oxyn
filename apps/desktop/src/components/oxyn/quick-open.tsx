import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { addressKey, addressLabel } from "@/components/oxyn/catalog-tree"
import {
  Command,
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"
import { TEXT_FIELD_ATTRIBUTES } from "./text-field"

/** Where the search of the loaded catalog stands. */
export type QuickOpenResults =
  | { status: "idle" }
  | { status: "searching" }
  | { status: "done"; hits: Array<CatalogSearchHit> }
  | { status: "error"; error: BackendFailure }

/** The scope, said whatever the results: the catalog is never read for it. */
const SCOPE = "Searches the objects already loaded in the catalog."

function Status({ children }: { children: React.ReactNode }) {
  return (
    <p role="status" className="px-3 py-6 text-center text-muted-foreground">
      {children}
    </p>
  )
}

/**
 * Opening an object by name, ⌘P (ADR-0041, point 7). It searches what the
 * catalog has already loaded — the bounded search of the catalog sidebar, no
 * query to the server — and says so. Choosing a hit opens it as a click in
 * the tree does.
 */
export function QuickOpen({
  open,
  onOpenChange,
  query,
  onQueryChange,
  results,
  onOpenObject,
  onRetry,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  query: string
  onQueryChange: (query: string) => void
  results: QuickOpenResults
  onOpenObject: (hit: CatalogSearchHit) => void
  onRetry?: () => void
}) {
  const hits = results.status === "done" ? results.hits : []
  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Open object"
      description={SCOPE}
      className="sm:max-w-lg"
    >
      {/* The backend searches and ranks: cmdk only moves the selection. */}
      <Command shouldFilter={false} label="Object name">
        <CommandInput
          value={query}
          onValueChange={onQueryChange}
          placeholder="Open a table, a view…"
          aria-label="Object name"
          {...TEXT_FIELD_ATTRIBUTES}
        />
        {/* Outside the list: a listbox holds options, not messages. */}
        {results.status === "error" ? (
          <div className="p-2">
            <BackendErrorAlert
              title="Cannot search the catalog"
              error={results.error}
              onRetry={onRetry}
            />
          </div>
        ) : null}
        {results.status === "idle" ? (
          <Status>Type part of a name.</Status>
        ) : null}
        {results.status === "searching" ? (
          <Status>Searching loaded objects…</Status>
        ) : null}
        {results.status === "done" && results.hits.length === 0 ? (
          <Status>No loaded object matches.</Status>
        ) : null}
        {/* Hidden when empty: the field names it in `aria-controls`, and an
            empty listbox is not a list. */}
        <CommandList className="max-h-96" hidden={hits.length === 0}>
          {hits.length > 0 ? (
            <CommandGroup heading="Loaded objects">
              {hits.map((hit) => {
                const parent = addressLabel({ ...hit.address, relation: null })
                return (
                  <CommandItem
                    key={addressKey(hit.address)}
                    value={addressKey(hit.address)}
                    onSelect={() => onOpenObject(hit)}
                  >
                    <span className="flex min-w-0 flex-col">
                      <span dir="auto" className="truncate">
                        {hit.name}
                      </span>
                      <span className="truncate text-xs text-muted-foreground">
                        {parent ? `${hit.kind} · ${parent}` : hit.kind}
                      </span>
                    </span>
                  </CommandItem>
                )
              })}
            </CommandGroup>
          ) : null}
        </CommandList>
      </Command>
      <p className="border-t px-3 py-2 text-xs text-muted-foreground">
        {SCOPE}
      </p>
    </CommandDialog>
  )
}
