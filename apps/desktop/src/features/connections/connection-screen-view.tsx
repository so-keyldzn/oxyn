import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowLeft01Icon,
  FileClockIcon,
  LockIcon,
} from "@hugeicons/core-free-icons"

import markUrl from "@/assets/oxyn-mark.png"
import { ApprovalDialog } from "@/components/oxyn/approval-dialog"
import type { PendingApproval } from "@/components/oxyn/approval-dialog"
import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { ConnectionForm } from "@/components/oxyn/connection-form"
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
  DriverChoice,
  Environment,
  FormField,
} from "@/lib/ipc/types"

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
  onChooseDriver: (driver: DriverChoice) => void
  onLeaveDriver: () => void

  /** The saved connection being opened, by id. */
  opening: string | null
  openError?: BackendFailure | null
  onOpen: (connection: ConnectionSummary) => void
  onRetryOpen?: () => void

  /** A new connection being created and opened. */
  submitting: boolean
  formError?: BackendFailure | null
  onSubmit: (draft: ConnectionDraft) => void
  onBrowse?: (field: FormField) => Promise<string | null>

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
    onChooseDriver,
    onLeaveDriver,
    opening,
    openError,
    onOpen,
    onRetryOpen,
    submitting,
    formError,
    onSubmit,
    onBrowse,
    cancelling,
    onCancelOpening,
    approval,
    deciding,
    onDecide,
    onReturnToWorkspace,
    onOpenLocalWork,
  } = props
  const busy = opening !== null || submitting
  const firstLaunch = connections?.length === 0 && !connectionsError
  const usedDrivers = React.useMemo(
    () =>
      Array.from(new Set(connections?.map((connection) => connection.driver))),
    [connections]
  )
  const [formDirty, setFormDirty] = React.useState(false)
  const [confirmingLeave, setConfirmingLeave] = React.useState(false)

  const leaveDriver = React.useCallback(() => {
    setConfirmingLeave(false)
    setFormDirty(false)
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
  // dialog. Esc inside a field leaves the form, as a dialog would; on the
  // screen itself it waits for the field to lose focus, and Alt+← inside a
  // field moves by word, so it is left to the field.
  const canGoBack = driver !== null || onReturnToWorkspace !== undefined
  React.useEffect(() => {
    if (!canGoBack || busy || approval || confirmingLeave) return
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return
      const target = event.target
      const typing =
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
      const back =
        (event.key === "Escape" && (driver !== null || !typing)) ||
        (event.key === "[" &&
          (event.metaKey || event.ctrlKey) &&
          !event.altKey &&
          !event.shiftKey) ||
        (event.key === "ArrowLeft" &&
          event.altKey &&
          !event.metaKey &&
          !event.ctrlKey &&
          !typing)
      if (!back) return
      event.preventDefault()
      goBack()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [canGoBack, busy, approval, confirmingLeave, driver, goBack])

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
                  <CardDescription>{driver.family}</CardDescription>
                </CardHeader>
                <CardContent className="flex flex-col">
                  <ConnectionForm
                    key={driver.id}
                    driver={driver}
                    submitting={submitting}
                    aborting={cancelling}
                    error={formError}
                    onSubmit={onSubmit}
                    onBrowse={onBrowse}
                    onCancel={goBack}
                    onAbort={onCancelOpening}
                    onDirtyChange={setFormDirty}
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
