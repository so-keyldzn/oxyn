import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon } from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field"
import { Progress } from "@/components/ui/progress"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type { ModelState, SemanticSnapshot } from "@/lib/ipc/semantic"

import { downloadedBytes } from "./update-model"

export interface SemanticRankingSettingsProps {
  snapshot: SemanticSnapshot
  /** A switch request is on its way to the backend: the controls wait. */
  pending?: boolean
  /** The backend refused the last request; nothing was changed. */
  refused?: string | null
  /** Turns the option on and downloads; also « Download again ». */
  onEnable: () => void
  /** Turns the option off: stops a download, deletes the model. */
  onDisable: () => void
}

/**
 * Settings ▸ Semantic ranking (ADR-0056): the switch, what it costs, and
 * where the local model stands, with the actions that state allows.
 *
 * Off by default. Turning it on downloads the model; turning it off — or
 * cancelling the download — deletes it. The model runs on this computer:
 * the view says that nothing is sent, and what memory and disk it takes.
 */
export function SemanticRankingSettings({
  snapshot,
  pending = false,
  refused = null,
  onEnable,
  onDisable,
}: SemanticRankingSettingsProps) {
  const { enabled, model } = snapshot
  const switchId = React.useId()
  const unavailable = model.type === "unavailable"
  const busy = isBusy(model)

  return (
    <div className="flex flex-col gap-6 text-sm">
      <section className="flex flex-col gap-1">
        <h3 className="font-medium">Semantic ranking</h3>
        <p className="text-muted-foreground">
          Before a question reaches the model, Oxyn keeps the tables that
          matter. By default it matches their names; semantic ranking also ranks
          them by meaning, so a question that names no table, or asks in another
          language than the schema, still finds the right ones.
        </p>
      </section>

      <section className="flex flex-col gap-2">
        <Field orientation="horizontal" data-disabled={unavailable}>
          <FieldContent>
            <FieldLabel htmlFor={switchId}>
              Rank tables by meaning with a local model
            </FieldLabel>
            <FieldDescription>
              Computed on this computer: no table name, comment or question is
              sent anywhere. Downloads about 220 MB once, keeps about 415 MB on
              disk, and uses about 800 MB of memory while you ask questions —
              released 5 minutes after the last one.
            </FieldDescription>
            {enabled ? (
              <FieldDescription>
                Turning this off deletes the model from this computer.
              </FieldDescription>
            ) : null}
          </FieldContent>
          <Switch
            id={switchId}
            checked={enabled || busy}
            disabled={unavailable || pending}
            onCheckedChange={(next: boolean) => {
              if (next) onEnable()
              else onDisable()
            }}
          />
        </Field>
        {refused ? (
          <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <HugeiconsIcon
              icon={Alert02Icon}
              strokeWidth={2}
              className="size-3.5 text-warning"
              aria-hidden
            />
            <span data-selectable className="text-foreground">
              Nothing was changed: {refused}
            </span>
          </p>
        ) : null}
      </section>

      <section className="flex flex-col gap-3" aria-label="Model status">
        {/* Always mounted, so a change is announced; it holds the state only —
            the bytes beside it are never read aloud. A failure is announced
            by its alert. */}
        <p
          className={
            model.type === "failed" || model.type === "corrupt"
              ? "sr-only"
              : "flex flex-wrap items-center gap-x-1"
          }
        >
          {busy ? <Spinner className="mr-1 size-3.5" aria-hidden /> : null}
          <span role="status" aria-live="polite">
            {model.type === "failed" || model.type === "corrupt"
              ? null
              : statusLine(model, enabled)}
          </span>
          {model.type === "downloading" ? (
            <span className="tabular-nums">
              — {downloadedBytes(model.received, model.total)}
            </span>
          ) : null}
        </p>
        <ModelView
          model={model}
          enabled={enabled}
          pending={pending}
          onEnable={onEnable}
          onDisable={onDisable}
        />
      </section>
    </div>
  )
}

