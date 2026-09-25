// The one way Oxyn writes the clipboard (docs/RESEARCH-NOTES.md,
// « Presse-papiers de la webview »).

/**
 * Said when the engine refuses a copy whose text had to be read first: the
 * system allows a copy only while the click that asked for it is recent.
 */
export const LATE_COPY_REFUSED =
  "The system refused the clipboard: the text arrived after the click that asked for it. Copy again."

/**
 * Writes `text` to the clipboard — or the text a promise will resolve to.
 *
 * Call it **synchronously** from the click, before any `await`: WebKit lets
 * a page write only during the user's gesture, and an IPC round trip loses
 * it (« The request is not allowed by the user agent… »). A text still to be
 * read — composed by the backend, read from a held result — goes through a
 * `ClipboardItem` built on the spot and resolved later, the one form WebKit
 * accepts after the gesture (docs/RESEARCH-NOTES.md, « Presse-papiers de la
 * webview »). The promise resolves to a `Blob`, which every engine of the
 * target takes; a string there is younger.
 *
 * Rejects with the text's own error when reading the text failed: a copy
 * that could not compose its SQL says why, not that the clipboard refused.
 */
export async function writeClipboard(
  text: string | Promise<string>
): Promise<void> {
  if (typeof text === "string") return navigator.clipboard.writeText(text)
  if (
    typeof ClipboardItem === "undefined" ||
    typeof navigator.clipboard.write !== "function"
  ) {
    // No deferred item on this engine: the gesture is gone by the time the
    // text arrives, and a refusal is said as such rather than as a riddle.
    const resolved = await text
    try {
      await navigator.clipboard.writeText(resolved)
    } catch (error) {
      if (error instanceof DOMException && error.name === "NotAllowedError")
        throw new Error(LATE_COPY_REFUSED, { cause: error })
      throw error
    }
    return
  }
  const blob = text.then((value) => new Blob([value], { type: "text/plain" }))
  // An engine that refuses before reading the item leaves its failure
  // unobserved; the text's error is thrown below instead.
  blob.catch(() => undefined)
  try {
    await navigator.clipboard.write([new ClipboardItem({ "text/plain": blob })])
  } catch (error) {
    // The engine reports a failed item as a refusal: the text's own error
    // comes first when there is one.
    await text
    throw error
  }
}
