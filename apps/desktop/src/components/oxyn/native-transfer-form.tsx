import * as React from "react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from "@/components/ui/card"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
  FieldSet,
  FieldLegend,
} from "@/components/ui/field"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from "@/components/ui/select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import type { NativeTransferViewState } from "./native-transfer-model"
import type { NativeOperation, NativeFormat } from "@/lib/ipc/native-transfers"

/** Structured input only; format availability comes from the original source. */
export function NativeTransferForm({
  state,
  onOperation,
  onFormat,
  onTool,
  onFile,
  onOptions,
  onTrust,
  onPrepare,
}: {
  state: NativeTransferViewState
  onOperation: (operation: NativeOperation) => void
  onFormat: (format: NativeFormat) => void
  onTool: () => void
  onFile: () => void
  onOptions: (options: { noOwner: boolean; noPrivileges: boolean }) => void
  onTrust: (trusted: boolean) => void
  onPrepare: () => void
}) {
  const id = React.useId()
  const support = state.support
  if (!support) return null
  const busy = state.busy !== null
  const locked = busy || state.review !== null
  const allowed =
    state.operation === "export" ? support.export : support.restore
  const formats =
    state.operation === "export"
      ? support.exportFormats
      : support.restoreFormats
  const labels = {
    customArchive: "Custom native archive",
    sql: "Readable SQL export",
  }
  const canPrepare =
    allowed &&
    state.tool &&
    state.file &&
    (state.operation === "export" || state.trustedArchive)
  return (
    <Card>
      <CardHeader>
        <CardTitle>Prepare a database transfer</CardTitle>
        <CardDescription>
          Engine: {support.engine}. Files and executables are selected by native
          dialogs; free-form commands and flags are not accepted.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <FieldGroup>
          <Field>
            <FieldLabel>Operation</FieldLabel>
            <ToggleGroup
              value={[state.operation]}
              disabled={locked}
              onValueChange={(values) => {
                const value = values[0]
                if (value === "export" || value === "restore")
                  onOperation(value)
              }}
              aria-label="Native operation"
            >
              <ToggleGroupItem value="export" disabled={!support.export}>
                Export database
              </ToggleGroupItem>
              <ToggleGroupItem value="restore" disabled={!support.restore}>
                Restore archive
              </ToggleGroupItem>
            </ToggleGroup>
            <FieldDescription>
              The native tool opens its own reviewed database connection.
              Uncommitted console changes and private temporary objects are not
              a database backup.
            </FieldDescription>
          </Field>
          <Field data-disabled={locked || !allowed}>
            <FieldLabel id={`${id}-format`}>Native format</FieldLabel>
            <Select
              items={formats.map((format) => ({
                value: format,
                label: labels[format],
              }))}
              value={allowed ? state.format : null}
              disabled={locked || !allowed}
              onValueChange={(value) => {
                if (value === "customArchive" || value === "sql")
                  onFormat(value)
              }}
            >
              <SelectTrigger aria-labelledby={`${id}-format`}>
                <SelectValue placeholder="No available format" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {formats.map((format) => (
                    <SelectItem key={format} value={format}>
                      {labels[format]}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel>Trusted installed tool</FieldLabel>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                disabled={locked || !allowed}
                onClick={onTool}
              >
                Select {state.operation === "export" ? "export" : "restore"}{" "}
                tool…
              </Button>
              {state.tool ? (
                <Badge variant="secondary">
                  {state.tool.identity.name} {state.tool.identity.version}
                </Badge>
              ) : (
                <span className="text-sm text-muted-foreground">
                  No tool selected
                </span>
              )}
            </div>
            <FieldDescription>
              Selection runs a bounded version check on a private executable
              snapshot. Unsupported and outdated clients are refused.
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel>
              {state.operation === "export"
                ? "New destination file"
                : "Trusted input archive"}
            </FieldLabel>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                disabled={locked || !allowed}
                onClick={onFile}
              >
                {state.operation === "export"
                  ? "Choose new output file…"
                  : "Choose archive…"}
              </Button>
              {state.file ? (
                <bdi className="text-sm">{state.file.name}</bdi>
              ) : (
                <span className="text-sm text-muted-foreground">
                  No file selected
                </span>
              )}
            </div>
            <FieldDescription>
              {state.operation === "export"
                ? "A partial file is never published as a completed export. Existing files and symlinks are refused."
                : "Archive format compatibility does not certify its contents or effects. SQL scripts are not an automatic fallback."}
            </FieldDescription>
          </Field>
          {state.operation === "restore" && (
            <FieldSet>
              <FieldLegend>Restore options</FieldLegend>
              <Field orientation="horizontal">
                <Checkbox
                  id={`${id}-owner`}
                  checked={state.noOwner}
                  disabled={locked}
                  onCheckedChange={(value) =>
                    onOptions({
                      noOwner: value === true,
                      noPrivileges: state.noPrivileges,
                    })
                  }
                />
                <FieldLabel htmlFor={`${id}-owner`}>
                  Omit source ownership
                </FieldLabel>
              </Field>
              <Field orientation="horizontal">
                <Checkbox
                  id={`${id}-privileges`}
                  checked={state.noPrivileges}
                  disabled={locked}
                  onCheckedChange={(value) =>
                    onOptions({
                      noOwner: state.noOwner,
                      noPrivileges: value === true,
                    })
                  }
                />
                <FieldLabel htmlFor={`${id}-privileges`}>
                  Omit source privileges
                </FieldLabel>
              </Field>
              <Field orientation="horizontal">
                <Checkbox
                  id={`${id}-trusted`}
                  checked={state.trustedArchive}
                  disabled={locked}
                  onCheckedChange={(value) => onTrust(value === true)}
                />
                <FieldLabel htmlFor={`${id}-trusted`}>
                  I trust this archive and understand that its code runs with
                  the destination account’s privileges
                </FieldLabel>
              </Field>
            </FieldSet>
          )}
        </FieldGroup>
      </CardContent>
      <CardFooter className="flex flex-wrap gap-2">
        <Button disabled={locked || !canPrepare} onClick={onPrepare}>
          Prepare transfer
        </Button>
        <p className="text-xs text-muted-foreground">
          Local passwordless mode only: saved passwords are not passed. TLS,
          SSH, remote authentication and SQL-script restore remain unavailable.
        </p>
      </CardFooter>
    </Card>
  )
}
