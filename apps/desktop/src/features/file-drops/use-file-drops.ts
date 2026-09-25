import * as React from "react"
import { useNavigate } from "@tanstack/react-router"

import { toast } from "@/components/ui/toast"
import { offerConnection } from "@/features/connections/connection-offer"
import { openInConsole } from "@/features/consoles/open-in-console"
import { session } from "@/features/session"
import { fileDrops } from "@/lib/ipc/file-drops"
import type { DroppedFile } from "@/lib/ipc/file-drops"

/**
 * What a dropped file becomes once Rust has classified it
 * (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 9). Pure
 * of IPC, so it is tested with its effects replaced.
 */
export function receiveDrop(
  file: DroppedFile,
  effects: {
    connected: () => boolean
    openInConsole: typeof openInConsole
    offerConnection: typeof offerConnection
    showConnectionScreen: () => void
    warn: (title: string, description: string) => void
  }
) {
  switch (file.type) {
    case "sql":
      // A console belongs to a connection: without one, the text would wait
      // in a queue and surface in whatever workspace opens next.
      if (!effects.connected()) {
        effects.warn(
          `${file.name} was not opened`,
          "Open a connection first: a console runs on one."
        )
        return
      }
      effects.openInConsole(file.text, {
        newConsole: true,
        notice: `Opened from ${file.name}. Nothing was executed.`,
      })
      return
    case "database":
      effects.offerConnection({
        driver: file.driver,
        field: file.field,
        path: file.path,
      })
      effects.showConnectionScreen()
      return
    case "refused":
      effects.warn(`${file.name} was not opened`, file.reason)
      return
  }
}

/** Listens to the files dropped on the window, once, for every screen. */
export function useFileDrops() {
  const navigate = useNavigate()
  const latest = React.useRef(navigate)
  latest.current = navigate
  React.useEffect(() => {
    void fileDrops
      .subscribe((file) =>
        receiveDrop(file, {
          connected: () => session.state.open !== null,
          openInConsole,
          offerConnection,
          showConnectionScreen: () => void latest.current({ to: "/" }),
          warn: (title, description) =>
            toast.add({ title, description, type: "warning" }),
        })
      )
      // Outside the webview (stories, tests) there is no window to drop on.
      .catch(() => undefined)
  }, [])
}
