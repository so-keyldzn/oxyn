import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, within } from "storybook/test"

import { InputGroup } from "@/components/ui/input-group"
import {
  InputGroupTextArea,
  InputGroupTextInput,
  TextArea,
  TextInput,
} from "./text-field"

const meta = {
  title: "Oxyn/TextField",
  component: TextInput,
} satisfies Meta<typeof TextInput>

export default meta
type Story = StoryObj<typeof meta>

function Fields() {
  return (
    <div className="flex w-80 flex-col gap-3">
      <TextInput aria-label="Host" defaultValue="db.internal" />
      <TextArea aria-label="Notes" defaultValue="it's a replica" />
      <InputGroup>
        <InputGroupTextInput aria-label="Filter tables" />
      </InputGroup>
      <InputGroup>
        <InputGroupTextArea aria-label="Question" />
      </InputGroup>
      <TextInput
        aria-label="Password"
        type="password"
        autoComplete="new-password"
      />
    </div>
  )
}

export const NoCorrectionInAnyField: Story = {
  render: () => <Fields />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const name of ["Host", "Notes", "Filter tables", "Question"]) {
      const field = canvas.getByLabelText(name)
      // macOS would otherwise turn ' into ’ in a connection string or a
      // SQL literal, and correct a table name it does not know.
      await expect(field).toHaveAttribute("spellcheck", "false")
      await expect(field).toHaveAttribute("autocorrect", "off")
      await expect(field).toHaveAttribute("autocapitalize", "off")
      await expect(field).toHaveAttribute("autocomplete", "off")
    }
  },
}

export const ASecretFieldKeepsItsOwnFilling: Story = {
  render: () => <Fields />,
  play: async ({ canvasElement }) => {
    const password = within(canvasElement).getByLabelText("Password")
    await expect(password).toHaveAttribute("autocomplete", "new-password")
    await expect(password).toHaveAttribute("spellcheck", "false")
  },
}
