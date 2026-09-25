import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowLeft01Icon,
  CheckmarkCircle02Icon,
  FileClockIcon,
  LockIcon,
} from "@hugeicons/core-free-icons"

import markUrl from "@/assets/oxyn-mark.png"
import { ApprovalDialog } from "@/components/oxyn/approval-dialog"
import type { PendingApproval } from "@/components/oxyn/approval-dialog"
import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { ConnectionForm } from "@/components/oxyn/connection-form"
import type {
  ConnectionPrefill,
  FormValues,
} from "@/components/oxyn/connection-form"
import { DriverChoices } from "@/components/oxyn/driver-choices"
import { DiscardChangesDialog } from "@/components/oxyn/discard-changes-dialog"
import { SavedConnections } from "@/components/oxyn/saved-connections"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Kbd } from "@/components/ui/kbd"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { ConnectionSummary } from "@/lib/ipc/settings"
import type {
  ConnectionDraft,
  ConnectionTest,
  DriverChoice,
  Environment,
  FormField,
} from "@/lib/ipc/types"
import { cn } from "@/lib/utils"
import { useActionSource } from "@/lib/actions/context"
import type { ConnectionMenuActions } from "@/lib/actions/targets"

export type PendingConnectionApproval = PendingApproval & {
  name: string
  environment: Environment
}

export interface ConnectionScreenViewProps {
  connections: Array<ConnectionSummary> | undefined
  connectionsError?: BackendFailure | null
  onRetryConnections?: () => void

  drivers: Array<DriverChoice> | undefined
  driversError?: BackendFailure | null
  onRetryDrivers?: () => void

  /** The database type whose form is shown, if one was chosen. */
  driver: DriverChoice | null
  /**
   * The form copies a saved connection (`Duplicate`): the name of the source
   * and the form's starting values, never a secret.
   */
  duplicate?: { of: string; prefill: ConnectionPrefill } | null
  onChooseDriver: (driver: DriverChoice) => void
  onLeaveDriver: () => void

  /** The saved connection being opened, by id. */
  opening: string | null
  openError?: BackendFailure | null
  onOpen: (connection: ConnectionSummary) => void
  onRetryOpen?: () => void
  /** The connection whose workspace stayed open, by id. */
  openConnectionId?: string | null
  /** The context menu of a saved connection, beyond `Connect` (= `onOpen`). */
  connectionMenu?: (connection: ConnectionSummary) => ConnectionMenuActions

  /** A new connection being created and opened. */
  submitting: boolean
  formError?: BackendFailure | null
  onSubmit: (draft: ConnectionDraft) => void
  onBrowse?: (field: FormField) => Promise<string | null>
  /**
   * Values the form starts with — a dropped database file's path. `key`
   * changes with each drop, so the same driver's form restarts from them.
   */
  prefill?: { key: number; values: Readonly<Record<string, string>> } | null

  /** The form's values are being opened and closed, without being saved. */
  testing: boolean
  /** The last test's answer, `null` until one describes the current values. */
  testResult: ConnectionTest | null
  /** The test call itself was rejected — a refusal, not a server's answer. */
  testError?: BackendFailure | null
  onTest: (draft: ConnectionDraft) => void
  /** The values changed: a previous result no longer describes them. */
  onDraftChange: () => void

  /** A cancellation was sent for what is opening. */
  cancelling: boolean
  onCancelOpening: () => void

  approval: PendingConnectionApproval | null
  deciding: boolean
  onDecide: (approved: boolean) => void

  /** Absent when no workspace stayed open: the action is then not shown. */
  onReturnToWorkspace?: () => void
  onOpenLocalWork?: () => void
}

/**
 * The start screen (docs/UX-SPEC.md « Écran d'accueil »): a one-row title bar
 * — the mark and two title lines on the left, the window's actions on the
 * right — over one centred column: the saved connections, then the database
 * types this build registers. On a first launch there is nothing to reopen,
 * so the types take the whole column. No connection is opened for the user.
 *
 * Every step has a visible way back, and the same keys everywhere: Esc, ⌘[
 * and Alt+←. From the new connection form they return to the database types,
 * after a confirmation when something was typed; from the screen itself, to
 * the workspace left open. Nothing is cancelled that way while an opening is
 * in flight: Esc then cancels the opening.
 *
 * Choosing a type replaces the column with its form. The page scrolls as a
 * whole and the form's actions stick to its bottom, so Back and Connect stay
 * in view however short the window.
 */
