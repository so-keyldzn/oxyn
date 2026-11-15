import { backend, newCommandId } from "@/lib/ipc/client"

/** Availability/history reads have their own cancellation, separate from native jobs. */
export async function readNativeState<T>(
  signal: AbortSignal,
  read: (id: string) => Promise<T>
): Promise<T> {
  const id = newCommandId()
  const cancel = () => void backend.cancel(id).catch(() => undefined)
  signal.addEventListener("abort", cancel, { once: true })
  if (signal.aborted) cancel()
  try {
    return await read(id)
  } finally {
    signal.removeEventListener("abort", cancel)
  }
}
