import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import {
  AMBIGUOUS,
  ObjectOperationReviewDialog,
} from "./object-operation-review"
import { fetched, incomingKeys } from "./metadata-fixtures"
import type { ObjectOperationReview } from "@/lib/ipc/object-operations"

const postgresProduction: ObjectOperationReview = {
  sql: 'DROP TABLE "public"."invoices"',
  connectionName: "billing",
  environment: "production",
  objectName: "invoices",
  relationKind: "table",
  transactionalDdl: true,
  restrictDependents: true,
  dependents: { type: "incomingKeys", freshness: fetched, value: incomingKeys },
}

const sqliteLocal: ObjectOperationReview = {
  sql: 'DROP TABLE "main"."scratch"',
  connectionName: "notes.sqlite",
  environment: "local",
  objectName: "scratch",
  relationKind: "table",
  transactionalDdl: true,
  restrictDependents: false,
  dependents: { type: "incomingKeys", freshness: fetched, value: [] },
}

const meta = {
  title: "Oxyn/ObjectOperationReview",
  component: ObjectOperationReviewDialog,
  args: {
    open: true,
    operation: "drop",
    review: postgresProduction,
    problem: null,
    cascade: false,
    onCascadeChange: fn(),
    dependents: { kind: "read" },
    phase: { kind: "idle" },
    onRun: fn(),
    onStop: fn(),
    onCancel: fn(),
    onRefreshCatalog: fn(),
    onOpenInConsole: fn(),
  },
} satisfies Meta<typeof ObjectOperationReviewDialog>

export default meta
type Story = StoryObj<typeof meta>

/** The dialog animates in: its contents are read once it is shown. */
async function opened() {
  const dialog = within(document.body)
  const box = await dialog.findByRole("alertdialog")
  await waitFor(() => expect(box).toBeVisible())
  return dialog
}

/**
 * On production the action waits for the object's name, typed exactly:
 * `Cancel` has the focus, Enter in the field runs nothing, and a near miss
 * — the wrong case — keeps the action off.
 */
export const DropOnProduction: Story = {
  play: async ({ args }) => {
    const dialog = await opened()
    const cancel = dialog.getByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await expect(dialog.getByText("PRODUCTION")).toBeVisible()
    await expect(dialog.getByLabelText("Statement to run")).toHaveTextContent(
      'DROP TABLE "public"."invoices"'
    )
    await expect(
      dialog.getByText(
        "Without CASCADE, the server refuses if other objects depend on it."
      )
    ).toBeVisible()
    await expect(
      dialog.getByText(
        "Views, routines and triggers that use this object are not listed."
      )
    ).toBeVisible()

    const action = dialog.getByRole("button", { name: "Drop table" })
    await expect(action).toBeDisabled()
    const field = dialog.getByLabelText("Type the table name to confirm")
    await userEvent.type(field, "Invoices{Enter}")
    await expect(action).toBeDisabled()
    await expect(args.onRun).not.toHaveBeenCalled()

    await userEvent.clear(field)
    await userEvent.type(field, "invoices{Enter}")
    await expect(action).toBeEnabled()
    await expect(args.onRun).not.toHaveBeenCalled()
    await userEvent.click(action)
    await expect(args.onRun).toHaveBeenCalledOnce()
  },
}

/** Off production nothing is typed, and the gate's approval follows. */
export const DropOnLocalSqlite: Story = {
  args: { review: sqliteLocal },
  play: async ({ args }) => {
    const dialog = await opened()
    await expect(dialog.queryByLabelText(/Type the/)).toBeNull()
    // SQLite restricts nothing: no CASCADE box, and the box says so.
    await expect(dialog.queryByRole("checkbox")).toBeNull()
    await expect(
      dialog.getByText(
        "This database drops the object even if other objects still use it."
      )
    ).toBeVisible()
    await expect(
      dialog.getByText("No foreign key references it.")
    ).toBeVisible()
    await userEvent.click(dialog.getByRole("button", { name: "Drop table" }))
    await expect(args.onRun).toHaveBeenCalledOnce()
  },
}

/** Ticking CASCADE asks for the name in every environment. */
export const CascadeTicked: Story = {
  args: {
    review: {
      ...postgresProduction,
      environment: "development",
      sql: 'DROP TABLE "public"."invoices" CASCADE',
    },
    cascade: true,
  },
  play: async () => {
    const dialog = await opened()
    await expect(dialog.getByRole("checkbox")).toBeChecked()
    await expect(
      dialog.getByRole("button", { name: "Drop table" })
    ).toBeDisabled()
    await expect(
      dialog.getByLabelText("Type the table name to confirm")
    ).toBeVisible()
  },
}

