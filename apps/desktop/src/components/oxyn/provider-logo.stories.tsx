import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { ProviderLogo } from "./provider-logo"

const meta = {
  title: "Oxyn/ProviderLogo",
  component: ProviderLogo,
  args: { brand: "claude-code", className: "size-8" },
} satisfies Meta<typeof ProviderLogo>

export default meta
type Story = StoryObj<typeof meta>

function logo(canvasElement: HTMLElement) {
  const svg = canvasElement.querySelector("svg")
  if (svg === null) throw new Error("no logo drawn")
  return svg
}

/** Decorative and drawn: the brand's name is always written next to it. */
async function drawn(canvasElement: HTMLElement) {
  const svg = logo(canvasElement)
  await expect(svg).toHaveAttribute("aria-hidden", "true")
  await expect(svg).toHaveClass("size-8")
  await expect(svg).toBeVisible()
  return svg
}

/** The monochrome marks take the foreground token, so they flip with the theme. */
async function followsTheTheme(canvasElement: HTMLElement) {
  const svg = await drawn(canvasElement)
  const foreground = getComputedStyle(canvasElement).color
  for (const path of svg.querySelectorAll("path"))
    await expect(getComputedStyle(path).fill).toBe(foreground)
}

function story(
  brand: string,
  theme: "light" | "dark",
  check: (canvasElement: HTMLElement) => Promise<unknown> = drawn
): Story {
  return {
    args: { brand },
    globals: { theme },
    play: async ({ canvasElement }) => {
      await check(canvasElement)
    },
  }
}

export const ClaudeCode = story("claude-code", "light")
export const ClaudeCodeDark = story("claude-code", "dark")
export const Codex = story("codex", "light", followsTheTheme)
export const CodexDark = story("codex", "dark", followsTheTheme)
export const Anthropic = story("anthropic", "light", followsTheTheme)
export const AnthropicDark = story("anthropic", "dark", followsTheTheme)
export const OpenAI = story("openai", "light", followsTheTheme)
export const OpenAIDark = story("openai", "dark", followsTheTheme)
export const Gemini = story("gemini", "light")
export const GeminiDark = story("gemini", "dark")

/** No svgl mark for a generic gateway: the generic icon, never a drawn one. */
export const OpenAICompatible = story("openai_compatible", "light")
export const OpenAICompatibleDark = story("openai_compatible", "dark")

export const AgentDeclaredByHand: Story = {
  args: { brand: "custom-agent", fallback: "agent" },
  play: async ({ canvasElement }) => {
    await drawn(canvasElement)
  },
}
