import * as React from "react"
import { useQueryClient } from "@tanstack/react-query"
import { backend } from "@/lib/ipc/client"
import { serverProfiles } from "@/lib/ipc/server-profiles"
import type { ServerProfileSnapshot } from "@/lib/ipc/server-profiles"
import { settingsBackend } from "@/lib/ipc/settings"
import type { ConnectionDetails } from "@/lib/ipc/settings"
import type {
  ConnectResponse,
  ConnectionDraft,
  DriverChoice,
  OpenConnection,
} from "@/lib/ipc/types"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import type { PendingConnectionChange } from "@/components/oxyn/connection-change-review"
import type { WorkspaceActivations } from "./activation"
import { explorerFailure } from "./use-server-explorer"

export interface DatabaseWizard {
  key: string
  group: ServerProfileSnapshot
  seed: OpenConnection
  database: string
  driver: DriverChoice
  serverValues: Record<string, string>
  values: Record<string, string>
  candidates: Array<ConnectionDetails>
  submitting: boolean
  saved: boolean
  error: BackendFailure | null
}

/** New-child forms own no mutation cache; secrets live only in the form/call. */
export function useDatabaseOpening(
  activations: WorkspaceActivations,
  adopt: (open: OpenConnection, key: string) => void,
  report: (caught: unknown) => void
) {
  const client = useQueryClient()
  const remembered = React.useRef(new Map<string, string>())
  const [wizard, setWizardState] = React.useState<DatabaseWizard | null>(null)
  const ref = React.useRef<DatabaseWizard | null>(null)
  const setWizard = (next: DatabaseWizard | null) => {
    ref.current = next
    setWizardState(next)
  }
  const [approval, setApprovalState] = React.useState<
    (PendingConnectionChange & { key: string }) | null
  >(null)
  const operationKey = React.useRef<string | null>(null)
  const approvalRef = React.useRef(approval)
  const setApproval = (next: typeof approval) => {
    approvalRef.current = next
    setApprovalState(next)
  }
  React.useEffect(
    () => () => {
      const pending = approvalRef.current
      if (pending)
        void backend.decideConnection(pending.command, false).catch(report)
    },
    []
  )
  const [deciding, setDeciding] = React.useState(false)
  const update = (change: Partial<DatabaseWizard>) => {
    if (ref.current) setWizard({ ...ref.current, ...change })
  }
  const refresh = () =>
    Promise.all([
      client.invalidateQueries({ queryKey: ["connections"] }),
      client.invalidateQueries({ queryKey: ["server-profiles"] }),
    ])
  const response = async (
    answer: ConnectResponse | null,
    current: DatabaseWizard,
    cancelled: boolean
  ) => {
    if (answer?.type === "open")
      remembered.current.set(current.key, answer.connection)
    if (answer?.type === "saved")
      remembered.current.set(current.key, answer.connection.id)
    await refresh()
    if (answer?.type === "open") {
      if (cancelled) await activations.discard(answer)
      else adopt(answer, current.key)
      setWizard(null)
    } else if (answer?.type === "saved") {
      update({
        saved: true,
        submitting: false,
        error: { message: answer.message, retryable: answer.retryable },
      })
    } else if (answer?.type === "approval") {
      if (cancelled) {
        await backend.decideConnection(answer.command, false)
        setWizard(null)
      } else
        setApproval({
          key: current.key,
          kind: "create",
          command: answer.command,
          reason: answer.reason,
          connectionName: answer.preview?.connection ?? current.database,
          environment: "production",
        })
      update({ submitting: false })
    } else setWizard(null)
  }
  const choose = async (
    group: ServerProfileSnapshot,
    seed: OpenConnection,
    database: string,
    driver: DriverChoice,
    fresh: boolean
  ) => {
    const key = JSON.stringify([group.profile.id, database])
    activations.intent(key)
    if (ref.current?.key === key || activations.command(key)) return
    if (ref.current)
      throw new Error("Finish or cancel the current database form first.")
    const resolved: { connection: string | null } = { connection: null }
    await activations.run(key, async (_command, cancelled) => {
      const details = await settingsBackend.connectionDetails(seed.connection)
      const candidates: Array<ConnectionDetails> = []
      if (!fresh) {
        // Local configuration only, at most four reads at once, never introspection.
        const ids = [
          ...new Set([
            ...group.connections,
            ...(remembered.current.has(key)
              ? [remembered.current.get(key)!]
              : []),
          ]),
        ]
        for (let i = 0; i < ids.length; i += 4) {
          const batch = await Promise.all(
            ids
              .slice(i, i + 4)
              .map((id) => settingsBackend.connectionDetails(id))
          )
          candidates.push(
            ...batch.filter(
              (d) =>
                d.driver === "postgres" &&
                d.values.database === database &&
                Object.entries(details.values).every(
                  ([k, v]) =>
                    k === "database" || k === "user" || d.values[k] === v
                )
            )
          )
        }
      }
      if (cancelled() || !activations.foreground(key)) return
      const only = candidates.length === 1 ? candidates[0] : undefined
      if (only) {
        resolved.connection = only.id
        return
      }
      setWizard({
        key,
        group,
        seed,
        database,
        driver,
        serverValues: Object.fromEntries(
          driver.fields
            .filter((f) => !f.secret && f.key in details.values)
            .map((f) => [f.key, details.values[f.key] ?? ""])
        ),
        values: Object.fromEntries(
          driver.fields
            .filter((f) => !f.secret)
            .map((f) => [
              f.key,
              f.key === "database"
                ? database
                : f.key === "user"
                  ? (group.profile.params.user ?? details.values.user ?? "")
                  : (details.values[f.key] ?? f.default ?? ""),
            ])
        ),
        candidates,
        submitting: false,
        saved: false,
        error: null,
      })
    })
    if (resolved.connection && activations.foreground(key))
      await activations.activate(resolved.connection)
  }
  const submit = async (draft: ConnectionDraft) => {
    const current = ref.current
    if (
      !current ||
      current.submitting ||
      current.saved ||
      activations.command(current.key)
    )
      return
    if (
      draft.driver !== "postgres" ||
      draft.values.database !== current.database
    )
      throw new Error(
        "Choose the database again before changing the destination."
      )
    if (!activations.canStart()) {
      update({
        error: {
          message:
            "Workspace limit reached. Wait for pending openings or disconnect a connection.",
          retryable: false,
        },
      })
      return
    }
    update({ submitting: true, error: null })
    try {
      await activations.run(current.key, async (id, cancelled) =>
        response(
          await serverProfiles.open(
            id,
            current.group.profile.id,
            current.seed.connection,
            {
              ...draft,
              values: {
                ...current.serverValues,
                database: current.database,
                user: draft.values.user ?? "",
              },
            }
          ),
          current,
          cancelled()
        )
      )
    } catch (caught) {
      update({ submitting: false, error: explorerFailure(caught) })
    } finally {
      if (ref.current?.key === current.key) update({ submitting: false })
    }
  }
  const decide = async (approved: boolean) => {
    const review = approval
    const current = ref.current
    if (!review || !current || deciding) return
    setDeciding(true)
    const answered = { value: false }
    try {
      await activations.run(
        current.key,
        async (_id, cancelled) => {
          const answer = await backend.decideConnection(
            review.command,
            approved
          )
          answered.value = true
          await response(answer, current, cancelled())
        },
        review.command
      )
      if (!answered.value) {
        await backend.decideConnection(review.command, false)
        setWizard(null)
      }
      setApproval(null)
    } catch (caught) {
      try {
        await backend.decideConnection(review.command, false)
        setApproval(null)
        setWizard(null)
      } catch {
        /* Keep the withdrawal visible and retryable until acknowledged. */
      }
      report(caught)
    } finally {
      setDeciding(false)
    }
  }
  const close = () => {
    if (ref.current?.submitting) return
    setWizard(null)
  }
  const cancel = async () => {
    if (ref.current)
      await activations.cancel(operationKey.current ?? ref.current.key)
  }
  return {
    wizard,
    approval,
    deciding,
    choose,
    submit,
    decide,
    close,
    cancel,
    fresh: () => update({ candidates: [] }),
    chooseSaved: async (id: string) => {
      if (ref.current?.submitting) return
      operationKey.current = `connection:${id}`
      update({ submitting: true })
      try {
        await activations.activate(id)
        setWizard(null)
      } catch (caught) {
        update({ submitting: false, error: explorerFailure(caught) })
      } finally {
        operationKey.current = null
      }
    },
  }
}
