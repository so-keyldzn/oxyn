import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon } from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type { UpdateSnapshot, UpdateState } from "@/lib/ipc/updates"

import {
  disabledReasonText,
  downloadedBytes,
  errorNextStep,
  errorTitle,
  formatCheckedAt,
  liveStatus,
} from "./update-model"

/** The last save of the switch: applied for the session even when it failed. */
export type UpdatePreferenceSave =
  { status: "idle" } | { status: "failed"; message: string }

/** The version this launch updated to, said here for as long as it runs. */
export interface JustUpdated {
  from: string
  /** When this launch read the notice, RFC 3339. */
  at: string
}

export interface UpdateSettingsProps {
  snapshot: UpdateSnapshot
  save?: UpdatePreferenceSave
  justUpdated?: JustUpdated | null
  /** Rendered once, on the heading, when `Check for updates…` opened the section. */
  headingRef?: React.Ref<HTMLHeadingElement>
  onCheck: () => void
  onCancel: () => void
  onDownload: () => void
  onRestart: () => void
  onReleasePage: () => void
  onWhatsNew: () => void
  onAutomaticChange: (automatic: boolean) => void
  onRetrySave: () => void
}

/**
 * Settings ▸ Updates (ADR-0051, docs/UX-SPEC.md « Updates »): the version,
 * the switch, and where the update stands, with the actions that state
 * allows. It applies to the computer, not to the workspace, and says so.
 *
 * Where Oxyn does not update itself — a package manager, an administrator, a
 * development build — the view says why and offers no action. Release notes
 * are React text: never Markdown, never HTML.
 */
export function UpdateSettings({
  snapshot,
  save = { status: "idle" },
  justUpdated = null,
  headingRef,
  onCheck,
  onCancel,
  onDownload,
  onRestart,
  onReleasePage,
  onWhatsNew,
  onAutomaticChange,
  onRetrySave,
}: UpdateSettingsProps) {
  const { state, currentVersion, automatic } = snapshot
  const reasonId = React.useId()
  const switchId = React.useId()
  const locked =
    state.type === "disabled" && state.reason !== "user" ? state.reason : null
  const live = liveStatus(snapshot)

  return (
    <div className="flex flex-col gap-6 text-sm">
      <section className="flex flex-col gap-1">
        <h3
          ref={headingRef}
          tabIndex={-1}
          className="font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Updates
        </h3>
        <p>Oxyn {currentVersion} · Stable channel</p>
        <p className="text-muted-foreground">
          Applies to Oxyn on this computer, not only to this workspace.
        </p>
        {justUpdated ? (
          <p className="flex flex-wrap items-center gap-x-2">
            <span>
              Updated from {justUpdated.from} on{" "}
              {formatCheckedAt(justUpdated.at)}.
            </span>
            <Button
              variant="link"
              size="sm"
              className="h-auto px-0"
              onClick={onWhatsNew}
            >
              What&apos;s new
            </Button>
          </p>
        ) : null}
      </section>

      {locked === "packageManager" || locked === "dev" ? null : (
        <section className="flex flex-col gap-2">
          <Field orientation="horizontal" data-disabled={locked !== null}>
            <FieldContent>
              <FieldLabel htmlFor={switchId}>
                Download and install updates automatically
              </FieldLabel>
              <FieldDescription>
                Oxyn checks once a day and downloads in the background. An
                update installs when you quit; Oxyn never restarts on its own.
              </FieldDescription>
              {automatic && holdsDownload(state) ? (
                <FieldDescription>
                  Turning this off stops a download in progress and discards an
                  update that is not installed yet.
                </FieldDescription>
              ) : null}
            </FieldContent>
            <Switch
              id={switchId}
              checked={automatic}
              disabled={locked !== null}
              aria-describedby={locked ? reasonId : undefined}
              onCheckedChange={(next: boolean) => {
                if (next !== automatic) onAutomaticChange(next)
              }}
            />
          </Field>
          {save.status === "failed" ? (
            <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <HugeiconsIcon
                icon={Alert02Icon}
                strokeWidth={2}
                className="size-3.5 text-warning"
                aria-hidden
              />
              <span data-selectable className="text-foreground">
                Applied until Oxyn quits, but not saved: {save.message}
              </span>
              <Button variant="outline" size="xs" onClick={onRetrySave}>
                Save again
              </Button>
            </p>
          ) : null}
        </section>
      )}

      <section className="flex flex-col gap-3" aria-label="Update status">
        {/* Always mounted, so a change is announced; it holds state changes
            only — the bytes beside it are never read aloud. A failure is
            announced by its alert. */}
        <p
          className={
            state.type === "error"
              ? "sr-only"
              : "flex flex-wrap items-center gap-x-1"
          }
        >
          {state.type === "checking" ? (
            <Spinner className="mr-1 size-3.5" />
          ) : null}
          <span
            role="status"
            aria-live="polite"
            id={locked ? reasonId : undefined}
          >
            {state.type === "error" ? null : live}
          </span>
          {state.type === "downloading" ? (
            <span className="tabular-nums">
              — {downloadedBytes(state.received, state.total)}
            </span>
          ) : null}
        </p>
        {state.type === "error" ? (
          <ErrorView
            state={state}
            current={currentVersion}
            onCheck={onCheck}
            onDownload={onDownload}
            onReleasePage={onReleasePage}
          />
        ) : (
          <>
            {state.type === "downloading" ? (
              <DownloadProgress received={state.received} total={state.total} />
            ) : null}
            {state.type === "ready" && state.notes ? (
              <Notes notes={state.notes} version={state.version} />
            ) : null}
            <Actions
              state={state}
              automatic={automatic}
              reasonId={locked === "admin" ? reasonId : undefined}
              onCheck={onCheck}
              onCancel={onCancel}
              onDownload={onDownload}
              onRestart={onRestart}
              onReleasePage={onReleasePage}
            />
          </>
        )}
      </section>

      <p className="text-xs text-muted-foreground">
        A check sends only Oxyn&apos;s version, your operating system and your
        processor architecture to the update server (github.com). Nothing about
        your connections, queries or data.
      </p>
    </div>
  )
}

