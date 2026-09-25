import * as React from "react"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import type { SheetSection } from "@/lib/actions/listings"

/**
 * The keyboard shortcut sheet, ⌘/ outside the SQL editor (ADR-0041,
 * point 7). Generated from the manifest: every combination Oxyn binds, by the
 * zone where it applies, and no list written anywhere else.
 */
export function ShortcutSheet({
  open,
  onOpenChange,
  sections,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  sections: Array<SheetSection>
}) {
  const headingId = React.useId()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>
            A shortcut of a zone applies where the focus is, and takes over the
            one everywhere else.
          </DialogDescription>
        </DialogHeader>
        <div
          // Scrolled by the keyboard as well as by the pointer.
          tabIndex={0}
          role="region"
          aria-label="Shortcuts by zone"
          className="grid max-h-[60vh] gap-x-8 gap-y-4 overflow-y-auto rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring sm:grid-cols-2"
        >
          {sections.map((section, index) => (
            <section
              key={section.title}
              aria-labelledby={`${headingId}-${index}`}
            >
              <h3
                id={`${headingId}-${index}`}
                className="mb-1 text-xs font-medium text-muted-foreground"
              >
                {section.title}
              </h3>
              <dl className="flex flex-col">
                {section.rows.map((row) => (
                  <div
                    key={row.id}
                    className="flex items-center justify-between gap-4 border-b border-border/50 py-1 last:border-0"
                  >
                    <dt className="min-w-0 truncate">{row.label}</dt>
                    <dd className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                      {row.keys.map((keys, alternative) => (
                        <React.Fragment key={keys.join("+")}>
                          {alternative > 0 ? <span>or</span> : null}
                          <KbdGroup>
                            {keys.map((key) => (
                              <Kbd key={key}>{key}</Kbd>
                            ))}
                          </KbdGroup>
                        </React.Fragment>
                      ))}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  )
}
