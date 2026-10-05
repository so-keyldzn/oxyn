import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { RefreshIcon } from "@hugeicons/core-free-icons"

import { modelEntries, reasonCopy } from "@/components/oxyn/model-picker-model"
import type {
  DraftModels,
  ModelEntry,
} from "@/components/oxyn/model-picker-model"
import { TEXT_FIELD_ATTRIBUTES } from "@/components/oxyn/text-field"
import { Button } from "@/components/ui/button"
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from "@/components/ui/combobox"
import { Spinner } from "@/components/ui/spinner"

export interface ModelPickerProps {
  /** Ties the input to its `FieldLabel`. */
  id: string
  /**
   * The `FieldLabel`'s own id. The open list hides the rest of the page from
   * assistive technology, the label included: `htmlFor` then names nothing,
   * while `aria-labelledby` still reads a hidden element.
   */
  labelId: string
  /** The model id in the form; `""` while none is chosen. */
  value: string
  onChange: (model: string) => void
  onBlur?: () => void
  models: DraftModels
  /** Searched with the ids and names: « openai » finds every OpenAI model. */
  providerLabel: string
  invalid?: boolean
  /** Whether « Refresh models » can ask: an endpoint is known. */
  canRefresh: boolean
  onRefresh: () => void
}

function ModelsStatus({
  models,
  value,
}: {
  models: DraftModels
  value: string
}) {
  switch (models.status) {
    case "idle":
      return "Fill in the endpoint and the API key to list the models, or type a model id."
    case "loading":
      return (
        <span className="inline-flex items-center gap-1.5">
          <Spinner />
          Listing models…
        </span>
      )
    case "ok": {
      if (models.models.length === 0)
        return "The provider lists no model: type a model id."
      const listed = models.models.some((model) => model.id === value)
      return (
        <>
          {models.models.length === 1
            ? "1 model available."
            : `${models.models.length} models available.`}
          {value !== "" && !listed ? (
            <span className="block">
              <span className="font-mono">{value}</span> is not listed by the
              provider: it stays selected until you pick another.
            </span>
          ) : null}
        </>
      )
    }
    case "failed":
      return (
        <>
          {reasonCopy(models.reason)}{" "}
          <span data-selectable className="font-mono">
            {models.message}
          </span>
          <span className="block">You can still type a model id.</span>
        </>
      )
    case "error":
      return (
        <>
          The models could not be listed.{" "}
          <span data-selectable className="font-mono">
            {models.message}
          </span>
        </>
      )
  }
}

function emptyText(models: DraftModels) {
  if (models.status === "loading") return "Listing models…"
  if (models.status === "ok" && models.models.length > 0)
    return "No model matches."
  return "Type a model id."
}

function EntryContent({ entry }: { entry: ModelEntry }) {
  if (entry.origin === "manual")
    return (
      <span className="min-w-0 truncate">
        Use “<span className="font-mono">{entry.id}</span>” as model id
      </span>
    )
  return (
    <span className="flex min-w-0 flex-col">
      <span
        className={entry.label === entry.id ? "truncate font-mono" : "truncate"}
      >
        {entry.label}
      </span>
      {entry.label !== entry.id ? (
        <span className="truncate font-mono text-xs text-muted-foreground">
          {entry.id}
        </span>
      ) : null}
      {entry.unavailable ? (
        <span className="text-xs text-muted-foreground">
          Current model — unavailable from provider
        </span>
      ) : null}
    </span>
  )
}

/**
 * The default model of a provider: chosen in the provider's own list, or typed.
 *
 * Typing never waits on the list: a custom gateway, a private model or an
 * Azure deployment is declared with the id the user knows. The value is the
 * provider's model id; a model the provider stopped listing stays selected,
 * flagged, rather than replaced in silence.
 */
export function ModelPicker({
  id,
  labelId,
  value,
  onChange,
  onBlur,
  models,
  providerLabel,
  invalid,
  canRefresh,
  onRefresh,
}: ModelPickerProps) {
  const listed = models.status === "ok" ? models.models : null
  // What the user is typing; `null` shows the selection's label.
  const [input, setInput] = React.useState<string | null>(null)
  // The text of the selection is not a search: opening shows every model.
  const all = React.useMemo(
    () => modelEntries(listed, value, "", providerLabel),
    [listed, value, providerLabel]
  )
  // Kept by identity: a new object each render reads to the combobox as a new
  // selection, and it puts its label back over what is being typed.
  const selected = React.useMemo(
    () => all.find((entry) => entry.id === value) ?? null,
    [all, value]
  )
  const query = input === null || input === selected?.label ? "" : input
  const entries =
    query === "" ? all : modelEntries(listed, value, query, providerLabel)

  // Leaving the field with an id typed but not chosen keeps that id: saving
  // the previous model instead would go against what the user just wrote. An
  // emptied field keeps the value — clearing it is never silent.
  const commitTyped = () => {
    const typed = query.trim()
    if (typed !== "" && typed !== value) onChange(typed)
  }

  return (
    <div className="flex flex-col gap-2">
      <Combobox<ModelEntry>
        items={entries}
        filteredItems={entries}
        filter={null}
        value={selected}
        autoHighlight
        itemToStringLabel={(entry) => entry.label}
        itemToStringValue={(entry) => entry.id}
        isItemEqualToValue={(item, chosen) => item.id === chosen.id}
        inputValue={input ?? selected?.label ?? ""}
        onInputValueChange={setInput}
        onValueChange={(entry) => {
          if (entry === null) return
          setInput(null)
          onChange(entry.id)
        }}
      >
        <ComboboxInput
          id={id}
          aria-labelledby={labelId}
          className="w-full"
          // Its trigger would be a second, unnamed button for the same list:
          // the input opens it on click, on typing and on ArrowDown.
          showTrigger={false}
          placeholder="Search or type a model id"
          aria-invalid={invalid}
          onBlur={() => {
            commitTyped()
            setInput(null)
            onBlur?.()
          }}
          {...TEXT_FIELD_ATTRIBUTES}
        />
        <ComboboxContent>
          <ComboboxEmpty>{emptyText(models)}</ComboboxEmpty>
          <ComboboxList aria-label="Models">
            {(entry: ModelEntry) => (
              <ComboboxItem key={entry.id} value={entry}>
                <EntryContent entry={entry} />
              </ComboboxItem>
            )}
          </ComboboxList>
        </ComboboxContent>
      </Combobox>
      <div className="flex items-start justify-between gap-2">
        <p
          role="status"
          aria-live="polite"
          className="text-sm text-muted-foreground"
        >
          <ModelsStatus models={models} value={value} />
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!canRefresh || models.status === "loading"}
          onClick={onRefresh}
        >
          <HugeiconsIcon
            icon={RefreshIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Refresh models
        </Button>
      </div>
    </div>
  )
}
