import { isTauri } from "@tauri-apps/api/core"

import type { Platform } from "./shortcut"

/**
 * The keyboard family this window runs on.
 *
 * Read from the user agent: WKWebView reports `Macintosh`, WebView2 and
 * WebKitGTK do not. It decides `Mod`, never a permission.
 */
export const platform: Platform =
  typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent)
    ? "mac"
    : "other"

/**
 * The native macOS menu bar exists (ADR-0041, point 4). Storybook and a plain
 * browser on a Mac have none: the dispatcher then binds every combination.
 */
export const nativeMenu = platform === "mac" && isTauri()

/**
 * The key `Mod` stands for, as `userEvent.keyboard` names it: what a story
 * presses for ⌘ on a Mac and for Ctrl on the Linux runners of the CI.
 */
export const modKey = platform === "mac" ? "Meta" : "Control"
