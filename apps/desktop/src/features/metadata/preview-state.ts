import { createPreviewStore } from "@/features/metadata/preview-store"
import { backend, newCommandId } from "@/lib/ipc/client"
import { forget, watch } from "@/lib/ipc/events"
import { metadata } from "@/lib/ipc/metadata"
import { results } from "@/lib/ipc/results"

/** The previews of this window, kept across the object view's mounts. */
export const previews = createPreviewStore({
  read: (id, entry, shape) =>
    metadata.previewRelation(
      id,
      entry.connection,
      entry.session,
      entry.address,
      shape
    ),
  release: async (entry, result) => {
    const outcome = await metadata.releasePreview(
      newCommandId(),
      entry.connection,
      entry.session,
      result
    )
    if (outcome.type === "denied") throw new Error(outcome.reason)
    if (outcome.type === "needsApproval") {
      await backend.decide(outcome.command, false)
      throw new Error("The preview could not be released")
    }
  },
  cancel: (id) => void backend.cancel(id).catch(() => undefined),
  refuse: (command) =>
    void backend.decide(command, false).catch(() => undefined),
  forgetResult: (result) =>
    void results.forgetResult(result).catch(() => undefined),
  watch: (id) => watch(id),
  unwatch: (id) => forget(id),
  newId: () => newCommandId(),
  now: () => Date.now(),
})
