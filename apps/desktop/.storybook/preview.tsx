import * as React from "react"
import type { Decorator, Preview } from "@storybook/react-vite"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"

import { Toaster } from "@/components/ui/toast"
import { TooltipProvider } from "@/components/ui/tooltip"

import "../src/styles.css"

// Stories render inside the same providers and theme as the application, so a
// component that looks right here looks right in the window. The theme class
// goes on <html>, like the application: portalled dialogs and menus render
// outside the story's root and must still pick it up.
const withAppShell: Decorator = (Story, context) => {
  const dark = context.globals.theme !== "light"
  React.useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", dark)
  }, [dark])
  const [queryClient] = React.useState(
    () => new QueryClient({ defaultOptions: { queries: { retry: false } } })
  )
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <Toaster>
          <div className="bg-background font-sans text-foreground antialiased">
            <Story />
          </div>
        </Toaster>
      </TooltipProvider>
    </QueryClientProvider>
  )
}

const preview: Preview = {
  decorators: [withAppShell],
  globalTypes: {
    theme: {
      description: "Application theme",
      toolbar: {
        title: "Theme",
        icon: "mirror",
        items: ["dark", "light"],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { theme: "dark" },
  parameters: {
    layout: "fullscreen",
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    a11y: {
      // Keyboard reach and visible focus are blocking (.claude/rules): a
      // violation fails `make qualite`, it is not a note in a side panel.
      test: "error",
    },
  },
}

export default preview
