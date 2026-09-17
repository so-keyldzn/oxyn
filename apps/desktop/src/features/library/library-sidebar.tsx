import { HugeiconsIcon } from "@hugeicons/react"
import { BookOpen01Icon, Logout03Icon } from "@hugeicons/core-free-icons"

import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from "@/components/ui/sidebar"
import { LibrarySidebarSection } from "@/features/library/library-sidebar-section"
import type {
  DocumentEntry,
  DocumentView,
  HistoryDetail,
  HistoryRow,
} from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * The left sidebar when the library is the chosen view. Shaped like the
 * catalog's, so switching views does not move the work area.
 */
export function LibrarySidebar({
  open,
  onLeave,
  onOpenHistory,
  onOpenResult,
  onOpenCopy,
  onResume,
}: {
  open: OpenConnection
  onLeave: () => void
  onOpenHistory: (entry: HistoryDetail) => void
  onOpenResult: (row: HistoryRow) => void
  onOpenCopy: (document: DocumentView, entry: DocumentEntry) => void
  onResume: (document: DocumentView) => void
}) {
  return (
    <Sidebar variant="inset" collapsible="icon">
      <SidebarHeader data-tauri-drag-region className="pt-8">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" tooltip="Library">
              <div className="flex aspect-square size-8 items-center justify-center rounded-lg bg-primary text-primary-foreground">
                <HugeiconsIcon
                  icon={BookOpen01Icon}
                  strokeWidth={2}
                  className="size-4"
                />
              </div>
              <div className="grid flex-1 text-left leading-tight">
                <span className="truncate font-medium">Library</span>
                <span className="truncate text-xs text-muted-foreground">
                  Local queries and history
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup className="min-h-0 flex-1 p-0 group-data-[collapsible=icon]:hidden">
          <SidebarGroupLabel className="px-4">Library</SidebarGroupLabel>
          <LibrarySidebarSection
            open={open}
            onOpenHistory={onOpenHistory}
            onOpenResult={onOpenResult}
            onOpenCopy={onOpenCopy}
            onResume={onResume}
          />
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
      <SidebarRail />
    </Sidebar>
  )
}
