import { Button } from "@/components/ui/button"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Spinner } from "@/components/ui/spinner"
import type { SessionPlace } from "@/lib/ipc/consoles"

const SERVER_DEFAULT = "Server default"

/** The five states of the selector (docs/UX-SPEC.md, « États d'une vue »). */
export type SessionContextState =
  /** Nothing declared: Oxyn does not claim to know the schema. */
  | { status: "serverDefault" }
  | { status: "changing"; target: string; current: SessionPlace | null }
  /** What the session reports, never what was asked. */
  | { status: "declared"; place: SessionPlace }
  | {
      status: "failed"
      message: string
      retryable: boolean
      current: SessionPlace | null
    }

function label(place: SessionPlace | null) {
  return place?.namespace ?? SERVER_DEFAULT
}

function key(place: SessionPlace | null) {
  return JSON.stringify([place?.catalog ?? null, place?.namespace ?? null])
}

function confirmed(state: SessionContextState): SessionPlace | null {
  switch (state.status) {
    case "serverDefault":
      return null
    case "declared":
      return state.place
    default:
      return state.current
  }
}

/**
 * Where this console's session resolves unqualified names (ADR-0019).
 *
 * Never optimistic: choosing sends the change and the field keeps showing the
 * confirmed place until the session answers. The server default is always
 * offered. Rendered only for a session declaring `SESSION_CONTEXT`.
 */
export function SessionContextPicker({
  connectionName,
  choices,
  state,
  onChoose,
  onCancel,
  onLoadSchemas,
}: {
  connectionName: string
  /** From the catalog cache; the first is the server default. */
  choices: Array<SessionPlace>
  state: SessionContextState
  onChoose: (place: SessionPlace) => void
  onCancel: () => void
  onLoadSchemas: () => void
}) {
  const current = confirmed(state)
  // The place the session reports is offered even if the cache lacks it.
  const options = choices.some((choice) => key(choice) === key(current))
    ? choices
    : [...choices, ...(current ? [current] : [])]
  const place = `${connectionName} / ${label(current)}`
  const noSchema =
    choices.filter((choice) => choice.namespace !== null).length === 0

  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex items-center gap-1.5 text-xs">
        <bdi
          title={connectionName}
          className="min-w-0 truncate text-muted-foreground"
        >
          {connectionName}
        </bdi>
        <span aria-hidden className="text-muted-foreground">
          /
        </span>
        <NativeSelect
          size="sm"
          aria-label="Session schema"
          value={key(current)}
          disabled={state.status === "changing"}
          onChange={(event) => {
            const chosen = options.find(
              (option) => key(option) === event.target.value
            )
            if (chosen && key(chosen) !== key(current)) onChoose(chosen)
          }}
          className="w-44"
        >
          {options.map((option) => (
            <NativeSelectOption key={key(option)} value={key(option)}>
              {label(option)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        {state.status === "changing" ? (
          <Button size="xs" variant="ghost" onClick={onCancel}>
            <Spinner data-icon="inline-start" />
            Cancel context change
          </Button>
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground" role="status">
        {state.status === "serverDefault" && noSchema
          ? "The catalog lists no schema for this connection yet. Load it to choose one; until then the server default applies."
          : state.status === "serverDefault"
            ? "This session resolves unqualified names where the server placed it when it opened. Oxyn has not asked it to move, and does not claim to know the schema."
            : state.status === "changing"
              ? `Switching to ${state.target}… Until the server answers, this session still resolves in ${place}. Cancel stops the change at the server.`
              : null}
        {state.status === "failed" ? (
          <span className="text-warning">
            {state.message} Nothing moved: this session still resolves in{" "}
            {place}.{" "}
            {state.retryable
              ? "Choosing again may succeed."
              : "Choosing again gets the same answer until the cause is fixed."}
          </span>
        ) : null}
      </p>
      {state.status === "serverDefault" && noSchema ? (
        <div>
          <Button size="xs" variant="outline" onClick={onLoadSchemas}>
            Load schemas
          </Button>
        </div>
      ) : null}
    </div>
  )
}
