import type { MentionView } from "@/lib/ipc/ai"

export type QuestionSegment =
  | { kind: "text"; text: string }
  | { kind: "mention"; view: MentionView; index: number }

/** A character that would make `@orders` the start of a longer word. */
const WORD = /[\p{L}\p{N}_]/u

/**
 * Where each mention sits in the question, to draw it as a chip.
 *
 * Nothing is guessed from the text: `@` followed by a word is not a mention.
 * Only the mentions the question **carries** are placed — in the order typed,
 * each at the first `@label` after the previous one, and only where the label
 * ends on a word boundary. A mention not found — a label the catalog has
 * since renamed — is returned apart, to be shown after the text rather than
 * dropped.
 */
export function questionSegments(
  text: string,
  mentions: ReadonlyArray<MentionView>
): { segments: Array<QuestionSegment>; unplaced: Array<MentionView> } {
  const segments: Array<QuestionSegment> = []
  const unplaced: Array<MentionView> = []
  let cursor = 0
  mentions.forEach((view, index) => {
    const token = `@${view.label}`
    let at = text.indexOf(token, cursor)
    while (at >= 0 && WORD.test(text.charAt(at + token.length)))
      at = text.indexOf(token, at + 1)
    if (at < 0) {
      unplaced.push(view)
      return
    }
    if (at > cursor)
      segments.push({ kind: "text", text: text.slice(cursor, at) })
    segments.push({ kind: "mention", view, index })
    cursor = at + token.length
  })
  if (cursor < text.length)
    segments.push({ kind: "text", text: text.slice(cursor) })
  return { segments, unplaced }
}
