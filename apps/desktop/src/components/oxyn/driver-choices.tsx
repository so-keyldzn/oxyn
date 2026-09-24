import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { GridViewIcon } from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import {
  ButtonItemContent,
  ButtonItemDescription,
  ButtonItemMedia,
  ButtonItemTitle,
} from "@/components/oxyn/button-item"
import { DriverLogo } from "@/components/oxyn/driver-logo"
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import { Item } from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "cn"
import type { DriverChoice } from "@/lib/ipc/types"

/**
 * Up to this many types, every one is a tile. Beyond, the tiles become a
 * short list and the rest is searched: the start screen stays a launcher,
 * not a catalogue that pushes the saved connections out of sight.
 */
const ALL_AS_TILES = 6

/**
 * Whether the driver opens a file rather than reaching a server. Read from
 * the fields it declares, never from its name: a new driver is drawn right
 * without this component knowing it (ADR-0003).
 */
function opensAFile(driver: DriverChoice) {
  return driver.fields.some(
    (field) => field.required && field.kind.type === "path"
  )
}

function reach(driver: DriverChoice) {
  if (opensAFile(driver)) return "local file"
  return driver.defaultPort ? `port ${driver.defaultPort}` : "server"
}

/**
 * The tiles shown when not all fit: the types the saved connections already
 * use, then the order this build declares. No ranking is invented here.
 */
function shortlist(drivers: Array<DriverChoice>, used: Array<string>) {
  const rank = (driver: DriverChoice) => {
    const index = used.indexOf(driver.id)
    return index === -1 ? used.length : index
  }
  return drivers
    .map((driver, order) => ({ driver, order }))
    .sort((a, b) => rank(a.driver) - rank(b.driver) || a.order - b.order)
    .slice(0, ALL_AS_TILES - 1)
    .map(({ driver }) => driver)
}

/** Families in the order their first driver is declared. */
function byFamily(drivers: Array<DriverChoice>) {
  const families = new Map<string, Array<DriverChoice>>()
  for (const driver of drivers) {
    const family = families.get(driver.family)
    if (family) family.push(driver)
    else families.set(driver.family, [driver])
  }
  return Array.from(families)
}

/**
 * A substring, not cmdk's fuzzy default: « neo » scattered across
 * « SNowflakE » would rank it and Enter would open the wrong form.
 */
function containsSearch(value: string, search: string, keywords?: string[]) {
  const needle = search.trim().toLocaleLowerCase()
  if (!needle) return 1
  return [value, ...(keywords ?? [])].some((text) =>
    text.toLocaleLowerCase().includes(needle)
  )
    ? 1
    : 0
}

/**
 * Every type, searchable by name or family and grouped by family. The
 * driver's id is only the item's value: it is never rendered.
 */
