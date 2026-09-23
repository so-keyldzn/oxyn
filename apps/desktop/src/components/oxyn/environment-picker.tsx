import * as React from "react"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { FieldDescription, FieldLegend, FieldSet } from "@/components/ui/field"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { ENVIRONMENTS } from "@/lib/ipc/types"
import type { Environment } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

const DETAILS: Record<Environment, string> = {
  production: "Every write asks for a review that names it",
  staging: "Looks like production, data matters less",
  development: "Shared, disposable data",
  local: "On this machine",
}

// Literal class names: Tailwind only generates what it can read here.
const CHECKED: Record<Environment, string> = {
  production:
    "has-data-checked:border-env-production has-data-checked:bg-env-production/10",
  staging:
    "has-data-checked:border-env-staging has-data-checked:bg-env-staging/10",
  development:
    "has-data-checked:border-env-development has-data-checked:bg-env-development/10",
  local: "has-data-checked:border-env-local has-data-checked:bg-env-local/10",
}

/**
 * The environment of a connection: one exclusive choice, so radios, each
 * card carrying the full ringed pill (docs/UX-SPEC.md « Repères permanents »).
 *
 * The chosen card is marked three ways — its radio, its border and its tint —
 * and never by colour alone. Production comes first: it is the default, and
 * the value an unmarked connection gets (I-02).
 */
export function EnvironmentPicker({
  value,
  onChange,
  disabled = false,
}: {
  value: Environment
  onChange: (environment: Environment) => void
  disabled?: boolean
}) {
  // Scoped ids: two pickers on one screen would point every card's label at
  // the first group's radios.
  const group = React.useId()
  return (
    <FieldSet data-slot="environment-picker">
      <FieldLegend variant="label">Environment</FieldLegend>
      <FieldDescription>
        Writes on production always ask for a review that names the connection.
      </FieldDescription>
      <RadioGroup
        value={value}
        onValueChange={(next: unknown) => {
          const chosen = ENVIRONMENTS.find(
            (environment) => environment === next
          )
          if (chosen) onChange(chosen)
        }}
        disabled={disabled}
        aria-label="Environment"
        className="grid grid-cols-2 gap-2 sm:grid-cols-4"
      >
        {ENVIRONMENTS.map((environment) => (
          <label
            key={environment}
            htmlFor={`${group}-${environment}`}
            className={cn(
              "flex cursor-pointer flex-col gap-1.5 rounded-lg border bg-card p-2.5 transition-colors outline-none hover:bg-accent has-focus-visible:ring-3 has-focus-visible:ring-ring/50 has-disabled:cursor-not-allowed has-disabled:opacity-50 motion-reduce:transition-none",
              CHECKED[environment]
            )}
          >
            <span className="flex items-center justify-between gap-2">
              <EnvironmentBadge environment={environment} />
              <RadioGroupItem
                id={`${group}-${environment}`}
                value={environment}
                aria-describedby={`${group}-${environment}-detail`}
              />
            </span>
            <span
              id={`${group}-${environment}-detail`}
              className="text-xs text-muted-foreground"
            >
              {DETAILS[environment]}
            </span>
          </label>
        ))}
      </RadioGroup>
    </FieldSet>
  )
}
