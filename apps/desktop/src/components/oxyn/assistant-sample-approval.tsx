import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  CloudIcon,
  ComputerIcon,
} from "@hugeicons/core-free-icons"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { PRIVACY_TIERS } from "@/components/oxyn/privacy-tier"
import { Alert, AlertDescription } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Field, FieldLabel, FieldLegend, FieldSet } from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import type { ProviderReach, SampleRequest as SampleOffer } from "@/lib/ipc/ai"
import type { Environment, PrivacyTier, RelationField } from "@/lib/ipc/types"

/**
 * What the screen reads of the backend's offer — its schema's type, less the
 * address, which only the approval sent back needs.
 *
 * - `id` is the pending approval this screen answers, one per send and never
 *   rendered. The screen's state is keyed on it, so two sends on the same
 *   source are two decisions, never one remembered.
 * - `source` is written to be read, never to be run.
 * - `rows` is Oxyn's own bound, never an estimate.
 * - `fields` are `RelationField`s exactly as the catalog reports them: type,
 *   nullability, default and comment — all metadata. **They carry no value**,
 *   which is why they are the right type here (I-03).
 * - `reach` says whether the destination is off this machine.
 * - `requestedBy` names the agent when an agent asked for the sample
 *   (`request_sample`, ADR-0034), `null` when the user pinned the relation.
 */
export type SampleRequest = Omit<SampleOffer, "address">

function whereItGoes(reach: ProviderReach) {
  // `unresolved` counts as remote for every decision, and is said as what it
  // is rather than given the benefit of the doubt (ADR-0026).
  return reach === "local"
    ? { label: "on this machine", icon: ComputerIcon, leaves: false }
    : reach === "remote"
      ? { label: "off this machine", icon: CloudIcon, leaves: true }
      : { label: "to an unresolved address", icon: CloudIcon, leaves: true }
}

/** One column, and everything that is known about it that is not a value. */
function ColumnRow({
  field,
  checked,
  disabled,
  onToggle,
}: {
  field: RelationField
  checked: boolean
  disabled: boolean
  onToggle: (checked: boolean) => void
}) {
  const id = React.useId()
  return (
    <li className="min-w-0">
      <Field
        orientation="horizontal"
        className="min-w-0 items-start rounded-md px-1 py-1 hover:bg-muted/50"
      >
        <Checkbox
          id={id}
          checked={checked}
          disabled={disabled}
          onCheckedChange={onToggle}
          // Every box starts unticked here, so it is nothing but its edge. The
          // theme's `--input` carries the 3:1 WCAG 1.4.11 asks of that edge;
          // the stories below measure it on this dialog in both themes.
          className="mt-0.5"
        />
        <FieldLabel
          htmlFor={id}
          className="min-w-0 flex-1 cursor-pointer flex-col items-stretch gap-0.5 font-normal"
        >
          <span className="flex min-w-0 flex-wrap items-center gap-1.5">
            <bdi className="min-w-0 font-mono text-xs wrap-break-word">
              {field.name}
            </bdi>
            <span className="text-xs text-muted-foreground">
              {field.logicalType}
            </span>
            {field.primaryKey ? (
              <Badge variant="outline" className="shrink-0">
                Key
              </Badge>
            ) : null}
            {field.nullable ? (
              <span className="text-xs text-muted-foreground">nullable</span>
            ) : null}
          </span>
          {field.comment ? (
            <span className="min-w-0 text-xs wrap-break-word text-muted-foreground">
              {field.comment}
            </span>
          ) : null}
        </FieldLabel>
      </Field>
    </li>
  )
}