/** Whether turning automatic updates off would throw work away. */
function holdsDownload(state: UpdateState) {
  return state.type === "downloading" || state.type === "ready"
}

function DownloadProgress({
  received,
  total,
}: {
  received: number
  total: number | null
}) {
  const value =
    total !== null && total > 0
      ? Math.min(100, Math.round((received / total) * 100))
      : null
  const spoken = downloadedBytes(received, total)
  return (
    <Progress
      value={value}
      aria-label="Download progress"
      getAriaValueText={() => spoken}
    />
  )
}

function Notes({ notes, version }: { notes: string; version: string }) {
  return (
    <div className="flex flex-col gap-1">
      <h4 className="text-xs font-medium text-muted-foreground">
        What&apos;s new in Oxyn {version}
      </h4>
      <ScrollArea className="max-h-40 rounded-md border">
        {/* React text: a release note is shown as written, never as markup. */}
        <p
          data-selectable
          data-slot="update-notes"
          dir="auto"
          className="p-2 text-xs wrap-anywhere whitespace-pre-wrap"
        >
          {notes}
        </p>
      </ScrollArea>
    </div>
  )
}

function Actions({
  state,
  automatic,
  reasonId,
  onCheck,
  onCancel,
  onDownload,
  onRestart,
  onReleasePage,
}: {
  state: Exclude<UpdateState, { type: "error" }>
  automatic: boolean
  /** Set when the actions are refused by an administrator. */
  reasonId?: string
  onCheck: () => void
  onCancel: () => void
  onDownload: () => void
  onRestart: () => void
  onReleasePage: () => void
}) {
  const checkNow = (
    <Button variant="outline" size="sm" onClick={onCheck}>
      Check now
    </Button>
  )
  const releasePage = (
    <Button variant="outline" size="sm" onClick={onReleasePage}>
      Release page
    </Button>
  )
  let buttons: React.ReactNode = null
  switch (state.type) {
    case "idle":
    case "upToDate":
      buttons = checkNow
      break
    case "checking":
      // Reachable, so the keyboard does not lose its place, but inert: one
      // check at a time.
      buttons = (
        <>
          <Button
            variant="outline"
            size="sm"
            disabled
            focusableWhenDisabled
            className="data-disabled:opacity-50"
          >
            Check now
          </Button>
          <Button variant="outline" size="sm" onClick={onCancel}>
            Cancel
          </Button>
        </>
      )
      break
    case "available":
      buttons = automatic ? null : (
        <>
          <Button size="sm" onClick={onDownload}>
            Download and install
          </Button>
          {releasePage}
        </>
      )
      break
    case "downloading":
      buttons = (
        <Button variant="outline" size="sm" onClick={onCancel}>
          Stop download
        </Button>
      )
      break
    case "ready":
      buttons = (
        <>
          <Button size="sm" onClick={onRestart}>
            Restart now
          </Button>
          {releasePage}
        </>
      )
      break
    case "disabled":
      if (state.reason === "user") buttons = checkNow
      else if (state.reason === "admin")
        buttons = (
          <Button
            variant="outline"
            size="sm"
            disabled
            focusableWhenDisabled
            aria-describedby={reasonId}
            className="data-disabled:opacity-50"
            title={disabledReasonText(state.reason) ?? undefined}
          >
            Check now
          </Button>
        )
      break
  }
  return buttons ? <div className="flex flex-wrap gap-2">{buttons}</div> : null
}

function ErrorView({
  state,
  current,
  onCheck,
  onDownload,
  onReleasePage,
}: {
  state: Extract<UpdateState, { type: "error" }>
  current: string
  onCheck: () => void
  onDownload: () => void
  onReleasePage: () => void
}) {
  const nextStep = errorNextStep(state.kind, current)
  const downloads = state.kind === "signature" || state.kind === "install"
  return (
    <BackendErrorAlert
      title={errorTitle(state.kind, state.version)}
      error={{ message: state.message, retryable: state.retryable }}
      nextStep={nextStep}
      onRetry={downloads ? onDownload : onCheck}
      retryLabel={downloads ? "Download again" : "Check again"}
    >
      {/* The alert says the next step only for a final failure; these say it
          whatever `retryable`. */}
      {state.retryable && nextStep && state.kind !== "server" ? (
        <p className="text-xs text-muted-foreground">{nextStep}</p>
      ) : null}
      {downloads ? (
        <Button variant="outline" size="sm" onClick={onReleasePage}>
          Release page
        </Button>
      ) : null}
    </BackendErrorAlert>
  )
}
