import * as React from "react"
import { $applyNodeReplacement, DecoratorNode } from "lexical"
import type {
  LexicalNode,
  NodeKey,
  SerializedLexicalNode,
  Spread,
} from "lexical"

import { MentionChip } from "./assistant-mention-chip"
import type { MentionChoice, MentionKind } from "./assistant-mention-chip"
import type { Mention } from "@/lib/ipc/ai"

export type SerializedMentionNode = Spread<
  { kind: MentionKind; label: string; mention: Mention },
  SerializedLexicalNode
>

/**
 * A mention in the question: inline, removed whole by Backspace, never
 * edited letter by letter — the label and the address must not drift apart.
 */
export class MentionNode extends DecoratorNode<React.JSX.Element> {
  __kind: MentionKind
  __label: string
  __mention: Mention

  static getType() {
    return "oxyn-mention"
  }

  static clone(node: MentionNode) {
    return new MentionNode(
      node.__kind,
      node.__label,
      node.__mention,
      node.__key
    )
  }

  static importJSON(serialized: SerializedMentionNode) {
    return $createMentionNode({
      kind: serialized.kind,
      label: serialized.label,
      mention: serialized.mention,
    })
  }

  constructor(
    kind: MentionKind,
    label: string,
    mention: Mention,
    key?: NodeKey
  ) {
    super(key)
    this.__kind = kind
    this.__label = label
    this.__mention = mention
  }

  exportJSON(): SerializedMentionNode {
    return {
      ...super.exportJSON(),
      kind: this.__kind,
      label: this.__label,
      mention: this.__mention,
    }
  }

  createDOM() {
    const element = document.createElement("span")
    element.dataset.mention = ""
    return element
  }

  updateDOM() {
    return false
  }

  isInline() {
    return true
  }

  // Arrows step over the chip instead of selecting it: a plain-text field has
  // nothing to do with a selected node but delete it by surprise.
  isKeyboardSelectable() {
    return false
  }

  getTextContent() {
    return `@${this.__label}`
  }

  getMention() {
    return this.getLatest().__mention
  }

  decorate() {
    return <MentionChip kind={this.__kind} label={this.__label} />
  }
}

export function $createMentionNode(
  choice: Pick<MentionChoice, "kind" | "label" | "mention">
) {
  return $applyNodeReplacement(
    new MentionNode(choice.kind, choice.label, choice.mention)
  )
}

export function $isMentionNode(
  node: LexicalNode | null | undefined
): node is MentionNode {
  return node instanceof MentionNode
}
