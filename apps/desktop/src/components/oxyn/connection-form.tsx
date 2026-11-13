import * as React from "react"
import { useForm, useSelector } from "@tanstack/react-form"
import { HugeiconsIcon } from "@hugeicons/react"
import { FolderOpenIcon } from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { EnvironmentPicker } from "@/components/oxyn/environment-picker"
import { PrivacyTierField } from "@/components/oxyn/privacy-tier"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
} from "@/components/ui/input-group"
import { Kbd } from "@/components/ui/kbd"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type { ConnectionDetails } from "@/lib/ipc/settings"
import type {
  ConnectionDraft,
  DriverChoice,
  Environment,
  FormField,
  PrivacyTier,
} from "@/lib/ipc/types"
import { cn } from "@/lib/utils"
import { InputGroupTextInput, TextInput } from "./text-field"

export interface FormValues {
  name: string
  environment: Environment
  privacyTier: PrivacyTier
  readOnly: boolean
  values: Record<string, string>
}

/**
 * What a new connection starts from when it copies another (`Duplicate`):
 * its name and its declared non-secret parameters. Never a secret, and
 * neither the environment, the privacy tier nor the sqlite-vec switch — a
 * copy is a new connection, and starts as every new one does.
 */
export interface ConnectionPrefill {
  name: string
  readOnly: boolean
  values: Record<string, string>
}

/**
 * Parameters a copy does not inherit. Turning sqlite-vec on is trusting one
 * file with bundled C code (ADR-0054); a copy is a new connection, often to
 * another file, and starts with it off like every new one. `duplicatePrefill`
 * leaves them out too; the form drops them again for any other caller.
 */
export const NOT_COPIED: ReadonlySet<string> = new Set(["sqlite_vec"])

function initialValues(
  driver: DriverChoice,
  existing: ConnectionDetails | undefined,
  prefill: ConnectionPrefill | undefined,
  dropped: Readonly<Record<string, string>> | undefined
): FormValues {
  const values: Record<string, string> = {}
  const copied = existing?.values ?? prefill?.values
  for (const field of driver.fields) {
    if (field.secret) {
      // Never pre-filled: a stored secret is not read back into the webview
      // (I-03). Empty means « keep what the keyring holds » when editing, and
      // « type it » for a copy.
      values[field.key] = ""
    } else if (copied && !(prefill && !existing && NOT_COPIED.has(field.key))) {
      values[field.key] =
        copied[field.key] ?? (field.kind.type === "bool" ? "false" : "")
    } else {
      values[field.key] =
        dropped?.[field.key] ??
        field.default ??
        (field.kind.type === "bool" ? "false" : "")
    }
  }
  return {
    name: existing?.name ?? prefill?.name ?? "",
    // Every new connection starts as production until changed explicitly —
    // a copy of a staging one included (I-02, docs/UX-SPEC.md « First workspace
    // navigation »).
    environment: existing?.environment ?? "production",
    // ADR-0006's default, never Sampled: the tier belongs to the connection,
    // and a copy is another one (I-04).
    privacyTier: existing?.privacyTier ?? "metadata",
    readOnly: existing?.readOnly ?? prefill?.readOnly ?? false,
    values,
  }
}

/** Splits what goes to the configuration from what goes to the keyring. */
export function draftFrom(
  driver: DriverChoice,
  form: FormValues
): ConnectionDraft {
  const values: Record<string, string> = {}
  const secrets: Record<string, string> = {}
  for (const field of driver.fields) {
    const value = form.values[field.key] ?? ""
    if (value === "") continue
    if (field.secret) secrets[field.key] = value
    else values[field.key] = value
  }
  return {
    driver: driver.id,
    name: form.name.trim(),
    environment: form.environment,
    privacyTier: form.privacyTier,
    readOnly: form.readOnly,
    values,
    secrets,
  }
}

/**
 * Whether an edit changes a parameter of the saved connection — any
 * non-secret field, not only the host: its stored secrets are then forgotten
 * on save, and must be typed again for where it points now (SECURITY.md
 * « Secrets »). Compared trimmed, as the backend does.
 */
export function settingsChanged(
  driver: DriverChoice,
  form: FormValues,
  existing?: ConnectionDetails
): boolean {
  if (!existing) return false
  return driver.fields.some(
    (field) =>
      !field.secret &&
      (form.values[field.key] ?? "").trim() !==
        (existing.values[field.key] ?? "").trim()
  )
}

