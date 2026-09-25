import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent } from "storybook/test"

import { AboutSettings } from "./about-settings"
import type { ThirdPartyNotice } from "@/lib/third-party-licenses"

const MIT_TEXT = `MIT License

Copyright (c) Example Authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction.`

const notices: Array<ThirdPartyNotice> = [
  {
    license: "Apache-2.0",
    name: "Apache License 2.0",
    text: "Apache License\nVersion 2.0, January 2004",
    packages: [
      { ecosystem: "cargo", name: "arrow", version: "59.3.0" },
      { ecosystem: "cargo", name: "arrow-array", version: "59.3.0" },
      { ecosystem: "cargo", name: "arrow-buffer", version: "59.3.0" },
      { ecosystem: "cargo", name: "arrow-data", version: "59.3.0" },
      { ecosystem: "cargo", name: "arrow-schema", version: "59.3.0" },
      { ecosystem: "cargo", name: "tauri", version: "2.11.5" },
    ],
  },
  {
    license: "MIT",
    name: "MIT",
    text: MIT_TEXT,
    packages: [{ ecosystem: "npm", name: "react", version: "19.3.0" }],
  },
  {
    license: "OFL-1.1",
    name: "OFL-1.1",
    text: "SIL OPEN FONT LICENSE Version 1.1",
    packages: [
      {
        ecosystem: "npm",
        name: "@fontsource-variable/geist",
        version: "5.3.0",
      },
    ],
  },
]

const meta = {
  title: "Oxyn/AboutSettings",
  component: AboutSettings,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: { licenses: { status: "ready", notices } },
} satisfies Meta<typeof AboutSettings>

export default meta
type Story = StoryObj<typeof meta>

/** The GPL asks for the license and the absence of warranty to be shown. */
export const Populated: Story = {
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText("Copyright 2026 Nicolas Boromée")
    ).toBeVisible()
    await expect(canvas.getByText(/no warranty/)).toBeVisible()
    await expect(
      canvas.getByText(/8 packages, under 3 license texts/)
    ).toBeVisible()
    // Closed, a notice names its first packages and counts the rest.
    await expect(
      canvas.getByText(/arrow, arrow-array, arrow-buffer, arrow-data \+2/)
    ).toBeVisible()

    await userEvent.click(canvas.getByRole("button", { name: /^MIT/ }))
    await expect(canvas.getByText(/Permission is hereby granted/)).toBeVisible()
  },
}

export const Filtered: Story = {
  play: async ({ canvas }) => {
    const filter = canvas.getByRole("searchbox", {
      name: "Filter by package or license",
    })
    await userEvent.type(filter, "geist")
    await expect(
      canvas.getByRole("button", { name: /^OFL-1\.1/ })
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /^MIT/ })).toBeNull()

    await userEvent.clear(filter)
    await userEvent.type(filter, "nothing-like-this")
    await expect(
      canvas.getByText(/No package or license matches/)
    ).toBeVisible()
  },
}

export const Loading: Story = { args: { licenses: { status: "loading" } } }

/** A development build made without `cargo-about`. */
export const Missing: Story = {
  args: { licenses: { status: "missing" } },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/without its third-party notices/)
    ).toBeVisible()
  },
}

export const Empty: Story = {
  args: { licenses: { status: "ready", notices: [] } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/lists no third-party package/)).toBeVisible()
  },
}

export const Failure: Story = {
  args: {
    licenses: {
      status: "error",
      message: "notices.0.packages: expected array, received undefined",
    },
  },
}