export function ConnectionScreenView(props: ConnectionScreenViewProps) {
  const {
    connections,
    connectionsError,
    onRetryConnections,
    drivers,
    driversError,
    onRetryDrivers,
    driver,
    duplicate = null,
    onChooseDriver,
    onLeaveDriver,
    opening,
    openError,
    onOpen,
    onRetryOpen,
    openConnectionId,
    connectionMenu,
    submitting,
    formError,
    onSubmit,
    onBrowse,
    prefill,
    testing,
    testResult,
    testError,
    onTest,
    onDraftChange,
    cancelling,
    onCancelOpening,
    approval,
    deciding,
    onDecide,
    onReturnToWorkspace,
    onOpenLocalWork,
  } = props
  const busy = opening !== null || submitting || testing
  const firstLaunch = connections?.length === 0 && !connectionsError
  const usedDrivers = React.useMemo(
    () =>
      Array.from(new Set(connections?.map((connection) => connection.driver))),
    [connections]
  )
  const [formDirty, setFormDirty] = React.useState(false)
  const [confirmingLeave, setConfirmingLeave] = React.useState(false)
  const [draftName, setDraftName] = React.useState("")
  // By identity: the form's store hands out a new object only when a value
  // changed, and the first one it reports is the form as it opened.
  const seenValues = React.useRef<FormValues | null>(null)
  const onValuesChange = React.useCallback(
    (values: FormValues) => {
      if (seenValues.current === values) return
      const changed = seenValues.current !== null
      seenValues.current = values
      setDraftName(values.name.trim())
      if (changed) onDraftChange()
    },
    [onDraftChange]
  )

  const leaveDriver = React.useCallback(() => {
    setConfirmingLeave(false)
    setFormDirty(false)
    seenValues.current = null
    setDraftName("")
    onLeaveDriver()
  }, [onLeaveDriver])

  const goBack = React.useCallback(() => {
    if (driver) {
      if (formDirty) setConfirmingLeave(true)
      else leaveDriver()
    } else {
      onReturnToWorkspace?.()
    }
  }, [driver, formDirty, leaveDriver, onReturnToWorkspace])

  // Never while something is in flight (Esc cancels it then) or under a
  // dialog. ⌘[ and Alt+← are the registry's `Back` (ADR-0041), which leaves
  // Alt+← to a field, where it moves by word. Esc stays here, in the bubble
  // phase: a list or a popover it would close sees it first. Inside a field
  // it leaves the form, as a dialog would; on the screen itself it waits for
  // the field to lose focus.
  const canGoBack = driver !== null || onReturnToWorkspace !== undefined
  const backOffered = canGoBack && !busy && !approval && !confirmingLeave
  useActionSource("navigation", backOffered ? {} : null, { back: goBack })
  React.useEffect(() => {
    if (!backOffered) return
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.key !== "Escape") return
      const target = event.target
      const typing =
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
      if (typing && driver === null) return
      event.preventDefault()
      goBack()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [backOffered, driver, goBack])

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b pr-3 pl-20"
      >
        {/* pl-20 keeps the macOS traffic lights of the overlay title bar clear. */}
        <img
          src={markUrl}
          alt=""
          className="size-6 rounded-md"
          draggable={false}
          data-tauri-drag-region
        />
        <div
          className="flex min-w-0 flex-col leading-tight"
          data-tauri-drag-region
        >
          <h1 className="text-sm font-semibold">Oxyn</h1>
          {driver ? (
            <Breadcrumb>
              <BreadcrumbList className="flex-nowrap gap-1 text-xs">
                <BreadcrumbItem>
                  <BreadcrumbLink
                    render={
                      <button
                        type="button"
                        onClick={goBack}
                        disabled={busy}
                        className="rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
                      />
                    }
                  >
                    Connections
                  </BreadcrumbLink>
                </BreadcrumbItem>
                <BreadcrumbSeparator className="[&>svg]:size-3" />
                <BreadcrumbItem className="min-w-0">
                  <BreadcrumbPage className="truncate text-xs">
                    New {driver.displayName} connection
                  </BreadcrumbPage>
                </BreadcrumbItem>
              </BreadcrumbList>
            </Breadcrumb>
          ) : (
            <p className="truncate text-xs text-muted-foreground">
              Open a connection to start working
            </p>
          )}
        </div>
        <div className="ml-auto flex items-center gap-1">
          {onOpenLocalWork ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={onOpenLocalWork}
              disabled={busy}
            >
              <HugeiconsIcon
                icon={FileClockIcon}
                strokeWidth={2}
                data-icon="inline-start"
              />
              Local work…
            </Button>
          ) : null}
          {onReturnToWorkspace ? (
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={onReturnToWorkspace}
                    disabled={busy}
                  />
                }
              >
                <HugeiconsIcon
                  icon={ArrowLeft01Icon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Return to workspace
                <Kbd>Esc</Kbd>
              </TooltipTrigger>
              <TooltipContent>
                The workspace you left is still open · ⌘[
              </TooltipContent>
            </Tooltip>
          ) : null}
        </div>
      </header>

      {/* The column is centred while it is shorter than the window and
          scrolls from its top once it is not: an auto margin, not
          justify-center, which would push its head out of reach. The page
          scrolls as a whole, so the form's sticky actions stay in view. */}
      <main className="min-h-0 flex-1 overflow-y-auto px-4 sm:px-6">
        <div className="mx-auto flex min-h-full w-full max-w-2xl flex-col py-8">
          <div className="my-auto flex flex-col gap-8">
            {driver ? (
              <Card
                aria-labelledby="new-connection-title"
                className="overflow-visible"
                role="region"
              >
                <CardHeader>
                  <CardTitle id="new-connection-title">
                    New {driver.displayName} connection
                  </CardTitle>
                  <CardDescription>
                    {duplicate ? (
                      <>
                        A copy of <bdi>{duplicate.of}</bdi>. Its secrets are not
                        copied: type them again. Nothing is saved until you
                        connect.
                      </>
                    ) : (
                      driver.family
                    )}
                  </CardDescription>
                </CardHeader>
                <CardContent className="flex flex-col gap-6">
                  <DraftStatusBar
                    name={draftName}
                    saving={submitting}
                    testing={testing}
                    result={testResult}
                    error={testError ?? null}
                  />
                  <ConnectionForm
                    key={`${driver.id}:${prefill?.key ?? ""}`}
                    driver={driver}
                    prefill={duplicate?.prefill}
                    dropped={prefill?.values}
                    submitting={submitting}
                    testing={testing}
                    aborting={cancelling}
                    error={formError}
                    onSubmit={onSubmit}
                    onTest={onTest}
                    onBrowse={onBrowse}
                    onCancel={goBack}
                    onAbort={onCancelOpening}
                    onDirtyChange={setFormDirty}
                    onValuesChange={onValuesChange}
                    stickyActions
                  />
                </CardContent>
              </Card>
            ) : (
              <>
                <div className="flex flex-col gap-1.5">
                  <h2 className="text-2xl font-semibold tracking-tight">
                    {firstLaunch
                      ? "Connect your first database"
                      : "Open a connection"}
                  </h2>
                  <p className="text-sm text-muted-foreground">
                    {firstLaunch
                      ? "Choose its type: Oxyn asks only for what that driver needs."
                      : "Pick up a saved connection, or connect to another database."}
                  </p>
                </div>

                {/* The empty list is not drawn: on a first launch the
                    database types are the whole screen, not a box saying
                    there is nothing here. */}
                {firstLaunch ? null : (
                  <section
                    aria-labelledby="saved-connections-title"
                    className="flex flex-col gap-3"
                  >
                    <h3
                      id="saved-connections-title"
                      className="text-xs font-medium tracking-wide text-muted-foreground uppercase"
                    >
                      Saved connections
                    </h3>
                    <SavedConnections
                      connections={connections}
                      error={connectionsError}
                      onRetry={onRetryConnections}
                      opening={submitting ? "" : opening}
                      cancelling={cancelling}
                      onOpen={onOpen}
                      onCancelOpening={onCancelOpening}
                      openConnectionId={openConnectionId}
                      menuActions={connectionMenu}
                    />
                  </section>
                )}
                {openError && opening === null ? (
                  <BackendErrorAlert
                    title="Connection failed"
                    error={openError}
                    onRetry={onRetryOpen}
                    nextStep="Edit the connection in Settings, then open it again."
                  />
                ) : null}

                <section
                  aria-labelledby="new-connection-title"
                  className="flex flex-col gap-3"
                >
                  <h3
                    id="new-connection-title"
                    className="text-xs font-medium tracking-wide text-muted-foreground uppercase"
                  >
                    {firstLaunch ? "Database type" : "New connection"}
                  </h3>
                  <DriverChoices
                    drivers={drivers}
                    used={usedDrivers}
                    error={driversError}
                    onRetry={onRetryDrivers}
                    disabled={busy}
                    prominent={firstLaunch}
                    onChoose={onChooseDriver}
                  />
                </section>
              </>
            )}

            <p className="flex items-center gap-2 text-xs text-muted-foreground">
              <HugeiconsIcon
                icon={LockIcon}
                strokeWidth={2}
                className="size-3.5 shrink-0"
              />
              Secrets are stored in the system keyring, never in the workspace
              file.
            </p>
          </div>
        </div>
      </main>

      <DiscardChangesDialog
        open={confirmingLeave}
        what={driver ? `this new ${driver.displayName} connection` : "it"}
        onKeep={() => setConfirmingLeave(false)}
        onDiscard={leaveDriver}
      />

      <ApprovalDialog
        approval={approval}
        connectionName={approval?.name ?? ""}
        environment={approval?.environment ?? "production"}
        deciding={deciding}
        onDecide={onDecide}
      />
    </div>
  )
}

