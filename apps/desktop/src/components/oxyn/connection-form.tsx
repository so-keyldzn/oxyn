import * as React from "react"
import { useForm, useStore } from "@tanstack/react-form"
import { cn } from "cn"
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
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Kbd } from "@/components/ui/kbd"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
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

export interface FormValues {
  name: string
  environment: Environment
  privacyTier: PrivacyTier
  readOnly: boolean
  values: Record<string, string>
}

function initialValues(
  driver: DriverChoice,
  existing: ConnectionDetails | undefined
): FormValues {
  const values: Record<string, string> = {}
  for (const field of driver.fields) {
    if (field.secret) {
      // Never pre-filled: a stored secret is not read back into the webview
      // (I-03). Empty means « keep what the keyring holds ».
      values[field.key] = ""
    } else if (existing) {
      values[field.key] =
        existing.values[field.key] ??
        (field.kind.type === "bool" ? "false" : "")
    } else {
      values[field.key] =
        field.default ?? (field.kind.type === "bool" ? "false" : "")
    }
  }
  return {
    name: existing?.name ?? "",
    // Every new connection starts as production until changed explicitly
    // (I-02, docs/UX-SPEC.md « Navigation du premier workspace »).
    environment: existing?.environment ?? "production",
    // ADR-0006's default, never Sampled.
    privacyTier: existing?.privacyTier ?? "metadata",
    readOnly: existing?.readOnly ?? false,
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
 * The labels of the required fields still empty.
 *
 * A stored secret left empty is kept, so it is not missing.
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
    if (field.secret && existing?.hasStoredSecrets) continue
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
 * is kept.
 *
 * While `submitting`, the only live action is `onAbort` (Esc), which cancels
 * the opening on the server side (docs/UX-SPEC.md « Annulation »). The submit
 * stays disabled while a required field is empty, and says which.
 *
 * The fields scroll and the actions do not: in a bounded parent (a flex
 * column with `min-h-0`), Back and Connect stay reachable however many fields
 * the driver declares. `stickyActions` keeps them at the bottom of a scrolling
 * page as well. `onDirtyChange` lets the parent confirm before a typed value
 * is abandoned.
 */
export function ConnectionForm({
  driver,
  existing,
  submitting = false,
  aborting = false,
  error,
  onSubmit,
  onBrowse,
  onCancel,
  onAbort,
  onDirtyChange,
  stickyActions = false,
}: {
  driver: DriverChoice
  existing?: ConnectionDetails
  submitting?: boolean
  /** The cancellation was sent and the backend has not answered yet. */
  aborting?: boolean
  error?: BackendFailure | null
  onSubmit: (draft: ConnectionDraft) => void
  onBrowse?: (field: FormField) => Promise<string | null>
  onCancel?: () => void
  onAbort?: () => void
  onDirtyChange?: (dirty: boolean) => void
  stickyActions?: boolean
}) {
  const editing = existing !== undefined
  const form = useForm({
    defaultValues: initialValues(driver, existing),
    onSubmit: ({ value }) => onSubmit(draftFrom(driver, value)),
  })
  const values = useStore(form.store, (state) => state.values)
  const dirty = useStore(form.store, (state) => state.isDirty)
  React.useEffect(() => {
    onDirtyChange?.(dirty)
  }, [dirty, onDirtyChange])
  const missing = missingFields(driver, values, existing)
  const malformed = driver.fields.some(
    (field) =>
      field.kind.type === "number" &&
      (values.values[field.key] ?? "") !== "" &&
      !/^\d+$/.test(values.values[field.key] ?? "")
  )
  const verb = editing ? "Save" : "Connect"
  const shownError = error && !submitting ? error : null
  const errorRef = React.useRef<HTMLDivElement>(null)

  // The failure lands at the end of the scrolling fields, next to the action
  // that caused it: bring it into view rather than leave it below the fold.
  React.useEffect(() => {
    if (shownError) errorRef.current?.scrollIntoView({ block: "nearest" })
  }, [shownError])

  // Esc cancels an opening in flight, from anywhere in the form.
  React.useEffect(() => {
    if (!submitting || !onAbort) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault()
        onAbort()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [submitting, onAbort])

  return (
    <form
      noValidate
      aria-busy={submitting || undefined}
      onSubmit={(event) => {
        event.preventDefault()
        // A second Enter while the first is in flight sends nothing.
        if (submitting || missing.length > 0 || malformed) return
        void form.handleSubmit()
      }}
      className="flex min-h-0 flex-1 flex-col"
    >
      {/* The padding keeps focus rings clear of the scroll edge. */}
      <div
        // While the fieldset is disabled nothing inside takes the focus: the
        // region itself must, or the keyboard cannot scroll it.
        tabIndex={submitting ? 0 : undefined}
        role={submitting ? "group" : undefined}
        aria-label={submitting ? "Connection details" : undefined}
        className="-m-1 flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto rounded-md p-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <fieldset disabled={submitting} className="contents">
          <FieldGroup>
            <form.Field name="name">
              {(field) => (
                <Field>
                  <FieldLabel htmlFor="connection-name">
                    Name
                    <span aria-hidden className="text-destructive">
                      *
                    </span>
                  </FieldLabel>
                  <Input
                    id="connection-name"
                    value={field.state.value}
                    onBlur={field.handleBlur}
                    onChange={(event) => field.handleChange(event.target.value)}
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
                  onChange={(environment) => field.handleChange(environment)}
                  disabled={submitting}
                />
              )}
            </form.Field>

            {driver.fields.map((spec) => (
              <form.Field key={spec.key} name={`values.${spec.key}`}>
                {(field) => (
                  <DriverField
                    spec={spec}
                    value={field.state.value}
                    onChange={(next) => field.handleChange(next)}
                    onBlur={field.handleBlur}
                    storedSecret={
                      editing && spec.secret && existing.hasStoredSecrets
                    }
                    editing={editing}
                    onBrowse={onBrowse}
                  />
                )}
              </form.Field>
            ))}

            <form.Field name="readOnly">
              {(field) => (
                <Field orientation="horizontal">
                  <FieldContent>
                    <FieldLabel htmlFor="connection-read-only">
                      Read only
                    </FieldLabel>
                    <FieldDescription>
                      Every write and DDL is refused on this connection, yours
                      included.
                    </FieldDescription>
                  </FieldContent>
                  <Switch
                    id="connection-read-only"
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
                  disabled={submitting}
                />
              )}
            </form.Field>
          </FieldGroup>
        </fieldset>

        {shownError ? (
          <div ref={errorRef}>
            <BackendErrorAlert
              title={editing ? "Connection not saved" : "Connection failed"}
              error={shownError}
              onRetry={() => void form.handleSubmit()}
              retryLabel={`${verb} again`}
              nextStep="Correct the connection details, then try again."
            />
          </div>
        ) : null}
      </div>

      <div
        data-slot="connection-form-actions"
        className={cn(
          "mt-4 flex shrink-0 flex-wrap items-center justify-end gap-2 border-t pt-4",
          stickyActions && "sticky bottom-0 bg-card pb-1"
        )}
      >
        {missing.length > 0 && !submitting ? (
          <p
            id="connection-form-missing"
            className="mr-auto text-xs text-muted-foreground"
          >
            Required: {missing.join(", ")}
          </p>
        ) : null}
        {submitting && onAbort ? (
          <>
            <p role="status" className="mr-auto text-xs text-muted-foreground">
              {aborting
                ? "Cancelling…"
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
            disabled={submitting}
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
        <Button
          type="submit"
          disabled={submitting || missing.length > 0 || malformed}
          aria-describedby={
            missing.length > 0 ? "connection-form-missing" : undefined
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
  editing,
  onBrowse,
}: {
  spec: FormField
  value: string
  onChange: (value: string) => void
  onBlur: () => void
  storedSecret: boolean
  editing: boolean
  onBrowse?: (field: FormField) => Promise<string | null>
}) {
  const id = `field-${spec.key}`
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
    case "choice":
      control = (
        <NativeSelect
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          aria-required={required || undefined}
          aria-describedby={describedBy}
        >
          <NativeSelectOption value="">Default</NativeSelectOption>
          {spec.kind.options.map((option) => (
            <NativeSelectOption key={option} value={option}>
              {option}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      )
      break
    case "path":
      control = (
        <InputGroup>
          <InputGroupInput
            id={id}
            value={value}
            onChange={(event) => onChange(event.target.value)}
            onBlur={onBlur}
            placeholder="/path/to/database.sqlite"
            aria-required={required || undefined}
            aria-describedby={describedBy}
            spellCheck={false}
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
        <Input
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
          spellCheck={false}
          className={spec.kind.type === "number" ? "tabular-nums" : undefined}
        />
      )
  }
  return (
    <Field
      data-invalid={invalidPort || undefined}
      orientation={spec.kind.type === "bool" ? "horizontal" : "vertical"}
    >
      <FieldLabel htmlFor={id}>
        {spec.label}
        {required ? (
          <span aria-hidden className="text-destructive">
            *
          </span>
        ) : null}
      </FieldLabel>
      {control}
      {editing && spec.secret ? (
        <FieldDescription id={`${id}-help`}>
          Leave empty to keep the stored value. It is never shown again.
        </FieldDescription>
      ) : spec.help ? (
        <FieldDescription id={`${id}-help`}>{spec.help}</FieldDescription>
      ) : null}
      <FieldError
        errors={
          invalidPort ? [{ message: `${spec.label} is digits only.` }] : []
        }
      />
    </Field>
  )
}
