import { QueryClient } from "@tanstack/react-query"

/** The webview talks to local IPC; only Rust knows whether a server is reachable. */
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        networkMode: "always",
        // An ambiguous write must never be replayed (I-13). Backend failures
        // stay visible until the user or a domain event asks for another read.
        retry: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
      mutations: { networkMode: "always", retry: false },
    },
  })
}
