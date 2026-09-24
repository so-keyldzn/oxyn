import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, SourceCodeIcon } from "@hugeicons/core-free-icons"

import { renderMermaid } from "@/components/oxyn/mermaid-render"
import type { MermaidOutcome } from "@/components/oxyn/mermaid-render"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"

function subscribeToTheme(onChange: () => void) {
  const observer = new MutationObserver(onChange)
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  })
  return () => observer.disconnect()
}

/** The theme is a class on `<html>`; an image cannot follow it by CSS. */
function useDarkDocument() {
  return React.useSyncExternalStore(
    subscribeToTheme,
    () => document.documentElement.classList.contains("dark"),
    () => true
  )
}

/**
 * A closed `mermaid` block of an answer, drawn as an image.
 *
 * mermaid is loaded the first time such a block appears, and never before.
 * What it draws is shown through `<img>`: nothing of the model's source
 * reaches the DOM as markup (see `mermaid-render.ts`).
 */
export function AssistantMermaid({ source }: { source: string }) {
  const dark = useDarkDocument()
  const [outcome, setOutcome] = React.useState<{
    key: string
    value: MermaidOutcome
  } | null>(null)
  const [showSource, setShowSource] = React.useState(false)
  const sourceId = React.useId()
  const key = JSON.stringify([source, dark])

  React.useEffect(() => {
    let live = true
    void renderMermaid(source, dark).then((value) => {
      if (live) setOutcome({ key, value })
    })
    return () => {
      live = false
    }
  }, [source, dark, key])

  const current = outcome?.key === key ? outcome.value : null
  // An invalid diagram shows its source by itself: it is what to correct.
  const sourceShown = showSource || current?.status === "invalid"

  return (
    <figure
      data-slot="assistant-mermaid"
      className="flex min-w-0 flex-col overflow-hidden rounded-lg border bg-card"
    >
      <figcaption className="flex h-8 items-center justify-between gap-2 border-b px-2.5 text-xs text-muted-foreground">
        <span className="font-mono">mermaid</span>
        {current?.status === "drawn" ? (
          <Button
            size="xs"
            variant="ghost"
            aria-expanded={showSource}
            aria-controls={sourceId}
            onClick={() => setShowSource((shown) => !shown)}
          >
            <HugeiconsIcon
              icon={SourceCodeIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            {showSource ? "Hide source" : "Show source"}
          </Button>
        ) : null}
      </figcaption>
      {current === null ? (
        <p
          aria-busy
          className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground"
        >
          <Spinner aria-hidden />
          Drawing the diagram…
        </p>
      ) : current.status === "drawn" ? (
        <div
          tabIndex={0}
          role="region"
          aria-label="Diagram"
          className="overflow-x-auto p-3 outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <img
            src={current.src}
            // What the diagram says is in its source, one click away.
            alt="Diagram drawn from the mermaid source of this answer"
            width={current.width}
            height={current.height}
            className="mx-auto h-auto max-w-full"
            style={{ width: current.width }}
          />
        </div>
      ) : (
        <div className="p-2">
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>This diagram could not be drawn.</AlertTitle>
            <AlertDescription>
              <p
                dir="auto"
                className="font-mono break-words whitespace-pre-wrap"
              >
                {current.message}
              </p>
            </AlertDescription>
          </Alert>
        </div>
      )}
      {current?.stripped ? (
        <p className="border-t px-3 py-1.5 text-xs text-muted-foreground">
          Its configuration lines (%%{"{init}"}%% or front matter) were ignored:
          a diagram does not choose how Oxyn draws it.
        </p>
      ) : null}
      {sourceShown ? (
        <pre
          id={sourceId}
          data-selectable
          tabIndex={0}
          aria-label="Diagram source"
          className="overflow-x-auto border-t p-3 font-mono text-xs leading-5 whitespace-pre outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <code>{source}</code>
        </pre>
      ) : null}
    </figure>
  )
}
