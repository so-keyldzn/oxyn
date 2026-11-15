import type { ReactNode } from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ChartRelationshipIcon,
  DatabaseSync01Icon,
} from "@hugeicons/core-free-icons"
import {
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
} from "@/components/ui/sidebar"
import { ContentHint } from "./content-hint"

/** Compact tools belong to the displayed source; the preference trigger remains host-owned. */
export function CatalogSourceTools({
  onSchemaMap,
  onNativeTransfers,
  preferences,
}: {
  onSchemaMap?: () => void
  onNativeTransfers?: () => void
  preferences?: ReactNode
}) {
  if (!onSchemaMap && !onNativeTransfers && !preferences) return null
  return (
    <SidebarFooter>
      <SidebarGroup className="p-0">
        <SidebarGroupLabel>Source tools</SidebarGroupLabel>
        <SidebarMenu
          aria-label="Source tools"
          className="flex-row group-data-[collapsible=icon]:flex-col"
        >
          {onSchemaMap && (
            <SidebarMenuItem className="min-w-0 flex-1">
              <ContentHint content="Schema map">
                <SidebarMenuButton
                  aria-label="View schema map"
                  onClick={onSchemaMap}
                >
                  <HugeiconsIcon
                    icon={ChartRelationshipIcon}
                    data-icon="inline-start"
                    strokeWidth={2}
                  />
                  <span className="sr-only">Schema map</span>
                </SidebarMenuButton>
              </ContentHint>
            </SidebarMenuItem>
          )}
          {onNativeTransfers && (
            <SidebarMenuItem className="min-w-0 flex-1">
              <ContentHint content="Database transfers">
                <SidebarMenuButton
                  aria-label="Database transfers"
                  onClick={onNativeTransfers}
                >
                  <HugeiconsIcon
                    icon={DatabaseSync01Icon}
                    data-icon="inline-start"
                    strokeWidth={2}
                  />
                  <span className="sr-only">Database transfers</span>
                </SidebarMenuButton>
              </ContentHint>
            </SidebarMenuItem>
          )}
          {preferences}
        </SidebarMenu>
      </SidebarGroup>
    </SidebarFooter>
  )
}