/** The action waits for the read of the keys that reference the table. */
export const ReadingDependents: Story = {
  args: {
    review: {
      ...sqliteLocal,
      dependents: {
        type: "incomingKeys",
        freshness: { state: "never" },
        value: null,
      },
    },
    dependents: { kind: "reading" },
  },
  play: async () => {
    const dialog = await opened()
    await expect(
      dialog.getByRole("button", { name: "Drop table" })
    ).toBeDisabled()
  },
}

/** A failed read is said, never shown as « none ». */
export const DependentsUnreadable: Story = {
  args: {
    review: {
      ...sqliteLocal,
      dependents: {
        type: "incomingKeys",
        freshness: { state: "never" },
        value: null,
      },
    },
    dependents: {
      kind: "failed",
      message: "permission denied for pg_constraint",
    },
  },
  play: async () => {
    const dialog = await opened()
    await expect(
      dialog.getByText(/could not be read: permission denied/)
    ).toBeVisible()
    await expect(dialog.queryByText("No foreign key references it.")).toBeNull()
  },
}

/** Redshift: TRUNCATE exists, but commits and restricts nothing. */
export const TruncateWithoutTransactionalDdl: Story = {
  args: {
    operation: "truncate",
    review: {
      ...postgresProduction,
      sql: 'TRUNCATE TABLE "public"."events"',
      connectionName: "warehouse",
      environment: "staging",
      objectName: "events",
      transactionalDdl: false,
      restrictDependents: false,
      dependents: { type: "notReported" },
    },
  },
  play: async () => {
    const dialog = await opened()
    await expect(
      dialog.getByText("Dependents are not reported by this connection.")
    ).toBeVisible()
    await expect(
      dialog.getByText(/does not run DDL in a transaction/)
    ).toBeVisible()
  },
}

export const RenameTable: Story = {
  args: {
    operation: "rename",
    newName: "Orders",
    onNewNameChange: fn(),
    review: {
      ...sqliteLocal,
      sql: 'ALTER TABLE "main"."scratch" RENAME TO "Orders"',
      dependents: { type: "notApplicable" },
    },
  },
  play: async ({ args }) => {
    const dialog = await opened()
    // What is shown is the backend's quoting: `Orders` reads `"Orders"`.
    await expect(dialog.getByLabelText("Statement to run")).toHaveTextContent(
      'RENAME TO "Orders"'
    )
    await userEvent.type(dialog.getByLabelText("New name"), "x")
    await expect(args.onNewNameChange).toHaveBeenCalled()
  },
}

export const RenameProblem: Story = {
  args: {
    operation: "rename",
    newName: "",
    onNewNameChange: fn(),
    review: null,
    problem: "Type the new name.",
  },
}

export const Running: Story = {
  args: { review: sqliteLocal, phase: { kind: "running" } },
  play: async ({ args }) => {
    const dialog = await opened()
    await userEvent.click(dialog.getByRole("button", { name: /Stop/ }))
    await expect(args.onStop).toHaveBeenCalledOnce()
  },
}

/** Sent, outcome unknown: no way to run again, a way to read the catalog. */
export const Ambiguous: Story = {
  args: {
    review: sqliteLocal,
    phase: { kind: "ambiguous", message: "the statement timed out after 30 s" },
  },
  play: async ({ args }) => {
    const dialog = await opened()
    await expect(dialog.getByText(AMBIGUOUS)).toBeVisible()
    await expect(
      dialog.queryByRole("button", { name: "Drop table" })
    ).toBeNull()
    await userEvent.click(
      dialog.getByRole("button", { name: "Refresh catalog" })
    )
    await expect(args.onRefreshCatalog).toHaveBeenCalledOnce()
  },
}

export const RefusedByTheServer: Story = {
  args: {
    review: { ...postgresProduction, environment: "development" },
    phase: {
      kind: "failed",
      message:
        "cannot drop table invoices because other objects depend on it (SQLSTATE 2BP01)",
    },
  },
  play: async ({ args }) => {
    const dialog = await opened()
    await userEvent.click(
      await dialog.findByRole("button", { name: "Open in console" })
    )
    await expect(args.onOpenInConsole).toHaveBeenCalledOnce()
  },
}

export const NotSent: Story = {
  args: {
    review: sqliteLocal,
    phase: { kind: "notSent", message: "Stopped before anything was sent." },
  },
  play: async () => {
    const dialog = await opened()
    await expect(
      await dialog.findByRole("button", { name: "Drop table" })
    ).toBeEnabled()
  },
}
