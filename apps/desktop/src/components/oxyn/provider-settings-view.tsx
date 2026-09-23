import * as React from "react"
import { useForm } from "@tanstack/react-form"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  CommandLineIcon,
  Delete02Icon,
  Key01Icon,
  PencilEdit02Icon,
  RefreshIcon,
  SparklesIcon,
} from "@hugeicons/core-free-icons"

import {
  CREDENTIALS_IN_ENDPOINT,
  KINDS,
  endpointCarriesCredentials,
  kindLabel,
  parseArguments,
  reachSummary,
  removalQuestion,
} from "@/components/oxyn/provider-settings-model"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { AgentPresets } from "@/components/oxyn/agent-presets"
import type { PresetStates } from "@/components/oxyn/agent-presets"
import type {
  AgentDraft,
  AgentPresetDraft,
  AgentPresetId,
  DeclaredProvider,
  ExternalAgent,
  ModelChoice,
  ProviderDraft,
  ProviderKind,
} from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

export type ModelsState =
  | { status: "loading" }
  | { status: "ready"; models: ReadonlyArray<ModelChoice> }
  | { status: "error"; message: string }

export interface ProviderSettingsFailure {
  message: string
  retryable: boolean
  /** A failed save forgot the key it carried: say so, rather than a blank field. */
  keyMustBeRetyped: boolean
}

export interface ProviderSettingsViewProps {
  status: "loading" | "ready" | "error"
  loadError?: string | null
  providers: ReadonlyArray<DeclaredProvider>
  agents: ReadonlyArray<ExternalAgent>
  /** The ready-made declarations for agents already on this machine. */
  presets: ReadonlyArray<AgentPresetDraft>
  presetStates: PresetStates
  declaringPreset: AgentPresetId | null
  working: null | "saving" | "removing"
  failure: ProviderSettingsFailure | null
  models: Readonly<Record<string, ModelsState>>
  onSaveProvider: (draft: ProviderDraft) => void
  onSaveAgent: (draft: AgentDraft) => void
  onDetectPreset: (id: AgentPresetId) => void
  onDeclarePreset: (draft: AgentPresetDraft) => void
  onRemoveProvider: (provider: DeclaredProvider) => void
  onRemoveAgent: (agent: ExternalAgent) => void
  onCheckModels: (provider: DeclaredProvider) => void
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
}

type Removal =
  | { kind: "provider"; provider: DeclaredProvider }
  | { kind: "agent"; agent: ExternalAgent }

function required(label: string) {
  return ({ value }: { value: string }) =>
    value.trim() === "" ? { message: `${label} is required.` } : undefined
}

