import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Cancel01Icon,
  CursorRectangleSelection01Icon,
  GridIcon,
  TableIcon,
} from "@hugeicons/core-free-icons"

import { PRIVACY_TIERS } from "@/components/oxyn/privacy-tier"
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentGroup,
  AttachmentMedia,
  AttachmentTitle,
} from "@/components/ui/attachment"
import type { PrivacyTier } from "@/lib/ipc/types"

export type ContextPinKind = "object" | "result" | "selection"

export interface ContextPin {
  key: string
  kind: ContextPinKind
  /**
   * What it is, by name — an object name, a query title.
   *
   * Never a cell, never a bound value, never a connection identifier: none of
   * those have a field here, so none can be rendered by accident (I-03).
   */
  label: string
  /** Its shape in words — « 12 columns », « 200 rows ». Never its content. */
  shape?: string | null
}

const KINDS: Record<ContextPinKind, { label: string; icon: typeof TableIcon }> =
  {
    object: { label: "Object", icon: TableIcon },
    result: { label: "Result", icon: GridIcon },
    selection: { label: "Selection", icon: CursorRectangleSelection01Icon },
  }

/** Stable order, so the list does not reshuffle as pins come and go. */
const ORDER: ReadonlyArray<ContextPinKind> = ["object", "result", "selection"]

/**
 * What leaves this machine, per kind of pin, under the connection's tier.
 *
 * This table is the user-facing reading of ADR-0006; it is not a second rule.
 * What actually leaves is decided in Rust by the single passage point (I-04).
 * The two must say the same thing, and this file is the one to correct if
 * they ever differ.
 */
const LEAVES: Record<ContextPinKind, Record<PrivacyTier, string>> = {
  object: {
    local: "nothing — only a local model reads its structure",
    metadata: "its name, columns, types and indexes",
    sampled: "its name, columns, types and indexes",
  },
  result: {
    local: "nothing — only a local model reads it",
    metadata: "column names and the number of rows; the values stay",
    sampled:
      "column names, row counts, and the values you approve, column by column",
  },
  selection: {
    local: "nothing — only a local model reads it",
    metadata: "column names and the number of rows; the values stay",
    sampled:
      "column names, row counts, and the values you approve, column by column",
  },
}

function Pin({
  pin,
  onRemove,
}: {
  pin: ContextPin
  onRemove: (key: string) => void
}) {
  const kind = KINDS[pin.kind]
  return (
    <Attachment size="sm" data-kind={pin.kind}>
      <AttachmentMedia>
        <HugeiconsIcon icon={kind.icon} strokeWidth={2} aria-hidden />
      </AttachmentMedia>
      <AttachmentContent>
        {/* One line with an end ellipsis: the full name stays readable in the
            attached sources and in the outgoing-context review
            (docs/UX-SPEC.md, « Lisibilité et hauteur de grille »). */}
        <AttachmentTitle title={pin.label}>
          <bdi>{pin.label}</bdi>
        </AttachmentTitle>
        <AttachmentDescription>
          {pin.shape ? `${kind.label} · ${pin.shape}` : kind.label}
        </AttachmentDescription>
      </AttachmentContent>
      <AttachmentActions>
        <AttachmentAction
          aria-label={`Remove ${pin.label} from this question's context`}
          onClick={() => onRemove(pin.key)}
        >
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
        </AttachmentAction>
      </AttachmentActions>
    </Attachment>
  )
}

/**
 * What the user attached to this question, and what it will cost to send it.
 *
 * This is the one place where the consequence of ADR-0006 is read **before**
 * speaking rather than after: each kind of pin says, in words, what leaves
 * this machine under the connection's tier. The tier arrives by prop — it
 * belongs to the connection, never to the session, the provider or the
 * application (I-04).
 *
 * The component is declarative: it pins nothing, sends nothing, and assembles
 * nothing. Removing a pin is local, and the assembly stays in Rust, behind
 * the single passage point.
 */
export function AssistantContextPins({
  pins,
  tier,
  onRemove,
}: {
  pins: ReadonlyArray<ContextPin>
  /** The connection's privacy tier (ADR-0006). */
  tier: PrivacyTier
  onRemove: (key: string) => void
}) {
  const titleId = React.useId()

  // Nothing pinned is no strip: an empty frame above the composer would take
  // the room of a control that is not there.
  if (pins.length === 0) return null

  const kinds = ORDER.filter((kind) => pins.some((pin) => pin.kind === kind))

  return (
    <section
      data-slot="assistant-context-pins"
      aria-labelledby={titleId}
      className="flex min-w-0 flex-col gap-1.5"
    >
      <p id={titleId} className="text-xs text-muted-foreground">
        Attached to this question
      </p>
      {/* Its own horizontal scroller: a long pin name never widens the panel
          around it. */}
      <AttachmentGroup aria-label="Attached to this question">
        {pins.map((pin) => (
          <Pin key={pin.key} pin={pin} onRemove={onRemove} />
        ))}
      </AttachmentGroup>
      <div className="flex min-w-0 flex-col gap-0.5 rounded-md border border-dashed px-2 py-1.5 text-xs text-muted-foreground">
        <p>
          Sending this question lets these leave this machine, under{" "}
          <strong className="font-medium text-foreground">
            {PRIVACY_TIERS[tier].label}
          </strong>
          :
        </p>
        <ul className="flex min-w-0 flex-col gap-0.5">
          {kinds.map((kind) => (
            <li key={kind} className="min-w-0 wrap-break-word">
              <span className="text-foreground">{KINDS[kind].label}s</span> —{" "}
              {LEAVES[kind][tier]}
            </li>
          ))}
        </ul>
        <p>Nothing is sent from here.</p>
      </div>
    </section>
  )
}
