import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, ArrowDown01Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"

/** The module sqlite-vec registers, which a connection can opt into. */
const SQLITE_VEC_MODULE = "vec0"

/**
 * Why a virtual table's rows cannot be read: the extension providing it is not
 * loaded. Shown in place of the raw failure, which stays one click away — the
 * engine's own words are never replaced, only explained.
 *
 * The module and table names come from the database: rendered as text.
 */
export function MissingModuleNotice({
  module,
  shadows,
  message,
  onOpenShadow,
}: {
  module: string
  /** The tables storing its data, as the catalog lists them. */
  shadows: Array<string>
  /** The driver's message, as it came. */
  message: string
  /** Selects a shadow table, as a click in the explorer would. */
  onOpenShadow?: (name: string) => void
}) {
  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto p-4">
      <Alert className="gap-1.5 px-3 py-3">
        <HugeiconsIcon
          icon={Alert02Icon}
          strokeWidth={2}
          className="text-warning"
        />
        <AlertTitle>This table needs an SQLite extension</AlertTitle>
        <AlertDescription className="flex min-w-0 flex-col gap-2">
          <p>
            This table is provided by the SQLite extension{" "}
            <code className="font-mono text-foreground">{module}</code>, which
            Oxyn does not load.{" "}
            {shadows.length > 0 ? (
              <>
                Its data is stored in{" "}
                {shadows.map((name, index) => (
                  <React.Fragment key={name}>
                    {index > 0
                      ? index === shadows.length - 1
                        ? " and "
                        : ", "
                      : null}
                    {onOpenShadow ? (
                      <Button
                        variant="link"
                        className="h-auto p-0 font-mono text-[length:inherit]"
                        onClick={() => onOpenShadow(name)}
                      >
                        {name}
                      </Button>
                    ) : (
                      <code className="font-mono text-foreground">{name}</code>
                    )}
                  </React.Fragment>
                ))}
                .
              </>
            ) : (
              "No table storing its data is listed beside it."
            )}
          </p>
          {module === SQLITE_VEC_MODULE ? (
            // The one extension Oxyn ships, off unless the connection opts in:
            // loading code a database file asks for is a trust decision.
            <p>
              You can enable sqlite-vec for this connection in its settings, if
              you trust this file.
            </p>
          ) : null}
          <p className="text-xs">
            Running it again as is will fail the same way.
          </p>
          <Collapsible className="flex min-w-0 flex-col">
            <CollapsibleTrigger className="group/raw inline-flex w-fit items-center gap-1 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
              Driver message
              <HugeiconsIcon
                icon={ArrowDown01Icon}
                strokeWidth={2}
                className="size-3.5 shrink-0 transition-transform group-data-[panel-open]/raw:rotate-180 motion-reduce:transition-none"
                aria-hidden
              />
            </CollapsibleTrigger>
            <CollapsibleContent className="overflow-hidden">
              {/* The server's words, code included — never a paraphrase. */}
              <pre
                data-selectable
                tabIndex={0}
                aria-label="Server message"
                className="mt-1 max-h-64 overflow-auto rounded-md border bg-muted/40 p-2.5 font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {message}
              </pre>
            </CollapsibleContent>
          </Collapsible>
        </AlertDescription>
      </Alert>
    </div>
  )
}
