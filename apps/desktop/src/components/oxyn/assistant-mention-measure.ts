/**
 * Measures, for the stories, how the mention chips sit in their text.
 *
 * `toBeVisible` says nothing of a chip taller than its line or riding above
 * it (.claude/rules/tests.md): these are boxes, read from the layout.
 */

const CHIP = "[data-slot=assistant-mention]"

function middle(rect: DOMRect) {
  return rect.top + rect.height / 2
}

/** The rects of the text around the chips, one per line fragment. */
function textRects(block: HTMLElement): Array<DOMRect> {
  const rects: Array<DOMRect> = []
  const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.parentElement?.closest(CHIP)) continue
    if ((node.textContent ?? "").trim() === "") continue
    const range = document.createRange()
    range.selectNodeContents(node)
    for (const rect of range.getClientRects())
      if (rect.width > 0) rects.push(rect)
  }
  return rects
}

export interface ChipAlignment {
  chips: number
  /** Per chip, how far its centre is from the text beside it on its line. */
  offsets: Array<number>
  /** Chips that share no line with any text: nothing to compare against. */
  alone: number
  /** The block's height with its chips, and with them flattened away. */
  withChips: number
  withoutChips: number
}

/**
 * Where each chip in `block` sits against its neighbouring text, and what the
 * chips cost the block's height.
 *
 * The height without chips is read from a clone where each chip is swapped for
 * an empty inline-block of the same width: the same words wrap at the same
 * places, and only the chips' own height is taken out of the lines.
 */
export async function measureChips(block: HTMLElement): Promise<ChipAlignment> {
  await document.fonts.ready
  const chips = Array.from(block.querySelectorAll<HTMLElement>(CHIP))
  const texts = textRects(block)
  const offsets: Array<number> = []
  let alone = 0
  for (const chip of chips) {
    const box = chip.getBoundingClientRect()
    const sameLine = texts.filter(
      (rect) => rect.top < box.bottom && rect.bottom > box.top
    )
    const nearest = sameLine.sort(
      (a, b) =>
        Math.min(Math.abs(a.right - box.left), Math.abs(a.left - box.right)) -
        Math.min(Math.abs(b.right - box.left), Math.abs(b.left - box.right))
    )[0]
    if (nearest === undefined) {
      alone += 1
      continue
    }
    offsets.push(Math.abs(middle(box) - middle(nearest)))
  }

  const clone = block.cloneNode(true) as HTMLElement
  clone.style.position = "absolute"
  clone.style.visibility = "hidden"
  clone.style.width = `${block.getBoundingClientRect().width}px`
  clone.querySelectorAll<HTMLElement>(CHIP).forEach((chip, index) => {
    const original = chips[index] ?? chip
    const flat = document.createElement("span")
    flat.style.display = "inline-block"
    flat.style.height = "0"
    flat.style.width = `${original.getBoundingClientRect().width}px`
    flat.style.marginInline = getComputedStyle(original).marginInline
    chip.replaceWith(flat)
  })
  block.parentElement?.append(clone)
  const withoutChips = clone.getBoundingClientRect().height
  clone.remove()

  return {
    chips: chips.length,
    offsets,
    alone,
    withChips: block.getBoundingClientRect().height,
    withoutChips,
  }
}