/** Whether an empty secret field keeps what the keyring holds. */
function keepsStoredSecret(
  driver: DriverChoice,
  form: FormValues,
  existing?: ConnectionDetails
): boolean {
  return (
    existing?.hasStoredSecrets === true &&
    !settingsChanged(driver, form, existing)
  )
}

/**
 * The labels of the required fields still empty.
 *
 * A stored secret left empty is kept, so it is not missing — unless a
 * parameter changed, which forgets it.
 */
export function missingFields(
  driver: DriverChoice,
  form: FormValues,
  existing?: ConnectionDetails
): Array<string> {
  const missing: Array<string> = []
  if (form.name.trim() === "") missing.push("Name")
  for (const field of driver.fields) {
    if (!field.required || field.kind.type === "bool") continue
    if ((form.values[field.key] ?? "").trim() !== "") continue
    if (field.secret && keepsStoredSecret(driver, form, existing)) continue
    missing.push(field.label)
  }
  return missing
}

/**
 * The form a driver describes, in the driver's order.
 *
 * Only registered drivers reach this form (ADR-0003). Secret fields are
 * password inputs and their values leave for the keyring, never the
 * configuration (I-03). With `existing`, it edits a saved connection: its
 * non-secret values are shown, its secrets are not, and a secret left empty
 * is kept — until a parameter changes: the stored secret would then reach a
 * destination it was not typed for, so it is forgotten and asked again.
 * With `prefill`, it creates a copy: the parameters are there, the secrets
 * are typed, and nothing is saved until the user connects.
 *
 * With `saved`, a new connection was saved and could not open: connecting
 * again edits it, with `existing`'s secret semantics. The secrets typed for
 * the first attempt are cleared from the form; left empty, the stored ones
 * are kept — until a parameter changes, and they are asked again.
 *
 * While `submitting`, the only live action is `onAbort` (Esc), which cancels
 * the opening on the server side (docs/UX-SPEC.md « Cancellation »). The submit
 * stays disabled while a required field is empty, and says which.
 *
 * With `onTest`, a Test action sends the same draft to be opened and closed
 * without being saved. While `testing`, the form is held as it is — the result
 * describes exactly those values — and `onAbort` cancels the test the same way.
 * `onValuesChange` reports every change, so the parent can tell a result from
 * the values it no longer describes.
 *
 * The fields scroll and the actions do not: in a bounded parent (a flex
 * column with `min-h-0`), Back and Connect stay reachable however many fields
 * the driver declares. `onDirtyChange` lets the parent confirm before a typed value
 * is abandoned.
 */
