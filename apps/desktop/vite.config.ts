/// <reference types="vitest/config" />
import path from "node:path"
import { fileURLToPath } from "node:url"

import { storybookTest } from "@storybook/addon-vitest/vitest-plugin"
import tailwindcss from "@tailwindcss/vite"
import { devtools } from "@tanstack/devtools-vite"
import { tanstackStart } from "@tanstack/react-start/plugin/vite"
import viteReact from "@vitejs/plugin-react"
import { playwright } from "@vitest/browser-playwright"
import { defineConfig } from "vite"

const dirname = path.dirname(fileURLToPath(import.meta.url))

// Tauri serves static files only: there is no HTTP server behind the webview,
// so Start runs in SPA mode and its shell is written as `index.html`, the file
// Tauri opens (docs/adr/0029-interface-tauri-shadcn.md).
export default defineConfig({
  resolve: { tsconfigPaths: true },
  // Tauri prints its own logs in the same terminal; clearing it would hide a
  // Rust compile error behind Vite's banner.
  clearScreen: false,
  server: {
    port: 3000,
    // Tauri's `devUrl` names this port. Falling back to another one would open
    // a blank window instead of failing loudly.
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // Targets from Tauri's Vite guide (docs/RESEARCH-NOTES.md): WebView2 on
    // Windows, WebKit elsewhere. A newer target would ship syntax an older
    // webview cannot parse, and the window would stay blank without an error.
    target:
      process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  plugins: [
    devtools(),
    tailwindcss(),
    tanstackStart({
      spa: {
        enabled: true,
        prerender: { outputPath: "/index.html" },
      },
    }),
    viteReact(),
  ],
  test: {
    projects: [
      {
        // Pure logic: IPC decoding, formatting, state. No browser needed.
        extends: true,
        test: {
          name: "unit",
          environment: "jsdom",
          include: ["src/**/*.test.{ts,tsx}"],
        },
      },
      {
        // Every story is a test: it renders, its `play` runs, and axe checks it.
        extends: true,
        // Its own dependency cache: `make desktop-dev` keeps a Vite server
        // running in this directory, and two optimizers rewriting the same
        // cache made a story file fail to import once, at random.
        cacheDir: "node_modules/.vite-storybook-tests",
        plugins: [
          storybookTest({ configDir: path.join(dirname, ".storybook") }),
        ],
        test: {
          name: "storybook",
          browser: {
            enabled: true,
            headless: true,
            provider: playwright({}),
            instances: [{ browser: "chromium" }],
          },
        },
      },
    ],
  },
})