function isBusy(model: ModelState) {
  return (
    model.type === "verifying" ||
    model.type === "downloading" ||
    model.type === "converting"
  )
}

/** The words of each state, for the polite live region. */
function statusLine(model: ModelState, enabled: boolean) {
  switch (model.type) {
    case "unavailable":
      return "Not available: this system gives Oxyn no data directory to keep the model in."
    case "absent":
      return enabled
        ? "The model is not downloaded: questions are ranked by name only."
        : "Off. Questions are ranked by name only."
    case "verifying":
      return "Checking the files already on this computer…"
    case "downloading":
      return "Downloading the model"
    case "converting":
      return "Preparing the model for this computer… This last step takes a few seconds and cannot be cancelled."
    case "ready":
      return enabled
        ? "Ready. Loaded at the first question, released after 5 minutes without one."
        : "The model is on this computer, but this workspace does not use it."
    case "corrupt":
      return "The model files are damaged."
    case "failed":
      return "The local model is not usable."
  }
}

function ModelView({
  model,
  enabled,
  pending,
  onEnable,
  onDisable,
}: {
  model: ModelState
  enabled: boolean
  pending: boolean
  onEnable: () => void
  onDisable: () => void
}) {
  const remove = (
    <Button variant="outline" size="sm" disabled={pending} onClick={onDisable}>
      Delete model
    </Button>
  )
  switch (model.type) {
    case "unavailable":
      return null
    case "absent":
      return enabled ? (
        <Actions>
          <Button size="sm" disabled={pending} onClick={onEnable}>
            Download model
          </Button>
        </Actions>
      ) : null
    case "verifying":
    case "downloading":
      return (
        <>
          <DownloadProgress model={model} />
          <Actions>
            <Button
              variant="outline"
              size="sm"
              disabled={pending}
              onClick={onDisable}
            >
              Cancel download
            </Button>
          </Actions>
          <p className="text-xs text-muted-foreground">
            Cancelling turns semantic ranking off and removes what was
            downloaded.
          </p>
        </>
      )
    case "converting":
      return <DownloadProgress model={model} />
    case "ready":
      return <Actions>{remove}</Actions>
    case "corrupt":
      return (
        <Alert data-slot="model-corrupt">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>The model files are damaged</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>
              A file is not the one Oxyn expects: it was truncated — a full
              disk, for instance — or replaced. Until it is downloaded again,
              questions are ranked by name only.
            </p>
            <Actions>
              <Button size="sm" disabled={pending} onClick={onEnable}>
                Download again
              </Button>
              {remove}
            </Actions>
          </AlertDescription>
        </Alert>
      )
    case "failed":
      return (
        <BackendErrorAlert
          title="The local model is not usable"
          error={{ message: model.message, retryable: model.retryable }}
          onRetry={pending ? undefined : onEnable}
          retryLabel="Download again"
          nextStep="Check the disk space and the permissions of Oxyn's data directory, then download again."
        >
          <p className="text-xs text-muted-foreground">
            Until the model is downloaded, questions are ranked by name only.
          </p>
          <Actions>
            {model.retryable ? null : (
              <Button size="sm" disabled={pending} onClick={onEnable}>
                Download again
              </Button>
            )}
            {remove}
          </Actions>
        </BackendErrorAlert>
      )
  }
}

function Actions({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-wrap gap-2">{children}</div>
}

function DownloadProgress({ model }: { model: ModelState }) {
  // Only the bytes give a fraction; checking and preparing have none to give.
  const value =
    model.type === "downloading" && model.total > 0
      ? Math.min(100, Math.round((model.received / model.total) * 100))
      : null
  const spoken =
    model.type === "downloading"
      ? downloadedBytes(model.received, model.total)
      : model.type === "converting"
        ? "Preparing the model"
        : "Checking files"
  return (
    <Progress
      value={value}
      aria-label="Model download progress"
      getAriaValueText={() => spoken}
    />
  )
}
