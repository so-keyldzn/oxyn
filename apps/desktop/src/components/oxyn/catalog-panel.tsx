import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowReloadHorizontalIcon,
  DatabaseIcon,
  FolderLibraryIcon,
  LockIcon,
  Logout03Icon,
} from "@hugeicons/core-free-icons"

import { CatalogTree } from "@/components/oxyn/catalog-tree"
import type { OpenTarget, PinToQuestion } from "@/components/oxyn/catalog-tree"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"
import type { CatalogAddress, CatalogNode } from "@/lib/ipc/types"

/** Why the catalog is not what the tree shows: a failure, or a refusal. */
export type CatalogProblem =
  | { kind: "error"; message: string; retryable: boolean }
  | { kind: "denied"; reason: string }

/**
 * The catalog side of the workspace sidebar, by props: header, the Catalog
 * group and the footer. The container (`features/workspace/catalog-sidebar`)
 * wraps it in `Sidebar` and reads the backend.
 *
 * The header names the connection without being a control: the top bar and
 * the status bar already carry its environment. A refresh in flight can be
 * cancelled; a failed one says whether trying again can help; a refused one
 * says who refused. Folded to the rail, the group stays reachable as one
 * icon that unfolds the sidebar.
 */
export function CatalogPanel({
  connectionName,
  driver,
  supported,
  nodes,
  loading,
  selected,
  expanded,
  onExpandedChange,
  onExpand,
  onSelect,
  search,
  refreshing,
  cancelling = false,
  onRefresh,
  onCancelRefresh,
  problem,
  notice = null,
  onDismissProblem,
  onOpen,
  onCopyName,
  onRefreshLevel,
  pin,
  onLeave,
}: {
  connectionName: string
  driver: string
  /** Whether this session has a catalog to browse at all (ADR-0003). */
  supported: boolean
  /** `undefined` until the first tree arrives. */
  nodes: Array<CatalogNode> | undefined
  /** Keys of the levels being read. */
  loading: ReadonlySet<string>
  selected: string | null
  expanded: ReadonlySet<string>
  onExpandedChange: (expanded: Set<string>) => void
  onExpand: (address: CatalogAddress) => void
  onSelect: (node: CatalogNode) => void
  search: {
    hits: Array<CatalogSearchHit> | null
    searching: boolean
    onQuery: (query: string) => void
  }
  /** A refresh of the whole catalog runs. */
  refreshing: boolean
  /** Cancel was pressed and the answer has not arrived. */
  cancelling?: boolean
  onRefresh: () => void
  onCancelRefresh: () => void
  problem: CatalogProblem | null
  /** A quiet line, for what is neither an error nor news: a cancelled refresh. */
  notice?: string | null
  onDismissProblem?: () => void
  onOpen?: (node: CatalogNode, target: OpenTarget) => void
  onCopyName?: (node: CatalogNode) => void
  onRefreshLevel?: (node: CatalogNode) => void
  /** Offered only where a sample could follow; see `canPin`. */
  pin?: PinToQuestion
  onLeave: () => void
}) {
  const { setOpen } = useSidebar()

  return (
    <>
      <SidebarHeader data-tauri-drag-region className="pt-8">
        <div
          data-slot="catalog-connection"
          className="flex min-w-0 items-center gap-2 rounded-md p-2 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-0"
        >
          <div className="flex aspect-square size-8 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
            <HugeiconsIcon
              icon={DatabaseIcon}
              strokeWidth={2}
              className="size-4"
            />
          </div>
          <div className="grid min-w-0 flex-1 leading-tight group-data-[collapsible=icon]:hidden">
            <span
              dir="auto"
              title={connectionName}
              className="truncate text-sm font-medium"
            >
              {connectionName}
            </span>
            <span className="truncate text-xs text-muted-foreground">
              {driver}
            </span>
          </div>
        </div>
      </SidebarHeader>

      <SidebarContent>
        {/* Folded to the rail, the catalog is one icon away, not gone. */}
        <SidebarGroup className="hidden group-data-[collapsible=icon]:flex">
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                tooltip="Catalog"
                onClick={() => setOpen(true)}
              >
                <HugeiconsIcon icon={FolderLibraryIcon} strokeWidth={2} />
                <span>Catalog</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>

        <SidebarGroup className="min-h-0 flex-1 group-data-[collapsible=icon]:hidden">
          <SidebarGroupLabel>Catalog</SidebarGroupLabel>
          {supported ? (
            // One meaning per place: this action refreshes, and is inert while
            // a refresh runs. Cancelling is the button in the status line
            // below, so a missed click never restarts what was to be stopped
            // (docs/UX-SPEC.md, « Portée de Run »).
            <Tooltip>
              <TooltipTrigger
                render={
                  <SidebarGroupAction
                    aria-label="Refresh catalog"
                    disabled={refreshing}
                    onClick={onRefresh}
                  />
                }
              >
                <HugeiconsIcon
                  icon={ArrowReloadHorizontalIcon}
                  strokeWidth={2}
                />
              </TooltipTrigger>
              <TooltipContent>Refresh catalog</TooltipContent>
            </Tooltip>
          ) : null}
          <SidebarGroupContent className="flex min-h-0 flex-1 flex-col gap-2">
            {!supported ? (
              <p className="px-2 text-xs text-muted-foreground">
                This database has no catalog to browse.
              </p>
            ) : (
              <>
                {refreshing ? (
                  <div
                    role="status"
                    className="flex items-center gap-2 px-2 text-xs text-muted-foreground"
                  >
                    <Spinner aria-hidden className="size-3" />
                    <span className="min-w-0 flex-1 truncate">
                      {cancelling
                        ? "Cancelling the refresh…"
                        : nodes === undefined
                          ? "Reading the catalog…"
                          : "Refreshing the catalog…"}
                    </span>
                    <Button
                      size="xs"
                      variant="ghost"
                      disabled={cancelling}
                      onClick={onCancelRefresh}
                    >
                      Cancel
                    </Button>
                  </div>
                ) : notice ? (
                  <p
                    role="status"
                    className="px-2 text-xs text-muted-foreground"
                  >
                    {notice}
                  </p>
                ) : null}
                {problem ? (
                  <CatalogProblemAlert
                    problem={problem}
                    refreshing={refreshing}
                    onRefresh={onRefresh}
                    onDismiss={onDismissProblem}
                  />
                ) : null}
                {nodes === undefined ? (
                  problem ? null : (
                    <div
                      aria-hidden
                      className="flex flex-col gap-1.5 px-2 pt-1"
                    >
                      {SKELETON_WIDTHS.map((width, index) => (
                        <Skeleton
                          key={index}
                          className="h-5"
                          style={{ width }}
                        />
                      ))}
                    </div>
                  )
                ) : (
                  <CatalogTree
                    nodes={nodes}
                    loading={loading}
                    selected={selected}
                    expanded={expanded}
                    onExpandedChange={onExpandedChange}
                    onExpand={onExpand}
                    onSelect={onSelect}
                    search={search}
                    onOpen={onOpen}
                    onCopyName={onCopyName}
                    onRefresh={onRefreshLevel}
                    pin={pin}
                  />
                )}
              </>
            )}
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton tooltip="Switch connection" onClick={onLeave}>
              <HugeiconsIcon icon={Logout03Icon} strokeWidth={2} />
              <span>Switch connection</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </>
  )
}

