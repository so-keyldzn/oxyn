import type * as React from "react"
import { cva } from "class-variance-authority"
import { cn } from "cn"

import { Badge } from "@/components/ui/badge"
import type { Environment } from "@/lib/ipc/types"

const LABELS: Record<Environment, string> = {
  production: "PRODUCTION",
  staging: "Staging",
  development: "Development",
  local: "Local",
}

// Literal class names, one per environment: Tailwind only generates what it
// can read here.
const environmentBadge = cva(
  "text-[length:var(--reading-caption)] tracking-wide",
  {
    variants: {
      environment: {
        production: "border-env-production text-env-production",
        staging: "border-env-staging text-env-staging",
        development: "border-env-development text-env-development",
        local: "border-env-local text-env-local",
      },
    },
  }
)

/**
 * The environment marking, as a ringed pill.
 *
 * The label is always written out: colour alone never carries the marking
 * (docs/UX-SPEC.md, « Repères permanents »). Built on `Badge`, so the pill,
 * its focus ring and its slot come from one place; `...props` lets a caller
 * add its own `aria-*` or `id`.
 */
export function EnvironmentBadge({
  environment,
  className,
  ...props
}: React.ComponentProps<typeof Badge> & { environment: Environment }) {
  return (
    <Badge
      variant="outline"
      data-slot="environment-badge"
      data-environment={environment}
      className={cn(environmentBadge({ environment }), className)}
      {...props}
    >
      {LABELS[environment]}
    </Badge>
  )
}

export function environmentLabel(environment: Environment) {
  return LABELS[environment]
}
