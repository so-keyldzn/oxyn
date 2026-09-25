import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, ArrowRight01Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import type { ThirdPartyNotice } from "@/lib/third-party-licenses"

export type ThirdPartyLicensesState =
  | { status: "loading" }
  /** Built without `cargo-about`: a development build, never a release. */
  | { status: "missing" }
  | { status: "error"; message: string }
  | { status: "ready"; notices: Array<ThirdPartyNotice> }

/** How many package names a closed notice shows before « +N ». */
const NAMES_SHOWN = 4

function matches(notice: ThirdPartyNotice, filter: string) {
  if (!filter) return true
  const needle = filter.toLowerCase()
  return (
    notice.license.toLowerCase().includes(needle) ||
    notice.packages.some((p) => p.name.toLowerCase().includes(needle))
  )
}

function NoticeRow({ notice }: { notice: ThirdPartyNotice }) {
  const names = notice.packages.map((p) => p.name)
  const shown = names.slice(0, NAMES_SHOWN).join(", ")
  const more = names.length - NAMES_SHOWN
  return (
    <Collapsible className="border-b last:border-b-0">
      <CollapsibleTrigger className="group/notice flex w-full min-w-0 items-center gap-2 py-2 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          className="size-3.5 shrink-0 text-muted-foreground transition-transform group-data-[panel-open]/notice:rotate-90"
          aria-hidden
        />
        <span className="shrink-0 font-medium">{notice.license}</span>
        <span className="min-w-0 truncate text-muted-foreground">
          {shown}
          {more > 0 ? ` +${more}` : ""}
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-2 pb-3 pl-5.5">
        <ul className="text-xs text-muted-foreground">
          {notice.packages.map((p) => (
            <li key={`${p.ecosystem}:${p.name}@${p.version}`}>
              {p.name} {p.version} <span className="sr-only">from </span>
              <span>({p.ecosystem === "cargo" ? "crate" : "npm"})</span>
            </li>
          ))}
        </ul>
        <pre className="max-h-64 overflow-auto rounded-md bg-muted p-3 font-mono text-xs whitespace-pre-wrap">
          {notice.text}
        </pre>
      </CollapsibleContent>
    </Collapsible>
  )
}

function ThirdPartyList({ notices }: { notices: Array<ThirdPartyNotice> }) {
  const [filter, setFilter] = React.useState("")
  const packages = notices.reduce((n, notice) => n + notice.packages.length, 0)
  const shown = notices.filter((notice) => matches(notice, filter.trim()))

  if (notices.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        This build lists no third-party package.
      </p>
    )
  }
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-muted-foreground">
        {packages} packages, under {notices.length} license texts. Each is
        distributed under its own license, reproduced below.
      </p>
      <Input
        type="search"
        aria-label="Filter by package or license"
        placeholder="Filter by package or license"
        value={filter}
        onChange={(event) => setFilter(event.target.value)}
      />
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No package or license matches « {filter.trim()} ».
        </p>
      ) : (
        <div>
          {shown.map((notice) => (
            <NoticeRow
              key={`${notice.license}:${notice.packages[0]?.ecosystem}:${notice.packages[0]?.name}`}
              notice={notice}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * Who holds the copyright, under which licenses Oxyn is distributed, and the
 * licenses of the packages it ships (ADR-0044). The GPL asks an interactive
 * program to show its license and the absence of warranty: this is where.
 */
export function AboutSettings({
  licenses,
}: {
  licenses: ThirdPartyLicensesState
}) {
  return (
    <div className="flex flex-col gap-6">
      <section className="flex flex-col gap-2 text-sm">
        <h3 className="font-medium">Oxyn</h3>
        <p>Copyright 2026 Nicolas Boromée</p>
        <p className="text-muted-foreground">
          Oxyn is free software, distributed under the GNU General Public
          License, version 3 or any later version. The driver contract
          (oxyn-core, oxyn-catalog, oxyn-data and oxyn-driver) is distributed
          under the Apache License, version 2.0.
        </p>
        <p className="text-muted-foreground">
          Oxyn comes with absolutely no warranty, to the extent permitted by
          law.
        </p>
      </section>

      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-medium">Third-party licenses</h3>
        {licenses.status === "loading" ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner /> Reading the licenses…
          </p>
        ) : licenses.status === "missing" ? (
          <p className="text-sm text-muted-foreground">
            This build was made without its third-party notices:
            script/licences-tierces did not run. A release build always includes
            them.
          </p>
        ) : licenses.status === "error" ? (
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} aria-hidden />
            <AlertTitle>The third-party notices could not be read</AlertTitle>
            <AlertDescription>{licenses.message}</AlertDescription>
          </Alert>
        ) : (
          <ThirdPartyList notices={licenses.notices} />
        )}
      </section>
    </div>
  )
}
