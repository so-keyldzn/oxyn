import * as React from "react"

import { Input } from "@/components/ui/input"
import {
  InputGroupInput,
  InputGroupTextarea,
} from "@/components/ui/input-group"
import { Textarea } from "@/components/ui/textarea"

/**
 * What every text field of Oxyn carries: no spelling correction, no automatic
 * capital, no typographic quotes (ADR-0041 § 8; UX-SPEC, « Ce qu'Oxyn ne fait
 * pas, parce que ce n'est pas un navigateur »). A table name corrected in
 * silence is a wrong name, and `’` in a connection string or a SQL literal is
 * not `'`.
 *
 * `components/ui` is generated and not edited by hand: the fields below wrap
 * it, and ESLint refuses the unwrapped ones anywhere else.
 */
export const TEXT_FIELD_ATTRIBUTES = {
  spellCheck: false,
  autoCorrect: "off",
  autoCapitalize: "off",
} as const

/** The same, as DOM attributes, for a surface React does not render —
 * CodeMirror's `contentAttributes`. */
export const TEXT_FIELD_DOM_ATTRIBUTES = {
  spellcheck: "false",
  autocorrect: "off",
  autocapitalize: "off",
  autocomplete: "off",
} as const

/** What a caller may not undo. `autoComplete` stays open: a secret field asks
 * for `new-password`, which keeps the browser's filling away. */
type Fixed = keyof typeof TEXT_FIELD_ATTRIBUTES

export function TextInput(
  props: Omit<React.ComponentProps<typeof Input>, Fixed>
) {
  return <Input autoComplete="off" {...props} {...TEXT_FIELD_ATTRIBUTES} />
}

export function TextArea(
  props: Omit<React.ComponentProps<typeof Textarea>, Fixed>
) {
  return <Textarea autoComplete="off" {...props} {...TEXT_FIELD_ATTRIBUTES} />
}

export function InputGroupTextInput(
  props: Omit<React.ComponentProps<typeof InputGroupInput>, Fixed>
) {
  return (
    <InputGroupInput autoComplete="off" {...props} {...TEXT_FIELD_ATTRIBUTES} />
  )
}

export function InputGroupTextArea(
  props: Omit<React.ComponentProps<typeof InputGroupTextarea>, Fixed>
) {
  return (
    <InputGroupTextarea
      autoComplete="off"
      {...props}
      {...TEXT_FIELD_ATTRIBUTES}
    />
  )
}
