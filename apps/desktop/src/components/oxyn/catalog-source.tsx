import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowLeft01Icon } from "@hugeicons/core-free-icons"
import { Button } from "@/components/ui/button"
import { ContentHint } from "./content-hint"

/** Only an explicitly selected console adds context to the explorer header. */
export function CatalogSource({
  name,
  notice = null,
  onReturn,
}: {
  name: string | null
  notice?: string | null
  onReturn: () => void
}) {
  if (!name)
    return notice ? (
      <p
        role="status"
        aria-label="Explorer browsing source"
        className="px-3 py-1 text-xs text-muted-foreground group-data-[collapsible=icon]:sr-only"
      >
        {notice}
      </p>
    ) : null
  return (
    <div className="flex min-w-0 items-center gap-1 px-3 py-1 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0">
      <p
        role="status"
        aria-label="Explorer browsing source"
        className="min-w-0 flex-1 truncate text-xs group-data-[collapsible=icon]:sr-only"
      >
        Browsing <bdi>{name}</bdi>
      </p>
      <ContentHint content={`Browsing ${name} · Return to connection browsing`}>
        <Button
          variant="ghost"
          size="xs"
          onClick={onReturn}
          aria-label={`Return to connection browsing; currently browsing ${name}`}
        >
          <HugeiconsIcon
            icon={ArrowLeft01Icon}
            data-icon="inline-start"
            strokeWidth={2}
          />
          <span className="group-data-[collapsible=icon]:hidden">Return</span>
        </Button>
      </ContentHint>
    </div>
  )
}
