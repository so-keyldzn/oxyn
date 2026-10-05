import * as React from "react"
import { useForm, useStore } from "@tanstack/react-form"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, CheckmarkCircle02Icon } from "@hugeicons/core-free-icons"

import { ModelPicker } from "@/components/oxyn/model-picker"
import {
  CREDENTIALS_IN_ENDPOINT,
  FAILURE_TITLES,
  KINDS,
  counted,
  endpointCarriesCredentials,
  kindLabel,
  parseArguments,
  parseEnvironment,
} from "@/components/oxyn/provider-settings-model"
import { useDraftModels } from "@/components/oxyn/use-draft-models"
import type {
  ConnectionTest,
  ListDraftModels,
} from "@/components/oxyn/use-draft-models"
import type { ProviderSettingsFailure } from "@/components/oxyn/provider-settings-model"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type {
  AgentDraft,
  DeclaredProvider,
  ExternalAgent,
  ProviderDraft,
  ProviderKind,
} from "@/lib/ipc/ai"
import { TextArea, TextInput } from "./text-field"

/** What the form works on: a new declaration, or one already saved. */
export type FormTarget =
  | { kind: "provider"; provider: DeclaredProvider }
  | { kind: "agent"; agent: ExternalAgent }
  | null

export interface ProviderFormProps {
  target: FormTarget
  working: boolean
  /** A failed save of this form; other failures are shown by the screen. */
  failure: ProviderSettingsFailure | null
  /** Resolves `true` once saved; the form keeps its values otherwise. */
  onSaveProvider: (draft: ProviderDraft) => Promise<boolean>
  /** Resolves `true` once saved; a draft with an `id` replaces that agent. */
  onSaveAgent: (draft: AgentDraft) => Promise<boolean>
  /** Leaves an edit, saved or cancelled. */
  onDone: () => void
  /** Lists the models of the provider being typed; saves nothing. */
  onListModels: ListDraftModels
}

type Kind = ProviderKind | "agent"

interface FormValues {
  kind: Kind
  label: string
  endpoint: string
  model: string
  key: string
  clearKey: boolean
  command: string
  args: string
  env: string
}

const EMPTY_FORM: FormValues = {
  kind: "anthropic",
  label: "",
  endpoint: "",
  model: "",
  key: "",
  clearKey: false,
  command: "",
  args: "",
  env: "",
}

function initialValues(target: FormTarget): FormValues {
  if (target?.kind === "provider") {
    const { provider } = target
    return {
      ...EMPTY_FORM,
      kind: provider.kind,
      label: provider.label,
      // A redacted endpoint is not prefilled: saving it would lose the part
      // that was hidden. Empty keeps the stored one.
      endpoint: provider.endpointRedacted ? "" : provider.endpoint,
      model: provider.model,
    }
  }
  if (target?.kind === "agent") {
    // The IPC returns neither the arguments nor the environment values: only
    // what it sends back is prefilled, and the screen says what is missing.
    return {
      ...EMPTY_FORM,
      kind: "agent",
      label: target.agent.label,
      command: target.agent.command,
    }
  }
  return EMPTY_FORM
}

function title(target: FormTarget) {
  if (target?.kind === "provider") return `Edit “${target.provider.label}”`
  if (target?.kind === "agent") return `Replace “${target.agent.label}”`
  return "Declare a provider"
}

function secretByName(name: string) {
  return /(?:_API_KEY|_TOKEN|_SECRET)$|PASSWORD|^(?:API_KEY|TOKEN|SECRET)$/i.test(
    name
  )
}

function required(label: string) {
  return ({ value }: { value: string }) =>
    value.trim() === "" ? { message: `${label} is required.` } : undefined
}

function invalid(errors: ReadonlyArray<unknown>) {
  return errors.length > 0 || undefined
}

/** What replacing an agent does not carry over, said before saving. */
function ReplacementNote({ agent }: { agent: ExternalAgent }) {
  return (
    <p className="text-xs text-muted-foreground">
      Oxyn does not show the arguments
      {agent.argCount > 0
        ? ` (${counted(agent.argCount, "argument", "arguments")})`
        : ""}{" "}
      nor the environment
      {agent.envNames.length > 0 ? ` (${agent.envNames.join(", ")})` : ""} of
      the current declaration: type the arguments again, the environment is not
      carried over. Saving replaces “{agent.label}” with this declaration.
    </p>
  )
}

