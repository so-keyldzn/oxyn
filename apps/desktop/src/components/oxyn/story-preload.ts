// Loading a story file needs done before its first story is timed.
//
// What a component imports on demand — mermaid and its diagram chunks, shiki
// and a grammar — is loaded and compiled again in each story file's frame.
// A story file declares that work here, when it is imported; the Vitest setup
// file (`.storybook/vitest.setup.ts`) runs it in a `beforeAll`, once per file,
// outside every story's time budget. The Storybook workshop never runs it:
// nothing is timed there, and the components load on demand as in the app.

const pending: Array<() => Promise<unknown>> = []

/** Runs `load` once before the stories of the file that calls this. */
export function preloadBeforeStories(load: () => Promise<unknown>) {
  pending.push(load)
}

/** What the story files imported so far have declared, run and forgotten. */
export function runStoryPreloads() {
  return Promise.all(pending.splice(0).map((load) => load()))
}
