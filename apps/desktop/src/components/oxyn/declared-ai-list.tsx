import { HugeiconsIcon } from "@hugeicons/react"
import {
  CommandLineIcon,
  Delete02Icon,
  Key01Icon,
  PencilEdit02Icon,
  RefreshIcon,
  SecurityCheckIcon,
  SparklesIcon,
} from "@hugeicons/core-free-icons"

import {
  counted,
  kindLabel,
  reachSummary,
} from "@/components/oxyn/provider-settings-model"
import type { ModelsState } from "@/components/oxyn/provider-settings-model"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import type { DeclaredProvider, ExternalAgent } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

/** How many model ids are written out before « … ». */
const LISTED_MODELS = 6

function modelsSummary(listed: ModelsState) {
  if (listed.status === "loading") return "Asking the endpoint…"
  if (listed.status === "error") return listed.message
  const { models } = listed
  if (models.length === 0) return "The endpoint answered and lists no model."
  const ids = models
    .slice(0, LISTED_MODELS)
    .map((model) => model.id)
    .join(", ")
  const more = models.length > LISTED_MODELS ? "…" : ""
  return `The endpoint answered · ${counted(models.length, "model", "models")}: ${ids}${more}`
}

export function DeclaredProviderItem({
  provider,
  listed,
  busy,
  onListModels,
  onEdit,
  onRemove,
}: {
  provider: DeclaredProvider
  listed: ModelsState | undefined
  busy: boolean
  onListModels: () => void
  onEdit: () => void
  onRemove: () => void
}) {
  return (
    <Item variant="outline" className="flex-wrap">
      <ItemMedia variant="icon">
        <HugeiconsIcon icon={SparklesIcon} strokeWidth={2} />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle>
          {provider.label}
          <span className="text-xs font-normal text-muted-foreground">
            {kindLabel(provider.kind)}
          </span>
        </ItemTitle>
        <ItemDescription className="font-mono text-xs break-all">
          {provider.endpoint}
          {provider.endpointRedacted ? "…" : ""} · {provider.model}
        </ItemDescription>
        <div className="flex flex-wrap gap-1.5 pt-1">
          <Badge variant="outline">
            <HugeiconsIcon
              icon={Key01Icon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            {provider.keyConfigured ? "Key configured" : "No key"}
          </Badge>
          <Badge
            variant="outline"
            className={cn(
              provider.reach !== "local" &&
                "border-env-staging text-env-staging"
            )}
          >
            {reachSummary(provider.reach, provider.measuredAtMs)}
          </Badge>
        </div>
        {listed ? (
          <p
            role="status"
            className={cn(
              "pt-1 text-xs",
              listed.status === "error"
                ? "font-mono text-destructive"
                : "text-muted-foreground"
            )}
          >
            {modelsSummary(listed)}
          </p>
        ) : null}
      </ItemContent>
      <ItemActions>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy || listed?.status === "loading"}
          onClick={onListModels}
        >
          <HugeiconsIcon
            icon={RefreshIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          List models
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={`Edit ${provider.label}`}
          disabled={busy}
          onClick={onEdit}
        >
          <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={`Remove ${provider.label}`}
          disabled={busy}
          onClick={onRemove}
        >
          <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
        </Button>
      </ItemActions>
    </Item>
  )
}

export function DeclaredAgentItem({
  agent,
  busy,
  onReplace,
  onRemove,
}: {
  agent: ExternalAgent
  busy: boolean
  onReplace: () => void
  onRemove: () => void
}) {
  return (
    <Item variant="outline" className="flex-wrap">
      <ItemMedia variant="icon">
        <HugeiconsIcon icon={CommandLineIcon} strokeWidth={2} />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="flex-wrap">
          {agent.label}
          <span className="text-xs font-normal text-muted-foreground">
            External agent
          </span>
          {agent.confined ? (
            <Badge variant="outline" className="font-normal">
              <HugeiconsIcon
                icon={SecurityCheckIcon}
                strokeWidth={2}
                data-icon="inline-start"
              />
              Restricted by Oxyn
            </Badge>
          ) : null}
        </ItemTitle>
        {/* One line: a launcher under a version manager is a long path, and
            the whole of it stays selectable to copy. */}
        <ItemDescription
          data-selectable
          className="truncate font-mono text-xs"
          title={agent.command}
        >
          {agent.command}
          {agent.argCount > 0
            ? ` · ${counted(agent.argCount, "argument", "arguments")}`
            : ""}
        </ItemDescription>
        {agent.confined ? (
          <ItemDescription className="text-xs">
            Only Oxyn&apos;s tools, no shell or files, in its strictest mode
            {/* Codex's MDM and cloud layers are out of Oxyn's reach (ADR-0033). */}
            {agent.preset === "codex"
              ? " — except what your organization's managed configuration turns on."
              : "."}
          </ItemDescription>
        ) : null}
        {!agent.confined ? (
          <ItemDescription className="text-xs text-warning">
            {agent.preset === null
              ? "Not a known agent: "
              : "Not confined as declared — another version, other arguments, or a tool your organization's configuration turns on: "}
            Oxyn cannot stop it from acting on this machine by itself.
          </ItemDescription>
        ) : null}
      </ItemContent>
      <ItemActions>
        <Button size="sm" variant="ghost" disabled={busy} onClick={onReplace}>
          <HugeiconsIcon
            icon={PencilEdit02Icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Replace…<span className="sr-only"> {agent.label}</span>
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={`Remove ${agent.label}`}
          disabled={busy}
          onClick={onRemove}
        >
          <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
        </Button>
      </ItemActions>
    </Item>
  )
}