/**
 * The form's status bar (docs/UX-SPEC.md « Navigation du premier workspace »):
 * the connection being prepared, then where it stands — `Not tested`, the
 * test's own answer, or the save in flight. It never says `Connected`: nothing
 * here is open, and a passed test is a fact about one moment, not a session.
 * The server's message is shown whole, as the audience reads it.
 */
function DraftStatusBar({
  name,
  saving,
  testing,
  result,
  error,
}: {
  name: string
  saving: boolean
  testing: boolean
  result: ConnectionTest | null
  error: BackendFailure | null
}) {
  const failure: string | null =
    result?.type === "failed" ? result.message : (error?.message ?? null)
  let state: React.ReactNode
  if (saving) state = "Saving…"
  else if (testing) state = "Testing…"
  else if (result?.type === "succeeded")
    state = (
      <span className="flex items-center gap-1.5 text-success">
        <HugeiconsIcon
          icon={CheckmarkCircle02Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0"
        />
        Test passed in {result.elapsedMs} ms
      </span>
    )
  else if (failure !== null)
    state = (
      <span className="flex items-center gap-1.5 text-destructive">
        <HugeiconsIcon
          icon={Alert02Icon}
          strokeWidth={2}
          className="size-3.5 shrink-0"
        />
        Test failed
      </span>
    )
  else state = "Not tested"

  return (
    <div
      role="status"
      aria-label="Connection status"
      data-slot="connection-draft-status"
      className="flex flex-col gap-1 rounded-md border bg-muted/40 px-3 py-2 text-xs"
    >
      <div className="flex min-w-0 items-center gap-3">
        <span
          className={cn(
            "min-w-0 truncate font-medium",
            name === "" && "font-normal text-muted-foreground italic"
          )}
          dir="auto"
        >
          {name === "" ? "Unnamed connection" : name}
        </span>
        <span className="ml-auto shrink-0 text-muted-foreground">{state}</span>
      </div>
      {failure !== null && !saving && !testing ? (
        <p className="font-mono break-words whitespace-pre-wrap text-destructive">
          {failure}
        </p>
      ) : null}
    </div>
  )
}