/**
 * The approval of a row sample, column by column (ADR-0006, `Sampled`).
 *
 * This is the screen `Sampled` was always described with and never had: the
 * tier says row values leave « approuvé explicitement, colonne par colonne »,
 * and without a place to approve, the tier could not be held as written.
 *
 * Four decisions, and each one is a refusal of something easier:
 *
 * * **nothing is ticked when it opens.** A pre-ticked box is a consent nobody
 *   gave, and « explicitly approved » would become « did not object » ;
 * * **there is no « select all ».** Sending sixty columns of real data to a
 *   cloud model is exactly the act that should cost sixty gestures. The
 *   friction is the feature ;
 * * **no cell value is shown, and there is no prop that could carry one.** One
 *   approves columns, not a preview. A preview would put the very values in
 *   question into a dialog that gets screenshotted and pasted into tickets,
 *   and it would decide nothing the type and the comment do not decide ;
 * * **the screen refuses to be the hole.** Under any tier but `Sampled` it
 *   draws a refusal instead of checkboxes: a dialog that offered to send
 *   values under `Metadata` would be a defect that ships silently.
 *
 * And one rule about time, which is a rule about consent: **an approval covers
 * one send, and is never remembered** — not for the source, not for the
 * connection. A confirmation remembered is a confirmation clicked once for
 * every later time, including the time the table gained a `notes` column full
 * of customer addresses (I-02: a confirmation ends up being clicked). So:
 *
 * * there is no « remember this choice », and no prop that could ask for one ;
 * * the ticks live in `ApprovalBody`, **keyed on the request's id** and
 *   unmounted when the screen closes. Nothing survives a close, and a new send
 *   on the same source starts unticked — by construction, not by an effect
 *   that resets after a frame where the old ticks would still show ;
 * * `onDecide` returns the columns of **this** send only.
 *
 * Like `ApprovalDialog`, it cannot be confirmed by reflex: `Cancel` takes the
 * initial focus and comes first at every width, and Enter alone never approves.
 *
 * **The same screen when an agent asks** (ADR-0034). A request the user did
 * not start is the one most likely to be clicked through, so it says so first
 * — who asks, and that Cancel declines — and changes nothing else: the same
 * unticked boxes, the same destination named, the same refusal outside
 * `sampled`. The agent may have named the columns it wants; those are the
 * ones offered, and none is ticked for the user.
 */
