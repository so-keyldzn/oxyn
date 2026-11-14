import * as React from "react"
import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import {
  Field,
  FieldGroup,
  FieldTitle,
  FieldDescription,
} from "@/components/ui/field"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from "@/components/ui/select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Progress } from "@/components/ui/progress"
import { Separator } from "@/components/ui/separator"
import type {
  MetadataLevel,
  MetadataLoadingPolicy,
  MetadataPreloadProgress,
  MetadataScope,
} from "@/lib/ipc/metadata-loading"

export interface LoadingScope {
  key: string
  label: string
  scope: MetadataScope
}
const levels: Array<{ value: MetadataLevel; label: string }> = [
  { value: "minimal", label: "Minimal" },
  { value: "structure", label: "Structure" },
  { value: "extended", label: "Extended" },
]

/** Local preferences and an explicitly started, bounded page of metadata. */
export function MetadataLoadingDialog({
  connection,
  policy,
  scopes,
  scopeKey,
  saving,
  needsReload = false,
  dirty = false,
  running,
  cancelling,
  report,
  error,
  onPolicy,
  onScope,
  onSave,
  onReload,
  onPreload,
  onCancel,
  onClose,
}: {
  connection: string
  policy: MetadataLoadingPolicy | null
  scopes: Array<LoadingScope>
  scopeKey: string | null
  saving: boolean
  needsReload?: boolean
  dirty?: boolean
  running: boolean
  cancelling: boolean
  report: MetadataPreloadProgress | null
  error: BackendFailure | null
  onPolicy: (policy: MetadataLoadingPolicy) => void
  onScope: (key: string) => void
  onSave: () => void
  onReload: () => void
  onPreload: (continuation: boolean) => void
  onCancel: () => void
  onClose: () => void
}) {
  const label = React.useId()
  const scopeLabel = React.useId()
  const overrideLabel = React.useId()
  const selected = scopes.find((scope) => scope.key === scopeKey)
  const override = policy?.overrides.find(
    (entry) => JSON.stringify(entry.scope) === scopeKey
  )
  const rootScope =
    selected?.scope.catalog === null && selected.scope.namespace === null
  const overrideItems = [
    { value: "inherit", label: "Use connection default" },
    ...levels,
  ]
  const updateOverride = (value: string | null) => {
    if (!policy || !selected || value === null) return
    const rest = policy.overrides.filter(
      (entry) => JSON.stringify(entry.scope) !== scopeKey
    )
    if (value !== "inherit")
      rest.push({ scope: selected.scope, level: value as MetadataLevel })
    onPolicy({ ...policy, overrides: rest })
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !running && !saving) onClose()
      }}
    >
      <DialogContent
        className="max-h-[90dvh] overflow-y-auto sm:max-w-2xl"
        showCloseButton={false}
      >
        <DialogHeader>
          <DialogTitle>Metadata loading</DialogTitle>
          <DialogDescription>
            <bdi>{connection}</bdi> · Preferences do not start reads or change
            open editors. Each view still reads the facets it needs.
          </DialogDescription>
        </DialogHeader>
        {error && (
          <BackendErrorAlert
            title="Metadata operation did not complete"
            error={error}
            nextStep="Read the saved preferences again before saving another change."
          />
        )}
        {!policy ? (
          <p role="status">
            {error
              ? "Saved preferences are unavailable."
              : "Reading saved preferences…"}
          </p>
        ) : (
          <FieldGroup>
            <Field data-disabled={saving}>
              <FieldTitle id={label}>Connection default</FieldTitle>
              <ToggleGroup
                aria-labelledby={label}
                variant="outline"
                value={[policy.default]}
                disabled={saving}
                onValueChange={(values) => {
                  const next = levels.find((level) => level.value === values[0])
                  if (next) onPolicy({ ...policy, default: next.value })
                }}
              >
                {levels.map((level) => (
                  <ToggleGroupItem key={level.value} value={level.value}>
                    {level.label}
                  </ToggleGroupItem>
                ))}
              </ToggleGroup>
              <FieldDescription>
                Minimal: names. Structure: descriptions, indexes and outgoing
                keys. Extended: structure and supported definitions. Documentary
                inference always requires a separate sample confirmation.
              </FieldDescription>
            </Field>
            <Field data-disabled={running || saving || scopes.length === 0}>
              <FieldTitle id={scopeLabel}>Native scope</FieldTitle>
              <Select
                items={scopes.map((scope) => ({
                  value: scope.key,
                  label: scope.label,
                }))}
                value={scopeKey}
                disabled={running || saving || scopes.length === 0}
                onValueChange={(value) => {
                  if (value !== null) onScope(value)
                }}
              >
                <SelectTrigger aria-labelledby={scopeLabel} className="w-full">
                  <SelectValue placeholder="Expand a native level in the explorer first" />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {scopes.map((scope) => (
                      <SelectItem key={scope.key} value={scope.key}>
                        {scope.label}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {scopes.length === 0 && (
                <FieldDescription>
                  No observed native scope yet. Close this dialog and expand the
                  desired level in the explorer.
                </FieldDescription>
              )}
            </Field>
            {selected && !rootScope && (
              <Field data-disabled={saving}>
                <FieldTitle id={overrideLabel}>Scope override</FieldTitle>
                <Select
                  items={overrideItems}
                  value={override?.level ?? "inherit"}
                  disabled={saving}
                  onValueChange={updateOverride}
                >
                  <SelectTrigger
                    aria-labelledby={overrideLabel}
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {overrideItems.map((item) => (
                        <SelectItem key={item.value} value={item.value}>
                          {item.label}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
                <FieldDescription>
                  Exact catalog and namespace identity. Other scopes keep their
                  own overrides.
                </FieldDescription>
              </Field>
            )}
          </FieldGroup>
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            disabled={saving || running}
            onClick={onReload}
          >
            Read saved preferences
          </Button>
          <Button disabled={!policy || saving || needsReload} onClick={onSave}>
            {saving ? "Saving…" : "Save preferences"}
          </Button>
        </div>
        {dirty && (
          <p role="status" className="text-sm">
            Unsaved preferences. Save them before starting or continuing a
            preload.
          </p>
        )}
        <Separator />
        <p className="text-sm text-muted-foreground">
          Preload uses saved preferences for the selected scope: up to 128
          relations’ details per page. Stop is requested after 60 seconds;
          protocol cleanup may take longer. Names keep the catalog’s existing
          limits. Interactive reads take priority. No document samples are read.
        </p>
        {report && (
          <div className="flex flex-col gap-2" role="status" aria-live="polite">
            <p className="text-sm">
              {[report.scope.catalog, report.scope.namespace]
                .filter((name) => name !== null)
                .join(" / ") || "Source"}{" "}
              · {report.state} · {report.level} · {report.loaded} read,{" "}
              {report.cached} already cached, {report.failed} failed,{" "}
              {report.unavailable} unsupported or requiring sampling.
            </p>
            <Progress
              aria-label="Metadata page progress"
              max={Math.max(1, report.planned)}
              value={Math.min(
                report.planned,
                report.loaded +
                  report.cached +
                  report.failed +
                  report.unavailable
              )}
            />
            <p className="text-sm text-muted-foreground">
              {report.knownRelations ?? "Unknown"} known relations;{" "}
              {report.unreadRelations} outside this page.{" "}
              {report.metadataBytes.toLocaleString()} encoded metadata bytes;{" "}
              {report.elapsedMs.toLocaleString()} ms. Bytes count published
              patches, not current cache size or network traffic.
            </p>
            {report.availableFacets === null ? (
              <p className="text-sm text-muted-foreground">
                Facet coverage is not measured yet.
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">
                Coverage at page end: {report.availableFacets} fresh,{" "}
                {report.staleFacets} stale and {report.unreadFacets}{" "}
                unread/evicted supported facets in this page.
              </p>
            )}
            {report.stopped && (
              <p className="text-sm">
                {report.stopped === "policyChanged"
                  ? "Saved preferences changed."
                  : report.stopped === "deadline"
                    ? "The preload time budget ended."
                    : report.stopped === "sourceClosed"
                      ? "The owning source closed."
                      : report.stopped === "sourceChanged"
                        ? "The source metadata generation changed."
                        : report.stopped === "continuationUnavailable"
                          ? "The next page cannot be represented within the identifier limit."
                          : "Preload was stopped."}{" "}
                Completed reads remain cached.
              </p>
            )}
            {report.samplingRequired > 0 && (
              <p className="text-sm">
                {report.samplingRequired} descriptions need explicit inference
                from the object’s Structure tab.
              </p>
            )}
            {report.failures.length > 0 && (
              <ul className="list-inside list-disc text-sm">
                {report.failures.map((failure, index) => (
                  <li key={index}>
                    <bdi>
                      {failure.relation ??
                        (failure.facet === "names"
                          ? "Scope names"
                          : "Object name omitted")}
                    </bdi>
                    : {failure.facet} not read.
                  </li>
                ))}
              </ul>
            )}
            {report.omittedFailures > 0 && (
              <p>{report.omittedFailures} further failures omitted.</p>
            )}
          </div>
        )}
        {running && !report && <p role="status">Starting metadata preload…</p>}
        <DialogFooter>
          <Button
            variant="outline"
            autoFocus
            disabled={running || saving}
            onClick={onClose}
          >
            Close
          </Button>
          {running ? (
            <Button variant="outline" disabled={cancelling} onClick={onCancel}>
              {cancelling ? "Cancelling…" : "Cancel preload"}
            </Button>
          ) : (
            <>
              <Button
                variant="outline"
                disabled={
                  !policy || !selected || saving || needsReload || dirty
                }
                onClick={() => onPreload(false)}
              >
                Preload scope
              </Button>
              {report?.next && (
                <Button
                  disabled={saving || needsReload || dirty}
                  onClick={() => onPreload(true)}
                >
                  Load next page
                </Button>
              )}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
