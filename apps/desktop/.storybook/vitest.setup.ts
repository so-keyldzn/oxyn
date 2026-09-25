// Cold work paid once per story file, before any story is timed.
//
// Each story file runs in a fresh frame, and each story is one test with a
// 15 s budget. What a frame loads on first use — axe-core for the
// accessibility check, the app shell's first render, and whatever a component
// imports on demand — otherwise falls on the file's first story. On a loaded
// machine that cold start alone passed the budget, while the same story ran in
// under a second by itself: the story failed on the machine's speed, not on
// the component's behaviour. Here, in a `beforeAll`, it is part of no story's
// budget.

import { beforeAll } from "vitest"
import { composeStory } from "storybook/preview-api"

import { runStoryPreloads } from "@/components/oxyn/story-preload"

// The hook only loads and parses — mermaid and its diagram chunks, shiki and
// its grammars, axe-core — with no state to wait for, so its bound is sized
// for a machine under heavy load rather than Vitest's 30 s default for hooks.
const COLD_START_BUDGET_MS = 60_000

beforeAll(async () => {
  // One blank story, run through the project's decorators and `afterEach`:
  // it renders the app shell once and runs axe once, so both are loaded and
  // initialised in this frame. The next story's run unmounts it.
  await composeStory({ render: () => null }, { title: "Warm up" }).run()
  await runStoryPreloads()
}, COLD_START_BUDGET_MS)