function ProviderForm({
  editing,
  working,
  onSaveProvider,
  onSaveAgent,
  onDoneEditing,
}: {
  editing: DeclaredProvider | null
  working: boolean
  onSaveProvider: (draft: ProviderDraft) => void
  onSaveAgent: (draft: AgentDraft) => void
  onDoneEditing: () => void
}) {
  const form = useForm({
    defaultValues: editing
      ? {
          ...EMPTY_FORM,
          kind: editing.kind as Kind,
          label: editing.label,
          // A redacted endpoint is not prefilled: saving it would lose the part
          // that was hidden. Empty keeps the stored one.
          endpoint: editing.endpointRedacted ? "" : editing.endpoint,
          model: editing.model,
        }
      : EMPTY_FORM,
    onSubmit: ({ value, formApi }) => {
      if (value.kind === "agent") {
        onSaveAgent({
          label: value.label.trim(),
          command: value.command.trim(),
          args: parseArguments(value.args),
          env: [],
        })
      } else {
        onSaveProvider({
          id: editing?.id ?? null,
          kind: value.kind,
          label: value.label.trim(),
          baseUrl: value.endpoint.trim(),
          model: value.model.trim(),
          key: value.key.trim() === "" ? null : value.key,
          clearKey: value.clearKey,
        })
      }
      // The key leaves once and the screen forgets it at once (I-03).
      formApi.setFieldValue("key", "")
      if (editing) onDoneEditing()
      else formApi.reset()
    },
  })

  return (
    <form
      noValidate
      aria-label={editing ? `Edit ${editing.label}` : "Declare a provider"}
      className="flex flex-col gap-4 rounded-lg border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault()
        void form.handleSubmit()
      }}
    >
      <h3 className="text-sm font-medium">
        {editing ? `Edit “${editing.label}”` : "Declare a provider"}
      </h3>
      <FieldGroup>
        <form.Field name="kind">
          {(field) => (
            <Field>
              <FieldLabel htmlFor="provider-kind">Kind</FieldLabel>
              <NativeSelect
                id="provider-kind"
                value={field.state.value}
                disabled={editing !== null}
                onChange={(event) =>
                  field.handleChange(event.target.value as Kind)
                }
              >
                {KINDS.map((entry) => (
                  <NativeSelectOption key={entry.kind} value={entry.kind}>
                    {entry.label}
                  </NativeSelectOption>
                ))}
                {editing ? null : (
                  <NativeSelectOption value="agent">
                    External agent (no key)
                  </NativeSelectOption>
                )}
              </NativeSelect>
            </Field>
          )}
        </form.Field>

        <form.Field name="label" validators={{ onSubmit: required("A name") }}>
          {(field) => (
            <Field
              data-invalid={field.state.meta.errors.length > 0 || undefined}
            >
              <FieldLabel htmlFor="provider-label">Name</FieldLabel>
              <Input
                id="provider-label"
                value={field.state.value}
                maxLength={128}
                aria-invalid={field.state.meta.errors.length > 0 || undefined}
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
                    <Field
                      data-invalid={
                        field.state.meta.errors.length > 0 || undefined
                      }
                    >
                      <FieldLabel htmlFor="agent-command">Program</FieldLabel>
                      <Input
                        id="agent-command"
                        className="font-mono"
                        spellCheck={false}
                        placeholder="claude-code-acp"
                        value={field.state.value}
                        aria-invalid={
                          field.state.meta.errors.length > 0 || undefined
                        }
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
                      <Input
                        id="agent-args"
                        className="font-mono"
                        spellCheck={false}
                        value={field.state.value}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldDescription>
                        Run without a shell. Oxyn asks you to confirm the exact
                        command in a system dialog, and never lets an agent
                        serve a local-only connection: nothing says where it
                        sends data.
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
                    <Field
                      data-invalid={
                        field.state.meta.errors.length > 0 || undefined
                      }
                    >
                      <FieldLabel htmlFor="provider-endpoint">
                        Endpoint
                      </FieldLabel>
                      <Input
                        id="provider-endpoint"
                        className="font-mono"
                        spellCheck={false}
                        inputMode="url"
                        placeholder={
                          KINDS.find((entry) => entry.kind === kind)
                            ?.placeholder
                        }
                        value={field.state.value}
                        aria-invalid={
                          field.state.meta.errors.length > 0 || undefined
                        }
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
                    <Field
                      data-invalid={
                        field.state.meta.errors.length > 0 || undefined
                      }
                    >
                      <FieldLabel htmlFor="provider-model">
                        Default model
                      </FieldLabel>
                      <Input
                        id="provider-model"
                        className="font-mono"
                        spellCheck={false}
                        maxLength={128}
                        value={field.state.value}
                        aria-invalid={
                          field.state.meta.errors.length > 0 || undefined
                        }
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldError errors={field.state.meta.errors} />
                    </Field>
                  )}
                </form.Field>
                <form.Field name="key">
                  {(field) => (
                    <Field>
                      <FieldLabel htmlFor="provider-key">API key</FieldLabel>
                      <Input
                        id="provider-key"
                        type="password"
                        autoComplete="off"
                        spellCheck={false}
                        value={field.state.value}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                      />
                      <FieldDescription>
                        {editing?.keyConfigured
                          ? "A key is stored in the system keychain. Leave empty to keep it."
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
              </>
            )
          }
        </form.Subscribe>
      </FieldGroup>
      <div className="flex justify-end gap-2">
        {editing ? (
          <Button type="button" variant="outline" onClick={onDoneEditing}>
            Cancel
          </Button>
        ) : null}
        <Button type="submit" disabled={working}>
          {working ? <Spinner data-icon="inline-start" /> : null}
          {editing ? "Save changes" : "Declare"}
        </Button>
      </div>
    </form>
  )
}