/** « Test connection »: lists the models afresh, and says what came back. */
function ConnectionTestRow({
  test,
  disabled,
  onTest,
}: {
  test: ConnectionTest
  disabled: boolean
  onTest: () => void
}) {
  return (
    <Field orientation="horizontal" className="items-center">
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="shrink-0"
        disabled={disabled || test.status === "testing"}
        onClick={onTest}
      >
        {test.status === "testing" ? (
          <Spinner data-icon="inline-start" />
        ) : null}
        Test connection
      </Button>
      <p role="status" aria-live="polite" className="text-sm">
        {test.status === "ok" ? (
          <span className="inline-flex items-center gap-1.5">
            <HugeiconsIcon
              icon={CheckmarkCircle02Icon}
              strokeWidth={2}
              className="size-4 shrink-0"
            />
            Connected — {counted(test.count, "model", "models")} available
          </span>
        ) : test.status === "failed" ? (
          <span data-selectable className="text-destructive">
            Connection failed — {test.message}
          </span>
        ) : (
          <span className="text-muted-foreground">
            Lists the models with these settings. Saves nothing.
          </span>
        )}
      </p>
    </Field>
  )
}

/**
 * Declares a provider or an external agent, or edits one already saved.
 *
 * Nothing here is optimistic (docs/UX-SPEC.md): the values stay until the
 * backend says the declaration is saved. Only the key leaves at once — sent,
 * then forgotten by the screen, whatever the outcome (I-03).
 */
