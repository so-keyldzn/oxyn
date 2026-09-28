---
name: piege-edit-sequence-u-decodee
description: Edit/Write decode a `\uXXXX` sequence written in the text — an `r#"… …"#` becomes a real U+2028 in the file
metadata:
  type: feedback
---

A `\u` sequence followed by four hexadecimal digits, written in `old_string`/`new_string`/`content`, arrives **decoded** in the file: `r#""a b""#` becomes a raw string containing the real U+2028 character (invisible, it looks like a space). `\n`, `\"` go through as is; only `\u` is affected. The `'\u{2028}'` forms (with braces) go through intact.

**Why:** observed while writing a U+2028 escaping test in `oxyn-ai`: the assertion expected an escaped ` ` and actually compared against the raw character. Nothing flagged it, the test failed for no visible reason.

**How to apply:** to expect a `\uXXXX` sequence in a Rust test, build it with a function (`format!("\\u{code:04x}")`) rather than writing it in a raw literal. After an edit that contains one, check with a Python script looking for `' ' in line`, not with `grep` (which sees nothing).