function DriverSearch({
  drivers,
  onChoose,
  className,
}: {
  drivers: Array<DriverChoice>
  onChoose: (driver: DriverChoice) => void
  className?: string
}) {
  return (
    <Command className={className} filter={containsSearch}>
      <CommandInput
        placeholder="Search database types…"
        aria-label="Search database types"
      />
      <CommandList className="max-h-80">
        <CommandEmpty>No database type matches.</CommandEmpty>
        {byFamily(drivers).map(([family, members]) => (
          <CommandGroup
            key={family}
            heading={family}
            className="**:[[cmdk-group-heading]]:capitalize"
          >
            {members.map((driver) => (
              <CommandItem
                key={driver.id}
                value={driver.id}
                keywords={[driver.displayName, driver.family]}
                onSelect={() => onChoose(driver)}
              >
                <DriverLogo driver={driver.id} className="size-4" />
                <span className="truncate">{driver.displayName}</span>
                <CommandShortcut className="tracking-normal tabular-nums">
                  {reach(driver)}
                </CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>
        ))}
      </CommandList>
    </Command>
  )
}

/**
 * The database types this build registers. Choosing one only shows its form:
 * nothing is created or opened by the click.
 *
 * Up to six, all are tiles. Beyond, five tiles — the types already in use
 * first — and « All database types », a searchable palette. On a first launch
 * (`prominent`) with that many, the search is the page itself: there is
 * nothing else to do yet.
 */
export function DriverChoices({
  drivers,
  used = [],
  error,
  disabled = false,
  prominent = false,
  onChoose,
  onRetry,
}: {
  drivers: Array<DriverChoice> | undefined
  /** Driver ids of the saved connections, most relevant first. */
  used?: Array<string>
  error?: BackendFailure | null
  disabled?: boolean
  prominent?: boolean
  onChoose: (driver: DriverChoice) => void
  onRetry?: () => void
}) {
  const [browsing, setBrowsing] = React.useState(false)

  if (error) {
    return (
      <BackendErrorAlert
        title="Cannot list database types"
        error={error}
        onRetry={onRetry}
      />
    )
  }

  // auto-fit, not a column count: two drivers fill the row instead of
  // leaving a third of it empty, and a narrow window stacks them.
  const grid = cn(
    "grid grid-cols-[repeat(auto-fit,minmax(12rem,1fr))] gap-2",
    prominent && "gap-3"
  )

  if (drivers === undefined) {
    return (
      <div aria-busy="true">
        <span className="sr-only">Loading database types</span>
        <div className={grid}>
          <Skeleton className={prominent ? "h-24" : "h-16"} />
          <Skeleton className={prominent ? "h-24" : "h-16"} />
        </div>
      </div>
    )
  }

  if (drivers.length === 0) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyTitle>No database type</EmptyTitle>
          <EmptyDescription>
            This build registers no driver, so no connection can be created.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const many = drivers.length > ALL_AS_TILES

  if (many && prominent) {
    return (
      <DriverSearch
        drivers={drivers}
        onChoose={onChoose}
        className="h-auto rounded-lg! border bg-card"
      />
    )
  }

  const tiles = many ? shortlist(drivers, used) : drivers

  return (
    <>
      <div role="list" className={grid}>
        {tiles.map((driver) => (
          <div role="listitem" key={driver.id} className="flex">
            <Item
              variant="outline"
              render={
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => onChoose(driver)}
                />
              }
              className={cn(
                "min-w-0 text-left hover:bg-muted/50 disabled:pointer-events-none disabled:opacity-50",
                prominent && "flex-col flex-nowrap items-start p-4"
              )}
            >
              <ButtonItemMedia>
                <DriverLogo
                  driver={driver.id}
                  className={prominent ? "size-6" : "size-5"}
                />
              </ButtonItemMedia>
              <ButtonItemContent className="w-full min-w-0 gap-0.5">
                <ButtonItemTitle className="w-full min-w-0">
                  <span className="truncate" title={driver.displayName}>
                    {driver.displayName}
                  </span>
                </ButtonItemTitle>
                <ButtonItemDescription className="truncate text-xs tabular-nums">
                  {driver.family} · {reach(driver)}
                </ButtonItemDescription>
              </ButtonItemContent>
            </Item>
          </div>
        ))}
        {many ? (
          <div role="listitem" className="flex">
            <Item
              variant="outline"
              render={
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => setBrowsing(true)}
                  aria-haspopup="dialog"
                />
              }
              className="min-w-0 text-left hover:bg-muted/50 disabled:pointer-events-none disabled:opacity-50"
            >
              <ButtonItemMedia>
                <HugeiconsIcon icon={GridViewIcon} strokeWidth={2} />
              </ButtonItemMedia>
              <ButtonItemContent className="min-w-0 gap-0.5">
                <ButtonItemTitle className="w-full min-w-0">
                  <span className="truncate">All database types</span>
                </ButtonItemTitle>
                <ButtonItemDescription className="truncate text-xs tabular-nums">
                  {drivers.length} types · search
                </ButtonItemDescription>
              </ButtonItemContent>
            </Item>
          </div>
        ) : null}
      </div>
      {many ? (
        <CommandDialog
          open={browsing}
          onOpenChange={setBrowsing}
          title="All database types"
          description="Search a database type by name or family."
        >
          <DriverSearch
            drivers={drivers}
            onChoose={(driver) => {
              setBrowsing(false)
              onChoose(driver)
            }}
          />
        </CommandDialog>
      ) : null}
    </>
  )
}