export function ProviderForm({
  target,
  working,
  failure,
  onSaveProvider,
  onSaveAgent,
  onDone,
  onListModels,
}: ProviderFormProps) {
  const [secretNames, setSecretNames] = React.useState<Record<string, boolean>>(
    {}
  )
  const formRef = React.useRef<HTMLFormElement>(null)
  const nameRef = React.useRef<HTMLInputElement>(null)
  const replacing = target?.kind === "agent" ? target.agent : null

  // Opening an edit from the list lands on the form, not on the list.
  React.useEffect(() => {
    if (target === null) return
    formRef.current?.scrollIntoView({ block: "start" })
    nameRef.current?.focus({ preventScroll: true })
  }, [target])

  const form = useForm({
    defaultValues: initialValues(target),
    onSubmit: async ({ value, formApi }) => {
      const saving =
        value.kind === "agent"
          ? onSaveAgent({
              id: replacing?.id ?? null,
              label: value.label.trim(),
              command: value.command.trim(),
              args: parseArguments(value.args),
              // Validated on submit: a line without a name never gets here.
              env: (() => {
                const parsed = parseEnvironment(value.env)
                return parsed.ok
                  ? parsed.env.map((variable) => ({
                      ...variable,
                      secret:
                        secretByName(variable.name) ||
                        secretNames[variable.name] === true,
                    }))
                  : []
              })(),
            })
          : onSaveProvider({
              id: target?.kind === "provider" ? target.provider.id : null,
              kind: value.kind,
              label: value.label.trim(),
              baseUrl: value.endpoint.trim(),
              model: value.model.trim(),
              key: value.key.trim() === "" ? null : value.key,
              clearKey: value.clearKey,
            })
      // The key and the environment leave once, and the screen forgets them
      // at once, whatever the outcome: a value there is as often a token (I-03).
      formApi.setFieldValue("key", "")
      formApi.setFieldValue("env", "")
      setSecretNames({})
      if (!(await saving)) return
      if (target) onDone()
      else formApi.reset()
    },
  })

  const editing = target?.kind === "provider" ? target.provider : null
  // What the model list depends on, read above the fields: a hook cannot run
  // inside a field's render.
  const listedKind = useStore(form.store, (state) => state.values.kind)
  const endpoint = useStore(form.store, (state) => state.values.endpoint)
  const key = useStore(form.store, (state) => state.values.key)
  const stored = React.useMemo(
    () =>
      editing
        ? {
            id: editing.id,
            endpoint: editing.endpoint,
            keyConfigured: editing.keyConfigured,
          }
        : null,
    [editing]
  )
  const draftModels = useDraftModels(
    listedKind === "agent" || endpointCarriesCredentials(endpoint)
      ? null
      : { kind: listedKind, endpoint, key, editing: stored },
    onListModels
  )

  return (
    <form
      ref={formRef}
      noValidate
      aria-label={title(target)}
      className="flex scroll-mt-4 flex-col gap-4 rounded-lg border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault()
        void form.handleSubmit()
      }}
    >
      <h3 className="text-sm font-medium">{title(target)}</h3>
      {replacing ? <ReplacementNote agent={replacing} /> : null}
      <FieldGroup>
        <form.Field name="kind">
          {(field) => (
            <Field>
              <FieldLabel htmlFor="provider-kind">Kind</FieldLabel>
              <NativeSelect
                id="provider-kind"
                value={field.state.value}
                disabled={target !== null}
                onChange={(event) =>
                  field.handleChange(event.target.value as Kind)
                }
              >
                {KINDS.filter(
                  (entry) => entry.offered || entry.kind === field.state.value
                ).map((entry) => (
                  <NativeSelectOption key={entry.kind} value={entry.kind}>
                    {entry.label}
                  </NativeSelectOption>
                ))}
                {editing ? null : (
                  <NativeSelectOption value="agent">
                    External agent
                  </NativeSelectOption>
                )}
              </NativeSelect>
            </Field>
          )}
        </form.Field>

        <form.Field name="label" validators={{ onSubmit: required("A name") }}>
          {(field) => (
            <Field data-invalid={invalid(field.state.meta.errors)}>
              <FieldLabel htmlFor="provider-label">Name</FieldLabel>
              <TextInput
                ref={nameRef}
                id="provider-label"
                value={field.state.value}
                maxLength={128}
                aria-invalid={invalid(field.state.meta.errors)}
                onBlur={field.handleBlur}
                onChange={(event) => field.handleChange(event.target.value)}
              />
              <FieldError errors={field.state.meta.errors} />
            </Field>
          )}
        </form.Field>

        <form.Subscribe selector={(state) => state.values.kind}>
          {(kind) =>
            kind === "agent" ? (
              <>
                <form.Field
                  name="command"
                  validators={{ onSubmit: required("A program") }}
                >
                  {(field) => (
                    <Field data-invalid={invalid(field.state.meta.errors)}>
                      <FieldLabel htmlFor="agent-command">Program</FieldLabel>
                      <TextInput
                        id="agent-command"
                        className="font-mono"
                        placeholder="claude-agent-acp"
                        value={field.state.value}
                        aria-invalid={invalid(field.state.meta.errors)}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldError errors={field.state.meta.errors} />
                    </Field>
                  )}
                </form.Field>
                <form.Field name="args">
                  {(field) => (
                    <Field>
                      <FieldLabel htmlFor="agent-args">Arguments</FieldLabel>
                      <TextArea
                        id="agent-args"
                        className="font-mono"
                        rows={3}
                        placeholder={"--acp"}
                        value={field.state.value}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldDescription>
                        Run without a shell. Oxyn asks you to confirm the exact
                        command in a system dialog, and never lets an agent
                        serve a local-only connection: nothing says where it
                        sends data. One argument per line, exactly as the
                        program receives it: no quotes needed.
                      </FieldDescription>
                    </Field>
                  )}
                </form.Field>
                <form.Field
                  name="env"
                  validators={{
                    onSubmit: ({ value }) => {
                      const parsed = parseEnvironment(value)
                      return parsed.ok
                        ? undefined
                        : {
                            message: `Line ${parsed.line} is not NAME=value.`,
                          }
                    },
                  }}
                >
                  {(field) => (
                    <Field data-invalid={invalid(field.state.meta.errors)}>
                      <FieldLabel htmlFor="agent-env">Environment</FieldLabel>
                      <TextArea
                        id="agent-env"
                        className="font-mono"
                        autoComplete="off"
                        rows={2}
                        placeholder="NAME=value"
                        value={field.state.value}
                        aria-invalid={invalid(field.state.meta.errors)}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldError errors={field.state.meta.errors} />
                      {(() => {
                        const parsed = parseEnvironment(field.state.value)
                        if (!parsed.ok) return null
                        return [
                          ...new Set(
                            parsed.env.map((variable) => variable.name)
                          ),
                        ].map((name, index) => (
                          <Field key={name} orientation="horizontal">
                            <Switch
                              id={`agent-env-secret-${index}`}
                              checked={
                                secretByName(name) || secretNames[name] === true
                              }
                              disabled={secretByName(name)}
                              onCheckedChange={(checked: boolean) =>
                                setSecretNames((previous) => ({
                                  ...previous,
                                  [name]: checked,
                                }))
                              }
                            />
                            <FieldLabel htmlFor={`agent-env-secret-${index}`}>
                              Secret: {name}
                            </FieldLabel>
                          </Field>
                        ))
                      })()}
                      <FieldDescription>
                        One NAME=value per line. The agent gets these and a few
                        essentials such as PATH and HOME, nothing else of
                        Oxyn&apos;s environment. Secret values are stored in the
                        system keychain and injected only at launch. Other
                        values are stored in clear in Oxyn&apos;s local SQLite
                        state. Token-like names always use the keychain. Mark
                        any other sensitive variable as secret before saving.
                      </FieldDescription>
                    </Field>
                  )}
                </form.Field>
              </>
            ) : (
              <>
                <form.Field
                  name="endpoint"
                  validators={{
                    onChange: ({ value }) =>
                      endpointCarriesCredentials(value)
                        ? { message: CREDENTIALS_IN_ENDPOINT }
                        : undefined,
                    onSubmit: ({ value }) =>
                      endpointCarriesCredentials(value)
                        ? { message: CREDENTIALS_IN_ENDPOINT }
                        : editing === null && value.trim() === ""
                          ? { message: "An endpoint is required." }
                          : undefined,
                  }}
                >
                  {(field) => (
                    <Field data-invalid={invalid(field.state.meta.errors)}>
                      <FieldLabel htmlFor="provider-endpoint">
                        Endpoint
                      </FieldLabel>
                      <TextInput
                        id="provider-endpoint"
                        className="font-mono"
                        inputMode="url"
                        placeholder={
                          KINDS.find((entry) => entry.kind === kind)
                            ?.placeholder
                        }
                        value={field.state.value}
                        aria-invalid={invalid(field.state.meta.errors)}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      {editing?.endpointRedacted ? (
                        <FieldDescription>
                          Leave empty to keep the stored endpoint (
                          {editing.endpoint}
                          …).
                        </FieldDescription>
                      ) : (
                        <FieldDescription>
                          Local or remote is decided by where this host
                          resolves, not by its name.
                        </FieldDescription>
                      )}
                      <FieldError errors={field.state.meta.errors} />
                    </Field>
                  )}
                </form.Field>
                <form.Field
                  name="model"
                  validators={{ onSubmit: required("A model") }}
                >
                  {(field) => (
                    <Field data-invalid={invalid(field.state.meta.errors)}>
                      <FieldLabel
                        id="provider-model-label"
                        htmlFor="provider-model"
                      >
                        Default model
                      </FieldLabel>
                      <ModelPicker
                        id="provider-model"
                        labelId="provider-model-label"
                        value={field.state.value}
                        onChange={(model) => field.handleChange(model)}
                        onBlur={field.handleBlur}
                        models={draftModels.models}
                        providerLabel={kindLabel(kind)}
                        invalid={invalid(field.state.meta.errors)}
                        canRefresh={draftModels.canAsk}
                        onRefresh={draftModels.refresh}
                      />
                      <FieldError errors={field.state.meta.errors} />
                    </Field>
                  )}
                </form.Field>
                <form.Field name="key">
                  {(field) => (
                    <Field>
                      <FieldLabel htmlFor="provider-key">API key</FieldLabel>
                      <TextInput
                        id="provider-key"
                        type="password"
                        autoComplete="off"
                        value={field.state.value}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldDescription>
                        {editing?.keyConfigured
                          ? "A key is stored in the system keychain. Leave empty to keep it — unless you change the endpoint: the key is then removed, type it again."
                          : "Stored in the system keychain, never shown again. A local endpoint needs none."}
                      </FieldDescription>
                    </Field>
                  )}
                </form.Field>
                {editing?.keyConfigured ? (
                  <form.Field name="clearKey">
                    {(field) => (
                      <Field orientation="horizontal">
                        <Switch
                          id="provider-clear-key"
                          checked={field.state.value}
                          onCheckedChange={(checked: boolean) =>
                            field.handleChange(checked)
                          }
                        />
                        <FieldLabel htmlFor="provider-clear-key">
                          Remove the stored key
                        </FieldLabel>
                      </Field>
                    )}
                  </form.Field>
                ) : null}
                <ConnectionTestRow
                  test={draftModels.test}
                  disabled={!draftModels.canAsk}
                  onTest={draftModels.testConnection}
                />
              </>
            )
          }
        </form.Subscribe>
      </FieldGroup>

      {failure ? (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>{FAILURE_TITLES[failure.operation]}</AlertTitle>
          <AlertDescription data-selectable>
            <span className="font-mono text-xs">{failure.message}</span>
            {failure.keyMustBeRetyped ? (
              <span className="block">
                The key was not kept: type it again.
              </span>
            ) : null}
            {failure.retryable ? (
              <span className="block">You can try again.</span>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      <div className="flex justify-end gap-2">
        {target ? (
          <Button type="button" variant="outline" onClick={onDone}>
            Cancel
          </Button>
        ) : null}
        <Button type="submit" disabled={working}>
          {working ? <Spinner data-icon="inline-start" /> : null}
          {target?.kind === "provider"
            ? "Save changes"
            : target?.kind === "agent"
              ? "Replace"
              : "Declare"}
        </Button>
      </div>
    </form>
  )
}