const SKELETON_WIDTHS = ["60%", "78%", "52%", "70%", "45%", "66%"]

function CatalogProblemAlert({
  problem,
  refreshing,
  onRefresh,
  onDismiss,
}: {
  problem: CatalogProblem
  refreshing: boolean
  onRefresh: () => void
  onDismiss?: () => void
}) {
  if (problem.kind === "denied") {
    return (
      <Alert className="py-2">
        <HugeiconsIcon icon={LockIcon} strokeWidth={2} />
        <AlertTitle>Catalog read refused</AlertTitle>
        <AlertDescription className="flex flex-col gap-1.5">
          <p data-selectable dir="auto" className="text-xs">
            {problem.reason}
          </p>
          <p className="text-xs">
            The connection policy decides this read; refreshing as is gives the
            same answer.
          </p>
          {onDismiss ? (
            <div>
              <Button size="xs" variant="ghost" onClick={onDismiss}>
                Dismiss
              </Button>
            </div>
          ) : null}
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <Alert variant="destructive" className="py-2">
      <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
      <AlertTitle>The catalog could not be read</AlertTitle>
      <AlertDescription className="flex flex-col gap-1.5">
        {/* The server's words, code included — never a paraphrase. */}
        <pre
          data-selectable
          // Scrollable, so reachable by keyboard.
          tabIndex={0}
          aria-label="Server message"
          className="max-h-32 overflow-auto font-mono text-xs whitespace-pre-wrap text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {problem.message}
        </pre>
        <p className="text-xs">
          {problem.retryable
            ? "This error is transient: refreshing may succeed."
            : "Refreshing as is will fail the same way."}
        </p>
        <div className="flex gap-1">
          {problem.retryable ? (
            <Button
              size="xs"
              variant="outline"
              disabled={refreshing}
              onClick={onRefresh}
            >
              Refresh catalog
            </Button>
          ) : null}
          {onDismiss ? (
            <Button size="xs" variant="ghost" onClick={onDismiss}>
              Dismiss
            </Button>
          ) : null}
        </div>
      </AlertDescription>
    </Alert>
  )
}
