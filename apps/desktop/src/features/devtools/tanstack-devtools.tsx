import { TanStackDevtools } from "@tanstack/react-devtools"
import { ReactQueryDevtoolsPanel } from "@tanstack/react-query-devtools"
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools"

// Loaded by the root only under `import.meta.env.DEV`: the router state and
// the query cache are a developer's view, never a panel of the shipped app.
export default function Devtools() {
  return (
    <TanStackDevtools
      config={{ position: "bottom-left", hideUntilHover: true }}
      plugins={[
        { name: "TanStack Router", render: <TanStackRouterDevtoolsPanel /> },
        { name: "TanStack Query", render: <ReactQueryDevtoolsPanel /> },
      ]}
    />
  )
}