export function ConnectionForm({
  driver,
  existing,
  prefill,
  submitting = false,
  testing = false,
  aborting = false,
  error,
  saved = null,
  onSubmit,
  onTest,
  onBrowse,
  onCancel,
  onAbort,
  onDirtyChange,
  onValuesChange,
  dropped,
}: {
  driver: DriverChoice
  existing?: ConnectionDetails
  /** A new connection's starting values; ignored with `existing`. */
  prefill?: ConnectionPrefill
  submitting?: boolean
  /** A test of the current values is in flight. */
  testing?: boolean
  /** The cancellation was sent and the backend has not answered yet. */
  aborting?: boolean
  error?: BackendFailure | null
  /**
   * A new connection already saved, whose opening failed: `error` is then
   * the opening's, and the save is not undone. Ignored with `existing`.
   */
  saved?: ConnectionDetails | null
  onSubmit: (draft: ConnectionDraft) => void
  onTest?: (draft: ConnectionDraft) => void
  onBrowse?: (field: FormField) => Promise<string | null>
  onCancel?: () => void
  onAbort?: () => void
  onDirtyChange?: (dirty: boolean) => void
  onValuesChange?: (values: FormValues) => void
  /**
   * Values a new connection starts with, by field key — a dropped database
   * file fills its path field. A secret field is never prefilled (I-03).
   */
  dropped?: Readonly<Record<string, string>>
}) {
  const editing = existing !== undefined
  const formId = React.useId()
  // What the secrets were stored for: a change of parameter forgets them.
  const stored = existing ?? saved ?? undefined
  const savedAs = editing ? null : (saved?.name ?? null)
  const busy = submitting || testing
  const form = useForm({
    defaultValues: initialValues(driver, existing, prefill, dropped),
    onSubmit: ({ value }) => onSubmit(draftFrom(driver, value)),
  })
  const values = useSelector(form.store, (state) => state.values)
  const touched = useSelector(form.store, (state) => state.isDirty)
  // A failed opening can follow a successful save. Compare against that save,
  // not the empty form's original defaults; subsequent edits still need a guard.
  const dirty = saved
    ? values.name.trim() !== saved.name ||
      values.environment !== saved.environment ||
      values.privacyTier !== saved.privacyTier ||
      values.readOnly !== saved.readOnly ||
      settingsChanged(driver, values, saved) ||
      driver.fields.some(
        (field) => field.secret && (values.values[field.key] ?? "") !== ""
      )
    : touched
  React.useEffect(() => {
    onDirtyChange?.(dirty)
  }, [dirty, onDirtyChange])
  React.useEffect(() => {
    onValuesChange?.(values)
  }, [values, onValuesChange])
  // A secret typed before the save would otherwise be sent again with the
  // next attempt, to whatever destination it then names (I-03).
  React.useEffect(() => {
    if (!saved) return
    for (const field of driver.fields)
      if (field.secret) form.setFieldValue(`values.${field.key}`, "")
  }, [saved, driver, form])
  const missing = missingFields(driver, values, stored)
  const moved = settingsChanged(driver, values, stored)
  const malformed = driver.fields.some(
    (field) =>
      field.kind.type === "number" &&
      (values.values[field.key] ?? "") !== "" &&
      !/^\d+$/.test(values.values[field.key] ?? "")
  )
  const verb = editing ? "Save" : "Connect"
  const shownError = error && !busy ? error : null
  const errorRef = React.useRef<HTMLDivElement>(null)

  // The failure lands at the end of the scrolling fields, next to the action
  // that caused it: bring it into view rather than leave it below the fold.
  React.useEffect(() => {
    if (shownError) errorRef.current?.scrollIntoView({ block: "nearest" })
  }, [shownError])

  // Esc cancels an opening or a test in flight, from anywhere in the form.
  React.useEffect(() => {
    if (!busy || !onAbort) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault()
        onAbort()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [busy, onAbort])

  const ready = !busy && missing.length === 0 && !malformed

  return (
    <form
      noValidate
      aria-busy={busy || undefined}
      onSubmit={(event) => {
        event.preventDefault()
        // A second Enter while the first is in flight sends nothing.
        if (!ready) return
        void form.handleSubmit()
      }}
      className="flex min-h-0 min-w-0 flex-1 flex-col"
    >
      {/* The padding keeps focus rings clear of the scroll edge. */}
      <ScrollArea className="-mx-1 min-h-0 flex-1">
        <div
          data-slot="connection-form-fields"
          className="@container/connection-form flex min-w-0 flex-col gap-6 px-1 pt-1 pr-4 pb-5"
        >
          <FieldSet disabled={busy} className="contents">
            <FieldGroup className="gap-6">
              <FieldSet className="gap-4">
                <FieldLegend>Connection identity</FieldLegend>
                <form.Field name="name">
                  {(field) => (
                    <Field>
                      <FieldLabel htmlFor={`${formId}-name`}>
                        Name
                        <span aria-hidden className="text-destructive">
                          *
                        </span>
                      </FieldLabel>
                      <TextInput
                        id={`${formId}-name`}
                        value={field.state.value}
                        onBlur={field.handleBlur}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                        placeholder={`My ${driver.displayName} database`}
                        aria-required
                        autoComplete="off"
                        maxLength={200}
                        dir="auto"
                        autoFocus
                      />
                    </Field>
                  )}
                </form.Field>

                <form.Field name="environment">
                  {(field) => (
                    <EnvironmentPicker
                      value={field.state.value}
                      onChange={(environment) =>
                        field.handleChange(environment)
                      }
                      disabled={busy}
                    />
                  )}
                </form.Field>
              </FieldSet>
              <Separator />
              <FieldSet className="gap-4">
                <FieldLegend>Connection details</FieldLegend>
                <FieldGroup className="grid grid-cols-1 gap-x-4 gap-y-4 @min-[28rem]/connection-form:grid-cols-12">
                  {driver.fields.map((spec) => (
                    <form.Field key={spec.key} name={`values.${spec.key}`}>
                      {(field) => (
                        <DriverField
                          spec={spec}
                          value={field.state.value}
                          onChange={(next) => field.handleChange(next)}
                          onBlur={field.handleBlur}
                          storedSecret={
                            spec.secret &&
                            stored?.hasStoredSecrets === true &&
                            !moved
                          }
                          forgottenSecret={
                            spec.secret &&
                            stored?.hasStoredSecrets === true &&
                            moved
                          }
                          editing={stored !== undefined}
                          disabled={busy}
                          onBrowse={onBrowse}
                        />
                      )}
                    </form.Field>
                  ))}
                </FieldGroup>
              </FieldSet>
              <Separator />
              <FieldSet className="gap-4">
                <FieldLegend>Access and privacy</FieldLegend>
                <form.Field name="readOnly">
                  {(field) => (
                    <Field orientation="horizontal">
                      <FieldContent>
                        <FieldLabel htmlFor={`${formId}-read-only`}>
                          Read only
                        </FieldLabel>
                        <FieldDescription>
                          Every write and DDL is refused on this connection,
                          yours included.
                        </FieldDescription>
                      </FieldContent>
                      <Switch
                        id={`${formId}-read-only`}
                        checked={field.state.value}
                        onCheckedChange={(checked: boolean) =>
                          field.handleChange(checked)
                        }
                      />
                    </Field>
                  )}
                </form.Field>

                <form.Field name="privacyTier">
                  {(field) => (
                    <PrivacyTierField
                      value={field.state.value}
                      onChange={(tier) => field.handleChange(tier)}
                      disabled={busy}
                    />
                  )}
                </form.Field>
              </FieldSet>
            </FieldGroup>
          </FieldSet>

          {shownError ? (
            <div ref={errorRef}>
              <BackendErrorAlert
                title={
                  editing
                    ? "Connection not saved"
                    : savedAs !== null
                      ? "Saved, but not connected"
                      : "Connection failed"
                }
                context={
                  savedAs !== null ? (
                    <>
                      <bdi>{savedAs}</bdi> is saved in this workspace.
                      Connecting again updates it: no second connection is
                      saved.
                    </>
                  ) : undefined
                }
                error={shownError}
                onRetry={() => void form.handleSubmit()}
                retryLabel={`${verb} again`}
                nextStep="Correct the connection details, then try again."
              />
            </div>
          ) : null}
        </div>
      </ScrollArea>

      <div
        data-slot="connection-form-actions"
        className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t bg-card pt-4"
      >
        {missing.length > 0 && !busy ? (
          <p
            id={`${formId}-missing`}
            className="mr-auto text-xs text-muted-foreground"
          >
            Required: {missing.join(", ")}
          </p>
        ) : null}
        {busy && onAbort ? (
          <>
            <p role="status" className="mr-auto text-xs text-muted-foreground">
              {aborting
                ? "Cancelling…"
                : testing
                  ? "Testing…"
                  : editing
                    ? "Saving…"
                    : "Opening the connection…"}
            </p>
            <Button
              type="button"
              variant="outline"
              onClick={onAbort}
              disabled={aborting}
              aria-keyshortcuts="Escape"
            >
              Cancel <Kbd>Esc</Kbd>
            </Button>
          </>
        ) : onCancel ? (
          <Button
            type="button"
            variant="ghost"
            onClick={onCancel}
            disabled={busy}
            aria-keyshortcuts={editing ? undefined : "Escape"}
          >
            {editing ? (
              "Cancel"
            ) : (
              <>
                Back <Kbd>Esc</Kbd>
              </>
            )}
          </Button>
        ) : null}
        {onTest ? (
          <Button
            type="button"
            variant="outline"
            disabled={!ready}
            onClick={() => onTest(draftFrom(driver, values))}
            aria-describedby={
              missing.length > 0 ? `${formId}-missing` : undefined
            }
          >
            {testing ? <Spinner data-icon="inline-start" /> : null}
            Test
          </Button>
        ) : null}
        <Button
          type="submit"
          disabled={!ready}
          aria-describedby={
            missing.length > 0 ? `${formId}-missing` : undefined
          }
        >
          {submitting ? <Spinner data-icon="inline-start" /> : null}
          {verb}
        </Button>
      </div>
    </form>
  )
}

function DriverField({
  spec,
  value,
  onChange,
  onBlur,
  storedSecret,
  forgottenSecret,
  editing,
  disabled,
  onBrowse,
}: {
  spec: FormField
  value: string
  onChange: (value: string) => void
  onBlur: () => void
  storedSecret: boolean
  /** A secret is stored, and the changed parameters forget it on save. */
  forgottenSecret: boolean
  editing: boolean
  disabled: boolean
  onBrowse?: (field: FormField) => Promise<string | null>
}) {
  const id = React.useId()
  const required = spec.required && !storedSecret
  const describedBy =
    spec.help || (editing && spec.secret) ? `${id}-help` : undefined
  const invalidPort =
    spec.kind.type === "number" && value !== "" && !/^\d+$/.test(value)
  let control: React.ReactNode
  switch (spec.kind.type) {
    case "bool":
      control = (
        <Switch
          id={id}
          checked={value === "true"}
          onCheckedChange={(checked: boolean) =>
            onChange(checked ? "true" : "false")
          }
          aria-describedby={describedBy}
        />
      )
      break
    case "choice": {
      const options = [
        ...new Set(["", ...spec.kind.options, ...(value ? [value] : [])]),
      ]
      control = (
        <Select
          disabled={disabled}
          items={options.map((option) => ({
            value: option,
            label: option || "Default",
          }))}
          value={value}
          onValueChange={(next) => {
            if (next !== null) onChange(next)
          }}
        >
          <SelectTrigger
            id={id}
            aria-label={spec.label}
            aria-required={required || undefined}
            aria-describedby={describedBy}
            className="w-full"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent alignItemWithTrigger={false}>
            <SelectGroup>
              {options.map((option) => (
                <SelectItem key={option} value={option}>
                  {option || "Default"}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      )
      break
    }
    case "path":
      control = (
        <InputGroup>
          <InputGroupTextInput
            id={id}
            value={value}
            onChange={(event) => onChange(event.target.value)}
            onBlur={onBlur}
            // Any driver's file — a database, a certificate, a key: the
            // label and the help say which.
            placeholder="/path/to/file"
            aria-required={required || undefined}
            aria-describedby={describedBy}
            dir="auto"
          />
          {onBrowse ? (
            <InputGroupAddon align="inline-end">
              <InputGroupButton
                onClick={async () => {
                  const chosen = await onBrowse(spec)
                  if (chosen) onChange(chosen)
                }}
              >
                <HugeiconsIcon
                  icon={FolderOpenIcon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Browse…
              </InputGroupButton>
            </InputGroupAddon>
          ) : null}
        </InputGroup>
      )
      break
    default:
      control = (
        <TextInput
          id={id}
          // A port is digits, not a number input: a scrolled wheel must not
          // change it, and a spinner means nothing for 5432.
          type={spec.kind.type === "password" ? "password" : "text"}
          inputMode={spec.kind.type === "number" ? "numeric" : undefined}
          value={value}
          onBlur={onBlur}
          onChange={(event) => onChange(event.target.value)}
          placeholder={storedSecret ? "Stored in the keyring" : undefined}
          aria-required={required || undefined}
          aria-invalid={invalidPort || undefined}
          aria-describedby={describedBy}
          autoComplete={spec.secret ? "new-password" : "off"}
          className={spec.kind.type === "number" ? "tabular-nums" : undefined}
        />
      )
  }
  const label = (
    <FieldLabel htmlFor={id}>
      {spec.label}
      {required ? (
        <span aria-hidden className="text-destructive">
          *
        </span>
      ) : null}
    </FieldLabel>
  )
  const description =
    editing && spec.secret ? (
      // Polite: the text changes as the user types in another field.
      <FieldDescription id={`${id}-help`} aria-live="polite">
        {forgottenSecret
          ? `Connection settings changed — enter the ${spec.label.toLowerCase()} again. The stored one is removed on save.`
          : "Leave empty to keep the stored value. It is never shown again."}
      </FieldDescription>
    ) : spec.help ? (
      <FieldDescription id={`${id}-help`}>{spec.help}</FieldDescription>
    ) : null
  return (
    <Field
      data-connection-field={spec.key}
      data-invalid={invalidPort || undefined}
      orientation={spec.kind.type === "bool" ? "horizontal" : "vertical"}
      className={cn(
        "min-w-0",
        spec.key === "host"
          ? "@min-[28rem]/connection-form:col-span-8"
          : spec.key === "port"
            ? "@min-[28rem]/connection-form:col-span-4"
            : spec.kind.type === "path" ||
                spec.kind.type === "bool" ||
                spec.secret ||
                spec.help
              ? "@min-[28rem]/connection-form:col-span-12"
              : "@min-[28rem]/connection-form:col-span-6"
      )}
    >
      {spec.kind.type === "bool" ? (
        <>
          <FieldContent className="min-w-0">
            {label}
            {description}
          </FieldContent>
          {control}
        </>
      ) : (
        <>
          {label}
          {control}
          {description}
        </>
      )}
      <FieldError
        errors={
          invalidPort ? [{ message: `${spec.label} is digits only.` }] : []
        }
      />
    </Field>
  )
}
