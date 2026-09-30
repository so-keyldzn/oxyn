import { describe, expect, it } from "vitest"
import { identifierQuoteForDriver } from "@/lib/sql-identifiers"

import {
  ERD_MAX_NAMES,
  isErdBlock,
  isSqlBlock,
  parseErdNames,
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

describe("an erd block", () => {
  it("is drawn only once closed and labelled erd", () => {
    const blocks = parseMarkdown("```erd\norders\n```\n\n```ERD\ncustomers")
    expect(blocks.map(isErdBlock)).toEqual([true, false])
  })

  it("names tables one per line, qualified or not, quotes honoured", () => {
    const { names, dropped } = parseErdNames(
      [
        "-- the order tables",
        "- public.orders;",
        "customers,",
        '"Sales"."Q1.totals"',
        'shop."say ""hi"""',
        "public.orders",
        "",
        "prod.public.items",
      ].join("\n")
    )
    expect(dropped).toBe(0)
    expect(
      names.map(({ namespace, relation }) => [namespace, relation])
    ).toEqual([
      ["public", "orders"],
      [null, "customers"],
      ["Sales", "Q1.totals"],
      ["shop", 'say "hi"'],
      ["public", "items"],
    ])
  })

  it.each(["postgres", "sqlite"])("keeps %s identifier quoting", (driver) => {
    expect(
      parseErdNames(
        '"Sales"."Q1.totals"\nshop."say ""hi"""',
        identifierQuoteForDriver(driver)
      ).names
    ).toEqual([
      {
        namespace: "Sales",
        relation: "Q1.totals",
        written: '"Sales"."Q1.totals"',
      },
      { namespace: "shop", relation: 'say "hi"', written: 'shop."say ""hi"""' },
    ])
  })

  it("reads MySQL quoted qualifiers, spaces, dots and doubled backticks", () => {
    const source = [
      "`archive`.`order lines`",
      "`Sales`.`Q1.totals`",
      "shop.`say ``hi```",
      "shop.orders",
    ].join("\n")
    expect(
      parseErdNames(source, identifierQuoteForDriver("mysql")).names.map(
        ({ namespace, relation }) => [namespace, relation]
      )
    ).toEqual([
      ["archive", "order lines"],
      ["Sales", "Q1.totals"],
      ["shop", "say `hi`"],
      ["shop", "orders"],
    ])
  })

  it("never resolves fragments of invalid MySQL names or double-quoted strings", () => {
    expect(
      parseErdNames('`open\na..b\n`archive`.\n"orders"\n`orders`tail', "`")
        .names
    ).toEqual([])
  })

  it("drops a line that is not a name, and counts the surplus", () => {
    expect(parseErdNames('a..b\n"open\n.x').names).toEqual([])
    const many = Array.from({ length: ERD_MAX_NAMES + 3 }, (_, i) => `t${i}`)
    const { names, dropped } = parseErdNames(many.join("\n"))
    expect(names).toHaveLength(ERD_MAX_NAMES)
    expect(dropped).toBe(3)
  })
})
