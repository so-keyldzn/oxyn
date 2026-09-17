import { describe, expect, it } from "vitest"

import {
  isSqlBlock,
  parseInline,
  parseMarkdown,
} from "./assistant-markdown-model"

describe("the assistant markdown model", () => {
  it("keeps raw HTML as text", () => {
    const blocks = parseMarkdown(
      '<script>alert(1)</script>\n<img src=x onerror="alert(2)">'
    )
    expect(blocks).toEqual([
      {
        type: "paragraph",
        content: [
          {
            type: "text",
            text: '<script>alert(1)</script> <img src=x onerror="alert(2)">',
          },
        ],
      },
    ])
  })

  it("turns an image into its alternative text and a link into label and address", () => {
    expect(parseInline("![logo](https://evil.example/p.png)")).toEqual([
      { type: "image", alt: "logo" },
    ])
    expect(parseInline("[click](javascript:alert(1))")).toMatchObject([
      { type: "link", href: "javascript:alert(1)" },
    ])
  })

  it("does not read snake_case as emphasis", () => {
    expect(parseInline("created_at and updated_at")).toEqual([
      { type: "text", text: "created_at and updated_at" },
    ])
    expect(parseInline("**bold** and *soft*")).toEqual([
      { type: "strong", children: [{ type: "text", text: "bold" }] },
      { type: "text", text: " and " },
      { type: "emphasis", children: [{ type: "text", text: "soft" }] },
    ])
  })

  it("offers closed sql and unlabelled fences only", () => {
    const blocks = parseMarkdown(
      [
        "```sql",
        "SELECT count(*) FROM clients;",
        "```",
        "```python",
        "print('no')",
        "```",
        "```",
        "SELECT 1;",
        "```",
        "```sql",
        "SELECT still_streaming",
      ].join("\n")
    )
    expect(blocks.filter(isSqlBlock).map((block) => block.text)).toEqual([
      "SELECT count(*) FROM clients;",
      "SELECT 1;",
    ])
  })

  it("parses lists, headings and pipe tables", () => {
    const blocks = parseMarkdown(
      "## Plan\n- one\n- two\n\n| table | rows |\n| --- | --- |\n| clients | 42 |"
    )
    expect(blocks.map((block) => block.type)).toEqual([
      "heading",
      "list",
      "table",
    ])
  })
})
