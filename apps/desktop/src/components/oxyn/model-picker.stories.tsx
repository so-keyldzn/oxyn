import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ModelPicker } from "./model-picker"
import type { ModelPickerProps } from "./model-picker"
import { draftModels, listed } from "./model-picker-fixtures"
import { Field, FieldLabel } from "@/components/ui/field"

/** The picker as the form holds it: the value lives above it. */
function Picker(props: ModelPickerProps) {
  const [value, setValue] = React.useState(props.value)
  return (
    <Field>
      <FieldLabel id={props.labelId} htmlFor={props.id}>
        Default model
      </FieldLabel>
      <ModelPicker
        {...props}
        value={value}
        onChange={(model) => {
          setValue(model)
          props.onChange(model)
        }}
      />
    </Field>
  )
}

const meta = {
  title: "Oxyn/Settings/ModelPicker",
  component: ModelPicker,
  render: (args) => <Picker {...args} />,
  args: {
    id: "provider-model",
    labelId: "provider-model-label",
    value: "",
    onChange: fn(),
    models: listed(draftModels),
    providerLabel: "OpenAI",
    canRefresh: true,
    onRefresh: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ModelPicker>

export default meta
type Story = StoryObj<typeof meta>

const body = () => within(document.body)
// Found by its name list open as well: the open list hides its label.
const input = () => body().getByRole("combobox", { name: "Default model" })
const optionNames = async () =>
  (await body().findAllByRole("option")).map((option) => option.textContent)

export const Idle: Story = {
  args: { models: { status: "idle" }, canRefresh: false },
  play: async () => {
    await expect(body().getByText(/Fill in the endpoint/)).toBeVisible()
    await expect(
      body().getByRole("button", { name: "Refresh models" })
    ).toBeDisabled()
  },
}

export const Loading: Story = {
  args: { models: { status: "loading" } },
  play: async () => {
    await expect(body().getByText("Listing models…")).toBeVisible()
    await expect(
      body().getByRole("button", { name: "Refresh models" })
    ).toBeDisabled()
  },
}

export const Empty: Story = {
  args: { models: listed([]) },
  play: async () => {
    await expect(
      body().getByText("The provider lists no model: type a model id.")
    ).toBeVisible()
  },
}

/** Open, search on the id and on the name, then Escape. */
export const OpenAndSearch: Story = {
  play: async () => {
    await userEvent.click(input())
    await expect(await optionNames()).toEqual([
      "Orion 2model-orion-2",
      "Lyramodel-lyra-1",
      "Vega minimodel-vega-mini",
    ])
    await userEvent.type(input(), "lyra-1")
    await waitFor(async () =>
      expect(await optionNames()).toEqual([
        "Use “lyra-1” as model id",
        "Lyramodel-lyra-1",
      ])
    )
    await userEvent.clear(input())
    await userEvent.type(input(), "vega")
    await waitFor(async () =>
      expect(await optionNames()).toEqual([
        "Use “vega” as model id",
        "Vega minimodel-vega-mini",
      ])
    )
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(body().queryByRole("listbox")).not.toBeInTheDocument()
    )
  },
}

export const KeyboardSelect: Story = {
  play: async ({ args }) => {
    input().focus()
    await userEvent.keyboard("{ArrowDown}")
    await body().findByRole("listbox")
    await userEvent.keyboard("{ArrowDown}{Enter}")
    await waitFor(() =>
      expect(args.onChange).toHaveBeenCalledWith("model-lyra-1")
    )
    // The name is shown; the id is what the form stores.
    await waitFor(() => expect(input()).toHaveValue("Lyra"))
  },
}

/** Enter keeps what was typed, not the listed model it happens to match. */
export const EnterKeepsTheTypedId: Story = {
  play: async ({ args }) => {
    await userEvent.type(input(), "vega")
    await waitFor(async () =>
      expect(await optionNames()).toContain("Use “vega” as model id")
    )
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(args.onChange).toHaveBeenCalledWith("vega"))
    await expect(args.onChange).not.toHaveBeenCalledWith("model-vega-mini")
  },
}

