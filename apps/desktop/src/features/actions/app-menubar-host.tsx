import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { AppMenubar, FOLDED_MENU } from "@/components/oxyn/app-menubar"
import { useCompact } from "@/features/workspace/use-compact"
import {
  actionSources,
  currentContext,
  focusStore,
  useActionSource,
} from "@/lib/actions/context"
import { mnemonicsShown, setMnemonicHandler } from "@/lib/actions/keyboard"
import { manifest } from "@/lib/actions/manifest"
import { entryStates, menuBar } from "@/lib/actions/menu-model"
import { invoke } from "@/lib/actions/registry"

const MENUS = menuBar(manifest, "other")

/**
 * The web menu bar of Windows and Linux, fed by the registry: its entries,
 * their states in the current context, Alt, its mnemonics and F10.
 */
export function AppMenubarHost() {
  const compact = useCompact()
  const mnemonics = useStore(mnemonicsShown)
  const sources = useStore(actionSources, (state) => state.sources)
  const focus = useStore(focusStore)
  const [openMenu, setOpenMenu] = React.useState<string | null>(null)
  const bar = React.useRef<HTMLDivElement>(null)

  const states = React.useMemo(
    () =>
      Object.fromEntries(
        entryStates(manifest, "other", currentContext()).map((entry) => [
          entry.id,
          entry,
        ])
      ),
    // `currentContext` reads the stores these two values come from.
    [sources, focus]
  )

  useActionSource(
    "menubar",
    {},
    {
      focus: () =>
        bar.current
          ?.querySelector<HTMLElement>('[data-slot="menubar-trigger"]')
          ?.focus(),
    }
  )

  React.useEffect(() => {
    setMnemonicHandler((key) => {
      const menu = MENUS.find((candidate) => candidate.mnemonic === key)
      if (!menu) return false
      setOpenMenu(compact ? FOLDED_MENU : menu.id)
      return true
    })
    return () => setMnemonicHandler(null)
  }, [compact])

  return (
    <div ref={bar} className="shrink-0">
      <AppMenubar
        menus={MENUS}
        states={states}
        compact={compact}
        mnemonics={mnemonics}
        openMenu={openMenu}
        onOpenMenuChange={setOpenMenu}
        onInvoke={(id) => void invoke(id, "menu")}
      />
    </div>
  )
}
