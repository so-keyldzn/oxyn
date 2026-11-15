interface OwnedView {
  key: string
  owner?: { session: string }
}

/** A connection's catalog outlives individual consoles; only retired owners lose views. */
export function sessionViewRetirement(
  catalogSession: string,
  consoleSessions: ReadonlyArray<string>,
  views: ReadonlyArray<OwnedView>
) {
  const live = new Set([catalogSession, ...consoleSessions])
  const retired = new Set(
    views
      .filter((view) => view.owner && !live.has(view.owner.session))
      .map((view) => view.key)
  )
  return { live, retired }
}