/**
 * The model providers and external agents of this machine.
 *
 * Common to every workspace; the privacy tier stays on each connection
 * (ADR-0023). A key is typed once, leaves for the keychain, and is shown again
 * only as « key configured » (I-03).
 */
export function ProviderSettingsView({
  status,
  loadError = null,
  providers,
  agents,
  presets,
  presetStates,
  declaringPreset,
  working,
  failure,
  models,
  onSaveProvider,
  onSaveAgent,
  onDetectPreset,
  onDeclarePreset,
  onRemoveProvider,
  onRemoveAgent,
  onCheckModels,
}: ProviderSettingsViewProps) {
  const [editing, setEditing] = React.useState<DeclaredProvider | null>(null)
  // Kept while the dialog closes, so its title never empties mid-animation.
  const [removal, setRemovalTarget] = React.useState<Removal | null>(null)
  const [removalOpen, setRemovalOpen] = React.useState(false)
  const setRemoval = (target: Removal | null) => {
    if (target) setRemovalTarget(target)
    setRemovalOpen(target !== null)
  }
  const busy = working !== null || status !== "ready"

  return (
    <section
      aria-labelledby="ai-providers-title"
      className="flex flex-col gap-4"
    >
      <div className="flex flex-col gap-1">
        <h2 id="ai-providers-title" className="text-base font-medium">
          AI providers
        </h2>
        <p className="text-sm text-muted-foreground">
          Oxyn is a complete client without AI: the assistant appears only once
          a provider or an agent is declared. What each connection lets leave is
          set on the connection.
        </p>
      </div>

      <AgentPresets
        presets={presets}
        states={presetStates}
        declaring={declaringPreset}
        onDetect={onDetectPreset}
        onDeclare={onDeclarePreset}
      />

      {failure ? (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>
            {working === null ? "The last change was not saved" : "Failed"}
          </AlertTitle>
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

      {status === "loading" ? (
        <div
          role="status"
          className="flex flex-col gap-2"
          aria-busy="true"
          aria-label="Loading providers"
        >
          <Skeleton className="h-14 w-full" />
          <Skeleton className="h-14 w-full" />
        </div>
      ) : status === "error" ? (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>The declarations could not be read</AlertTitle>
          <AlertDescription data-selectable className="font-mono text-xs">
            {loadError}
          </AlertDescription>
        </Alert>
      ) : providers.length === 0 && agents.length === 0 ? (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={SparklesIcon} strokeWidth={2} />
            </EmptyMedia>
            <EmptyTitle>No provider declared</EmptyTitle>
            <EmptyDescription>
              Nothing is sent anywhere, and no AI entry appears in the
              workspace.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="flex flex-col gap-2">
          {providers.map((provider) => {
            const listed = models[provider.id]
            return (
              <Item key={provider.id} variant="outline" className="flex-wrap">
                <ItemMedia variant="icon">
                  <HugeiconsIcon icon={SparklesIcon} strokeWidth={2} />
                </ItemMedia>
                <ItemContent className="min-w-0">
                  <ItemTitle>
                    {provider.label}
                    <span className="text-xs font-normal text-muted-foreground">
                      {kindLabel(provider.kind)}
                    </span>
                  </ItemTitle>
                  <ItemDescription className="font-mono text-xs break-all">
                    {provider.endpoint}
                    {provider.endpointRedacted ? "…" : ""} · {provider.model}
                  </ItemDescription>
                  <div className="flex flex-wrap gap-1.5 pt-1">
                    <Badge variant="outline">
                      <HugeiconsIcon
                        icon={Key01Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      {provider.keyConfigured ? "Key configured" : "No key"}
                    </Badge>
                    <Badge
                      variant="outline"
                      className={cn(
                        provider.reach !== "local" &&
                          "border-env-staging text-env-staging"
                      )}
                    >
                      {reachSummary(provider.reach, provider.measuredAtMs)}
                    </Badge>
                  </div>
                  {listed ? (
                    <p
                      role="status"
                      className={cn(
                        "pt-1 text-xs",
                        listed.status === "error"
                          ? "font-mono text-destructive"
                          : "text-muted-foreground"
                      )}
                    >
                      {listed.status === "loading"
                        ? "Asking the endpoint…"
                        : listed.status === "error"
                          ? listed.message
                          : listed.models.length === 0
                            ? "The endpoint answered and lists no model."
                            : `The endpoint answered · ${listed.models.length} model(s): ${listed.models
                                .slice(0, 6)
                                .map((model) => model.id)
                                .join(
                                  ", "
                                )}${listed.models.length > 6 ? "…" : ""}`}
                    </p>
                  ) : null}
                </ItemContent>
                <ItemActions>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy || listed?.status === "loading"}
                    onClick={() => onCheckModels(provider)}
                  >
                    <HugeiconsIcon
                      icon={RefreshIcon}
                      strokeWidth={2}
                      data-icon="inline-start"
                    />
                    Check
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={`Edit ${provider.label}`}
                    disabled={busy}
                    onClick={() => setEditing(provider)}
                  >
                    <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={`Remove ${provider.label}`}
                    disabled={busy}
                    onClick={() => setRemoval({ kind: "provider", provider })}
                  >
                    <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                  </Button>
                </ItemActions>
              </Item>
            )
          })}
          {agents.map((agent) => (
            <Item key={agent.id} variant="outline">
              <ItemMedia variant="icon">
                <HugeiconsIcon icon={CommandLineIcon} strokeWidth={2} />
              </ItemMedia>
              <ItemContent className="min-w-0">
                <ItemTitle>
                  {agent.label}
                  <span className="text-xs font-normal text-muted-foreground">
                    External agent
                  </span>
                </ItemTitle>
                <ItemDescription className="font-mono text-xs break-all">
                  {agent.command}
                  {agent.argCount > 0 ? ` · ${agent.argCount} argument(s)` : ""}
                </ItemDescription>
              </ItemContent>
              <ItemActions>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`Remove ${agent.label}`}
                  disabled={busy}
                  onClick={() => setRemoval({ kind: "agent", agent })}
                >
                  <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                </Button>
              </ItemActions>
            </Item>
          ))}
        </div>
      )}

      <ProviderForm
        key={editing?.id ?? "new"}
        editing={editing}
        working={working === "saving"}
        onSaveProvider={onSaveProvider}
        onSaveAgent={onSaveAgent}
        onDoneEditing={() => setEditing(null)}
      />

      <AlertDialog
        open={removalOpen}
        onOpenChange={(open) => {
          if (!open) setRemoval(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {removal
                ? removalQuestion(
                    removal.kind === "provider"
                      ? removal.provider.label
                      : removal.agent.label
                  )
                : ""}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {removal?.kind === "provider"
                ? "The declaration and its stored key are removed. Documents keep the provenance they were written with."
                : "The declaration is removed. The program itself is left untouched."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (removal?.kind === "provider")
                  onRemoveProvider(removal.provider)
                if (removal?.kind === "agent") onRemoveAgent(removal.agent)
                setRemoval(null)
              }}
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
