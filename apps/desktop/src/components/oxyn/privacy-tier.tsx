import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { CheckmarkCircle02Icon, Cancel01Icon } from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
  FieldLegend,
  FieldSet,
  FieldTitle,
} from "@/components/ui/field"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import type { ProviderReach } from "@/lib/ipc/ai"
import type { PrivacyTier } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

interface TierDescription {
  label: string
  summary: string
  /** What may leave the machine when an agent speaks (ADR-0006). */
  leaves: Array<string>
  /** What never does, said as plainly as what does. */
  stays: Array<string>
}

export const PRIVACY_TIERS: Record<PrivacyTier, TierDescription> = {
  local: {
    label: "Local",
    summary: "Nothing leaves this machine. Only a local model may answer.",
    leaves: [],
    stays: ["Schema and names", "Query plans", "Row values"],
  },
  metadata: {
    label: "Metadata",
    // ADR-0006: the default, and not « nothing leaves ».
    summary:
      "The default. Metadata is not « nothing leaves »: the structure of the database is sent to the provider.",
    leaves: [
      "DDL, object names and types",
      "Indexes and cardinalities",
      "Query plans",
    ],
    stays: ["Row values, including server errors that quote one"],
  },
  sampled: {
    label: "Sampled",
    summary:
      "Metadata, plus row samples you approve column by column before they are sent.",
    leaves: [
      "DDL, object names and types",
      "Indexes and cardinalities",
      "Query plans",
      "Row samples you approve, column by column",
    ],
    stays: ["Columns you did not approve"],
  },
}

const ORDER: ReadonlyArray<PrivacyTier> = ["local", "metadata", "sampled"]

/**
 * Where the provider that would answer resolved, as the top bar says it.
 *
 * `unresolved` and an external agent get no suffix: Oxyn cannot see where they
 * send data, and « Cloud » is a claim, not a default (docs/UX-SPEC.md,
 * « Repères permanents »).
 */
const REACH_SUFFIX: Record<ProviderReach, string | null> = {
  local: "Local",
  remote: "Cloud",
  unresolved: null,
}

/** `Metadata · Cloud`, `Metadata · Local`, or `Metadata` when nobody known answers. */
export function tierLabel(tier: PrivacyTier, reach: ProviderReach | null) {
  // Under `Local`, only a local provider answers: the suffix would repeat it.
  const suffix = reach && tier !== "local" ? REACH_SUFFIX[reach] : null
  const label = PRIVACY_TIERS[tier].label
  return suffix ? `${label} · ${suffix}` : label
}

/**
 * The tier of a connection, written out: never a colour or an icon alone.
 *
 * With `reach` — `null` when no provider is known — it takes the top bar's
 * form, beside `Ask AI`. Without it, in a list where nothing else says « AI »,
 * it keeps that prefix.
 */
export function PrivacyTierBadge({
  tier,
  reach,
  className,
}: {
  tier: PrivacyTier
  reach?: ProviderReach | null
  className?: string
}) {
  const text =
    reach === undefined
      ? `AI · ${PRIVACY_TIERS[tier].label}`
      : tierLabel(tier, reach)
  return (
    <Badge
      variant="outline"
      data-privacy-tier={tier}
      className={className}
      aria-label={`AI privacy: ${reach === undefined ? PRIVACY_TIERS[tier].label : text}`}
    >
      {text}
    </Badge>
  )
}

/** What leaves and what stays under a tier, as a checklist. */
export function PrivacyTierDisclosure({ tier }: { tier: PrivacyTier }) {
  const description = PRIVACY_TIERS[tier]
  return (
    <div
      aria-live="polite"
      data-slot="privacy-disclosure"
      className="flex flex-col gap-2 rounded-md border bg-card p-3 text-xs"
    >
      <p className="font-medium text-foreground">
        What leaves this machine under {description.label}
      </p>
      <ul className="flex flex-col gap-1" aria-label="Sent to the provider">
        {description.leaves.length === 0 ? (
          <li className="text-muted-foreground">Nothing.</li>
        ) : (
          description.leaves.map((item) => (
            <li key={item} className="flex items-center gap-1.5">
              <HugeiconsIcon
                icon={CheckmarkCircle02Icon}
                strokeWidth={2}
                className="size-3.5 text-warning"
                aria-hidden
              />
              {item}
            </li>
          ))
        )}
      </ul>
      <ul
        className="flex flex-col gap-1 text-muted-foreground"
        aria-label="Never sent"
      >
        {description.stays.map((item) => (
          <li key={item} className="flex items-center gap-1.5">
            <HugeiconsIcon
              icon={Cancel01Icon}
              strokeWidth={2}
              className="size-3.5"
              aria-hidden
            />
            {item}
          </li>
        ))}
      </ul>
    </div>
  )
}

/**
 * The privacy tier of a connection (ADR-0006, I-04).
 *
 * Attached to the connection, never to the session or the provider. The
 * disclosure follows the selection, so changing the tier shows what will
 * leave before anything is saved.
 */
export function PrivacyTierField({
  value,
  onChange,
  disabled = false,
}: {
  value: PrivacyTier
  onChange: (tier: PrivacyTier) => void
  disabled?: boolean
}) {
  // Scoped ids: two of these on one screen would point every label at the
  // first group's radios.
  const group = React.useId()
  return (
    <FieldSet>
      <FieldLegend variant="label">AI privacy</FieldLegend>
      <FieldDescription>
        Decides what an assistant may send about this connection.
      </FieldDescription>
      <RadioGroup
        value={value}
        onValueChange={(next: unknown) => {
          if (typeof next === "string" && next in PRIVACY_TIERS)
            onChange(next as PrivacyTier)
        }}
        disabled={disabled}
        aria-label="AI privacy"
        className="gap-2"
      >
        {ORDER.map((tier) => (
          <FieldLabel
            key={tier}
            htmlFor={`${group}-${tier}`}
            className={cn(value === tier && "border-primary")}
          >
            <Field orientation="horizontal">
              <RadioGroupItem value={tier} id={`${group}-${tier}`} />
              <FieldContent>
                <FieldTitle>{PRIVACY_TIERS[tier].label}</FieldTitle>
                <FieldDescription>
                  {PRIVACY_TIERS[tier].summary}
                </FieldDescription>
              </FieldContent>
            </Field>
          </FieldLabel>
        ))}
      </RadioGroup>
      <PrivacyTierDisclosure tier={value} />
    </FieldSet>
  )
}