/** An exact id is still the one Enter picks. */
export const EnterPicksTheExactId: Story = {
  play: async ({ args }) => {
    await userEvent.type(input(), "model-lyra-1")
    await waitFor(async () =>
      expect(await optionNames()).toEqual(["Lyramodel-lyra-1"])
    )
    await userEvent.keyboard("{Enter}")
    await waitFor(() =>
      expect(args.onChange).toHaveBeenCalledWith("model-lyra-1")
    )
    await waitFor(() => expect(input()).toHaveValue("Lyra"))
  },
}

/** Leaving the field keeps the typed id; leaving it emptied clears nothing. */
export const LeavingKeepsTheTypedId: Story = {
  args: { value: "model-orion-2" },
  play: async ({ args }) => {
    await expect(input()).toHaveValue("Orion 2")
    await userEvent.clear(input())
    await userEvent.tab()
    await expect(args.onChange).not.toHaveBeenCalled()
    await waitFor(() => expect(input()).toHaveValue("Orion 2"))
    await userEvent.clear(input())
    await userEvent.type(input(), "my-deployment")
    await userEvent.tab()
    await waitFor(() =>
      expect(args.onChange).toHaveBeenCalledWith("my-deployment")
    )
    await expect(args.onChange).toHaveBeenCalledOnce()
    await waitFor(() => expect(input()).toHaveValue("my-deployment"))
  },
}

/** A failed list never blocks: the id the user knows is accepted. */
export const ManualModelId: Story = {
  args: {
    models: {
      status: "failed",
      reason: "unsupported",
      message: "the endpoint has no model list",
    },
  },
  play: async ({ args }) => {
    await expect(
      body().getByText(/does not list its models: type the model id/)
    ).toBeVisible()
    await userEvent.type(input(), "my-azure-deployment")
    await userEvent.click(
      await body().findByRole("option", {
        name: "Use “my-azure-deployment” as model id",
      })
    )
    await expect(args.onChange).toHaveBeenCalledWith("my-azure-deployment")
    await waitFor(() => expect(input()).toHaveValue("my-azure-deployment"))
  },
}

export const Unauthorized: Story = {
  args: {
    models: {
      status: "failed",
      reason: "unauthorized",
      message: "401 Unauthorized",
    },
  },
  play: async ({ args }) => {
    await expect(
      body().getByText("The provider refused the API key.")
    ).toBeVisible()
    await userEvent.click(
      body().getByRole("button", { name: "Refresh models" })
    )
    await expect(args.onRefresh).toHaveBeenCalledOnce()
  },
}

export const Unreachable: Story = {
  args: {
    models: {
      status: "failed",
      reason: "unreachable",
      message: "connection refused",
    },
  },
  play: async () => {
    await expect(
      body().getByText("The endpoint could not be reached.")
    ).toBeVisible()
  },
}

/** A reason this build does not know still reads as a failure. */
export const UnknownReason: Story = {
  args: {
    models: {
      status: "failed",
      // @ts-expect-error -- a reason added by a later backend.
      reason: "quotaExceeded",
      message: "monthly quota exceeded",
    },
  },
  play: async () => {
    await expect(
      body().getByText("The models could not be listed.")
    ).toBeVisible()
  },
}

/** Kept, first and flagged: never replaced in silence. */
export const CurrentModelUnavailable: Story = {
  args: { value: "model-retired-1" },
  play: async ({ args }) => {
    await expect(input()).toHaveValue("model-retired-1")
    await expect(
      body().getByText(/is not listed by the provider/)
    ).toBeVisible()
    await userEvent.click(input())
    const [first] = await body().findAllByRole("option")
    await expect(first).toHaveTextContent(
      "model-retired-1Current model — unavailable from provider"
    )
    await expect(first).toHaveAttribute("aria-selected", "true")
    await userEvent.keyboard("{Escape}")
    await expect(args.onChange).not.toHaveBeenCalled()
    await waitFor(() => expect(input()).toHaveValue("model-retired-1"))
  },
}
