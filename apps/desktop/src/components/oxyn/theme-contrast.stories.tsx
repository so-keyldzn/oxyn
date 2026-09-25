import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor, within } from "storybook/test"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Switch } from "@/components/ui/switch"
import { TextInput } from "./text-field"

/**
 * The theme's control contrast, measured in the browser (WCAG 1.4.11, 3:1).
 *
 * axe checks the contrast of text, never of what identifies a control: the
 * edge of an unticked checkbox or of a field, the track or thumb of an
 * unticked switch, and the focus ring. These are
 * theme tokens — `--input` and `--ring` in `styles.css` — so a defect there is
 * in every screen at once, and is guarded here once. Both themes are measured:
 * Storybook renders in dark by default, and the light theme is where the
 * Figma values fell to 1.45:1 (edges) and 2.31:1 (focus ring).
 */

const SURFACES = [
  { name: "background", className: "bg-background" },
  { name: "card", className: "bg-card" },
  { name: "muted", className: "bg-muted" },
] as const

/** An opaque sRGB colour, as `color` looks painted over `over`. */
function paint(color: string, over: string): [number, number, number] {
  const canvas = document.createElement("canvas")
  canvas.width = 1
  canvas.height = 1
  const context = canvas.getContext("2d", { willReadFrequently: true })
  if (!context) throw new Error("No 2D context")
  context.fillStyle = over
  context.fillRect(0, 0, 1, 1)
  // A colour the canvas cannot read would leave the previous one in place and
  // measure 1:1 in silence: it fails loudly instead.
  context.fillStyle = "#010203"
  context.fillStyle = color
  if (context.fillStyle === "#010203" && color !== "#010203")
    throw new Error(`Unreadable colour: ${color}`)
  context.fillRect(0, 0, 1, 1)
  const [red = 0, green = 0, blue = 0] = context.getImageData(0, 0, 1, 1).data
  return [red, green, blue]
}

function luminance(rgb: [number, number, number]) {
  const [r, g, b] = rgb.map((channel) => {
    const value = channel / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  }) as [number, number, number]
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function ratio(
  first: [number, number, number],
  second: [number, number, number]
) {
  const [high, low] = [luminance(first), luminance(second)].sort(
    (a, b) => b - a
  ) as [number, number]
  return (high + 0.05) / (low + 0.05)
}

function contrast(color: string, surface: string) {
  return ratio(paint(color, surface), paint(surface, "#000000"))
}

/**
 * A layer painted on another layer painted on the surface, against the
 * surface: the thumb of a switch sits on its track, not on the page.
 */
function layeredContrast(top: string, under: string, surface: string) {
  const [r, g, b] = paint(under, surface)
  return ratio(paint(top, `rgb(${r}, ${g}, ${b})`), paint(surface, "#000000"))
}

/**
 * The colour of the widest shadow in a computed `box-shadow`: Tailwind draws a
 * ring as one spread shadow among several empty ones.
 */
function ringColor(boxShadow: string) {
  const shadows: Array<string> = []
  let depth = 0
  let start = 0
  for (let index = 0; index < boxShadow.length; index++) {
    const char = boxShadow[index]
    if (char === "(") depth++
    else if (char === ")") depth--
    else if (char === "," && depth === 0) {
      shadows.push(boxShadow.slice(start, index))
      start = index + 1
    }
  }
  shadows.push(boxShadow.slice(start))
  let best: { color: string; spread: number } | null = null
  for (const shadow of shadows) {
    const trimmed = shadow.trim()
    const color = /^([a-z-]+\([^)]*\)|#[0-9a-f]+|[a-z]+)/i.exec(trimmed)?.[1]
    const lengths = trimmed.match(/-?[\d.]+px/g) ?? []
    const spread = Number.parseFloat(lengths[3] ?? "0")
    if (color && spread > (best?.spread ?? 0)) best = { color, spread }
  }
  if (!best) throw new Error(`No ring in: ${boxShadow}`)
  return best.color
}

function Surfaces() {
  return (
    <div className="flex flex-col">
      {SURFACES.map((surface) => (
        <section
          key={surface.name}
          aria-label={`On ${surface.name}`}
          data-surface={surface.name}
          className={`flex items-center gap-4 p-4 ${surface.className}`}
        >
          <Checkbox aria-label={`Checkbox on ${surface.name}`} />
          <TextInput
            aria-label={`Field on ${surface.name}`}
            className="w-48"
            defaultValue=""
          />
          <Switch aria-label={`Switch on ${surface.name}`} />
          <Button variant="outline">Button on {surface.name}</Button>
        </section>
      ))}
    </div>
  )
}

async function focusByKeyboard(target: HTMLElement) {
  // `:focus-visible` follows the keyboard: a programmatic focus may not draw
  // the ring the user actually sees.
  for (let step = 0; step < 20 && document.activeElement !== target; step++)
    await userEvent.tab()
  await expect(target).toHaveFocus()
}

async function measureTheme(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)
  for (const surface of SURFACES) {
    const section = canvasElement.querySelector<HTMLElement>(
      `[data-surface="${surface.name}"]`
    )
    if (!section) throw new Error(`No ${surface.name} surface`)
    const ground = getComputedStyle(section).backgroundColor

    const box = canvas.getByRole("checkbox", {
      name: `Checkbox on ${surface.name}`,
    })
    await expect(
      contrast(getComputedStyle(box).borderTopColor, ground),
      `unticked checkbox edge on ${surface.name}`
    ).toBeGreaterThanOrEqual(3)

    const field = canvas.getByRole("textbox", {
      name: `Field on ${surface.name}`,
    })
    await expect(
      contrast(getComputedStyle(field).borderTopColor, ground),
      `field edge on ${surface.name}`
    ).toBeGreaterThanOrEqual(3)

    // An unticked switch is identified by its track **or** its thumb. In
    // dark the track is `bg-input/80` in ui/ and falls just under 3:1 on its
    // own; the light thumb carries it. Measured, both reported, and held to
    // 3:1 by whichever stands out — not corrected while the thumb carries it.
    const toggle = canvas.getByRole("switch", {
      name: `Switch on ${surface.name}`,
    })
    const thumb = toggle.querySelector<HTMLElement>(
      '[data-slot="switch-thumb"]'
    )
    if (!thumb) throw new Error("No switch thumb")
    const track = getComputedStyle(toggle).backgroundColor
    const trackRatio = contrast(track, ground)
    const thumbRatio = layeredContrast(
      getComputedStyle(thumb).backgroundColor,
      track,
      ground
    )
    await expect(
      Math.max(trackRatio, thumbRatio),
      `unticked switch on ${surface.name}: track ${trackRatio.toFixed(2)}, thumb ${thumbRatio.toFixed(2)}`
    ).toBeGreaterThanOrEqual(3)

    const button = canvas.getByRole("button", {
      name: `Button on ${surface.name}`,
    })
    await focusByKeyboard(button)
    await waitFor(() =>
      expect(
        contrast(ringColor(getComputedStyle(button).boxShadow), ground),
        `focus ring on ${surface.name}`
      ).toBeGreaterThanOrEqual(3)
    )
  }
}

const meta = {
  title: "Oxyn/Theme/ControlContrast",
  parameters: { layout: "fullscreen" },
  render: () => <Surfaces />,
} satisfies Meta

export default meta
type Story = StoryObj<typeof meta>

export const Light: Story = {
  globals: { theme: "light" },
  play: async ({ canvasElement }) => measureTheme(canvasElement),
}

export const Dark: Story = {
  globals: { theme: "dark" },
  play: async ({ canvasElement }) => measureTheme(canvasElement),
}
