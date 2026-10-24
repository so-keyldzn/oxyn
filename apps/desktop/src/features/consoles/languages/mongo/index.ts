import { json, jsonLanguage } from "@codemirror/lang-json"
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language"
import type { Extension } from "@codemirror/state"
import { tags } from "@lezer/highlight"

import { mongoCompletionSource } from "./completion"
import type { EditorModeContext } from "../index"

// The editor's own style colours strings, numbers and literals; a command's
// structure is its keys, so they take the colour SQL gives its keywords.
const keys = syntaxHighlighting(
  HighlightStyle.define([
    { tag: tags.propertyName, color: "var(--primary-text)" },
  ])
)

/**
 * The MongoDB console: one database command as Extended JSON (ADR-0063),
 * highlighted by the JSON grammar and completed from the MongoDB vocabulary
 * and the catalog cache.
 */
export function mongoMode(context: EditorModeContext): Extension {
  return [
    json(),
    keys,
    jsonLanguage.data.of({
      autocomplete: mongoCompletionSource(context.catalog),
    }),
  ]
}
