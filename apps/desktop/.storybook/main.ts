import type { StorybookConfig } from "@storybook/react-vite"
import type { PluginOption } from "vite"

// Components are rendered in isolation, without the application's router.
// The TanStack Start and devtools plugins expect the application entry and
// its route tree, so they are left out of the workshop's Vite build.
function withoutAppPlugins(
  plugins: Array<PluginOption> | undefined
): Array<PluginOption> {
  const kept: Array<PluginOption> = []
  const visit = (option: PluginOption) => {
    if (Array.isArray(option)) {
      option.forEach(visit)
      return
    }
    const isApp =
      option !== null &&
      typeof option === "object" &&
      "name" in option &&
      String(option.name).toLowerCase().includes("tanstack")
    if (!isApp) kept.push(option)
  }
  ;(plugins ?? []).forEach(visit)
  return kept
}

const config: StorybookConfig = {
  stories: ["../src/**/*.mdx", "../src/**/*.stories.@(ts|tsx)"],
  addons: [
    "@storybook/addon-vitest",
    "@storybook/addon-a11y",
    "@storybook/addon-docs",
  ],
  framework: "@storybook/react-vite",
  core: {
    // Oxyn is offline by default (docs/VISION.md): a component workshop that
    // phones home on every start would contradict it on a developer machine.
    disableTelemetry: true,
  },
  viteFinal: (viteConfig) => ({
    ...viteConfig,
    plugins: withoutAppPlugins(viteConfig.plugins),
  }),
}

export default config
