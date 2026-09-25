import { describe, expect, it } from "vitest"

import { consoleLabels, publishConsoleLabels } from "./console-labels"

describe("console labels at exit", () => {
  it("keep a hidden workspace's consoles named beside the shown one's", () => {
    // Two retained workspaces, both connected (ADR-0046): the exit dialog
    // names a transaction left in the hidden one as well.
    publishConsoleLabels([], { hidden: "orders.sql" })
    publishConsoleLabels([], { shown: "report.sql" })
    // The shown workspace republishes its own consoles only.
    publishConsoleLabels(["shown"], { shown: "report v2.sql" })
    expect(consoleLabels.state).toEqual({
      hidden: "orders.sql",
      shown: "report v2.sql",
    })
  })
})
