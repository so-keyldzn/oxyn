import { HugeiconsIcon } from "@hugeicons/react"
import { TextFontIcon } from "@hugeicons/core-free-icons"

import { DENSITIES } from "@/components/oxyn/appearance-settings"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { DensityChoice } from "@/lib/ipc/settings"

/**
 * The text size, from the result bar: the same two presets as the settings,
 * and the same preference (docs/UX-SPEC.md « Lisibilité et hauteur de
 * grille »). Changing it redraws the rows and reruns nothing.
 */
export function DensityMenu({
  value,
  onChange,
}: {
  value: DensityChoice
  onChange: (density: DensityChoice) => void
}) {
  const current = DENSITIES.find((density) => density.value === value)
  return (
    <DropdownMenu>
      {/* `xs`, as Columns beside it: the footer keeps its height. */}
      <DropdownMenuTrigger
        render={
          <Button
            variant="outline"
            size="xs"
            aria-label={`Text size: ${current?.label ?? value}`}
          />
        }
      >
        <HugeiconsIcon
          icon={TextFontIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        {current?.label ?? value}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        <DropdownMenuGroup>
          <DropdownMenuLabel>Text size</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={value}
            onValueChange={(next: unknown) => {
              const chosen = DENSITIES.find((density) => density.value === next)
              if (chosen) onChange(chosen.value)
            }}
          >
            {DENSITIES.map((density) => (
              <DropdownMenuRadioItem key={density.value} value={density.value}>
                {density.label}
                <span className="ml-auto text-xs text-muted-foreground">
                  {density.detail}
                </span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