export function AssistantSampleApproval({
  request,
  connectionName,
  environment,
  tier,
  deciding = false,
  onDecide,
}: {
  request: SampleRequest | null
  connectionName: string
  environment: Environment
  /** The connection's tier. Anything but `sampled` is refused here. */
  tier: PrivacyTier
  deciding?: boolean
  /**
   * The approved column names, in catalog order — or `null` when nothing was
   * approved.
   *
   * `null` rather than an empty array: « approved no column » and « did not
   * approve » are two different answers, and only one of them is a decision.
   */
  onDecide: (columns: ReadonlyArray<string> | null) => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)

  return (
    <AlertDialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open && !deciding) onDecide(null)
      }}
    >
      <AlertDialogContent
        // The base width is keyed on `data-size`, so a plain `max-w-xl` loses
        // to it; and a grid track sized by its content lets a long label widen
        // the list and the footer past the frame. `grid-cols-1` bounds it.
        className="grid-cols-1 data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-xl"
        initialFocus={cancelRef}
        // Enter on the dialog body must never reach the action that sends.
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !(event.target instanceof HTMLButtonElement)
          ) {
            event.preventDefault()
          }
        }}
      >
        {request ? (
          <ApprovalBody
            // The key is the consent's scope: a new send remounts, and a
            // closed screen holds no body at all.
            key={request.id}
            request={request}
            connectionName={connectionName}
            environment={environment}
            tier={tier}
            deciding={deciding}
            cancelRef={cancelRef}
            onDecide={onDecide}
          />
        ) : null}
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Everything a decision holds. Mounted for one send, and one only. */
function ApprovalBody({
  request,
  connectionName,
  environment,
  tier,
  deciding,
  cancelRef,
  onDecide,
}: {
  request: SampleRequest
  connectionName: string
  environment: Environment
  tier: PrivacyTier
  deciding: boolean
  cancelRef: React.RefObject<HTMLButtonElement | null>
  onDecide: (columns: ReadonlyArray<string> | null) => void
}) {
  const [ticked, setTicked] = React.useState<ReadonlyArray<string>>([])
  // `initialFocus` acts when the dialog opens, and only then. A request that
  // follows another without the dialog closing — an agent's ask waiting behind
  // the user's pin — remounts this body under a dialog already open: the
  // focus was on a control of the body just removed, and Cancel would not get
  // it back. Taken here, on every mount, it is Cancel's for every request.
  React.useEffect(() => {
    cancelRef.current?.focus()
  }, [cancelRef])
  const legendId = React.useId()
  const allowed = tier === "sampled"
  const fields = request.fields
  const where = whereItGoes(request.reach)
  const chosen = fields.filter((field) => ticked.includes(field.name))

  return (
    <>
      <AlertDialogHeader>
        <div className="flex flex-wrap items-center gap-2">
          <AlertDialogTitle>
            {request.requestedBy === null
              ? "Approve a row sample"
              : "An agent asks for a row sample"}
          </AlertDialogTitle>
          <EnvironmentBadge environment={environment} />
        </div>
        <AlertDialogDescription className="wrap-anywhere">
          {request.requestedBy !== null ? (
            <>
              <strong className="font-medium text-foreground">
                <bdi>{request.requestedBy}</bdi>
              </strong>{" "}
              asked for it, not you, and waits for your answer. Cancel
              declines.{" "}
            </>
          ) : null}
          {allowed ? (
            <>
              {request.rows.toLocaleString()} rows of{" "}
              <strong className="font-medium text-foreground">
                <bdi>{request.source}</bdi>
              </strong>{" "}
              on{" "}
              <strong className="font-medium text-foreground">
                <bdi>{connectionName}</bdi>
              </strong>
              . Only the columns you tick are sent.
            </>
          ) : (
            <>
              This connection is on{" "}
              <strong className="font-medium text-foreground">
                {PRIVACY_TIERS[tier].label}
              </strong>
              , and row values never leave under that tier. Nothing here can be
              approved.
            </>
          )}
        </AlertDialogDescription>
      </AlertDialogHeader>

      {!allowed ? (
        <p className="text-xs text-muted-foreground">
          {PRIVACY_TIERS[tier].summary} Change the tier on the connection itself
          if a sample is really what you want — not from here.
        </p>
      ) : fields.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          This source reports no column. There is nothing to approve, and
          nothing will be sent.
        </p>
      ) : (
        <>
          {/* A standing note, not news: `role="note"` keeps it from being
              announced before the dialog's title, as `Alert`'s own
              `role="alert"` would. */}
          <Alert role="note">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertDescription className="text-xs">
              A column you leave unticked is <strong>not sent</strong>. It is
              not truncated and not masked — it does not leave. Oxyn cannot tell
              you which column holds personal data: the name, the type and the
              comment are all it knows.
            </AlertDescription>
          </Alert>
          {/* `min-w-0`: a fieldset is as wide as its content by default, and
              a long column name would widen the dialog with it. */}
          <FieldSet className="min-w-0">
            <FieldLegend id={legendId} className="sr-only">
              Columns that may be sent
            </FieldLegend>
            {/* Vertical only, and its own region: sixty columns scroll here
                without the dialog ever scrolling sideways. */}
            <ul
              aria-labelledby={legendId}
              tabIndex={0}
              className="flex max-h-64 min-w-0 flex-col gap-0.5 overflow-x-hidden overflow-y-auto rounded-md border p-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {fields.map((field) => (
                <ColumnRow
                  key={field.name}
                  field={field}
                  checked={ticked.includes(field.name)}
                  disabled={deciding}
                  onToggle={(checked) =>
                    setTicked((current) =>
                      checked
                        ? [...current, field.name]
                        : current.filter((name) => name !== field.name)
                    )
                  }
                />
              ))}
            </ul>
          </FieldSet>
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {chosen.length === 0
              ? "No column ticked. Nothing would be sent."
              : `${chosen.length} of ${fields.length} columns ticked.`}
          </p>
        </>
      )}

      {/* `flex-col` rather than the footer's reversed stack: stacked on a
            narrow window, Cancel stays above the action. */}
      <AlertDialogFooter className="flex-col sm:flex-row">
        <AlertDialogCancel ref={cancelRef} disabled={deciding}>
          Cancel
        </AlertDialogCancel>
        {allowed && fields.length > 0 ? (
          <Button
            variant="default"
            disabled={deciding || chosen.length === 0}
            onClick={() => onDecide(chosen.map((field) => field.name))}
            // The label says what leaves and where, because this is the last
            // screen before it does: it wraps, and is never cut.
            title={`Send ${request.rows} rows of ${chosen.length} columns to ${request.destination}, ${where.label}`}
            className="h-auto min-h-8 min-w-0 shrink py-1.5 text-left whitespace-normal"
          >
            {deciding ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon
                icon={where.icon}
                strokeWidth={2}
                data-icon="inline-start"
              />
            )}
            <span className="min-w-0 wrap-anywhere">
              Send {chosen.length} of {fields.length} columns to{" "}
              <bdi>{request.destination}</bdi> · {where.label}
            </span>
          </Button>
        ) : null}
      </AlertDialogFooter>
    </>
  )
}
