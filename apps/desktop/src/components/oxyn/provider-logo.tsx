import type * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { CommandLineIcon, ServerStack01Icon } from "@hugeicons/core-free-icons"

import { AnthropicBlack } from "@/components/ui/svgs/anthropicBlack"
import { ClaudeAiIcon } from "@/components/ui/svgs/claudeAiIcon"
import { Gemini } from "@/components/ui/svgs/gemini"
import { Openai } from "@/components/ui/svgs/openai"
import { cn } from "@/lib/utils"

type Logo = (props: React.SVGProps<SVGSVGElement>) => React.ReactElement

/**
 * svgl ships OpenAI's and Anthropic's marks in a fixed black (and a white
 * twin): the foreground token overrides the fill, so one mark follows both
 * themes and the generated files stay untouched for `shadcn add`.
 */
function OpenaiMark({ className, ...props }: React.SVGProps<SVGSVGElement>) {
  return (
    <Openai {...props} className={cn(className, "[&_path]:fill-foreground")} />
  )
}

function AnthropicMark({ className, ...props }: React.SVGProps<SVGSVGElement>) {
  return (
    <AnthropicBlack {...props} className={cn(className, "fill-foreground")} />
  )
}

/**
 * The official marks, from the svgl registry (`shadcn add @svgl/<name>`),
 * keyed by agent preset id or provider family. Claude Code is Claude; Codex
 * signs in with a ChatGPT account and wears OpenAI's mark.
 */
const logos: Partial<Record<string, Logo>> = {
  "claude-code": ClaudeAiIcon,
  codex: OpenaiMark,
  anthropic: AnthropicMark,
  openai: OpenaiMark,
  gemini: Gemini,
}

/**
 * The brand's official logo, decorative: its name is always written next to
 * it. Without a known mark — an OpenAI-compatible gateway, an agent declared
 * by hand — the generic icon of what it is, never a drawn one.
 */
export function ProviderLogo({
  brand,
  fallback = "endpoint",
  className,
  "data-icon": dataIcon,
}: {
  /** An agent preset id (`claude-code`) or a provider family (`anthropic`). */
  brand: string
  fallback?: "endpoint" | "agent"
  className?: string
  /** Its place in a `Button`, which spaces an icon by this attribute. */
  "data-icon"?: "inline-start" | "inline-end"
}) {
  const Mark = logos[brand]
  if (Mark)
    return <Mark aria-hidden data-icon={dataIcon} className={className} />
  return (
    <HugeiconsIcon
      icon={fallback === "agent" ? CommandLineIcon : ServerStack01Icon}
      strokeWidth={2}
      aria-hidden
      data-icon={dataIcon}
      className={className}
    />
  )
}
