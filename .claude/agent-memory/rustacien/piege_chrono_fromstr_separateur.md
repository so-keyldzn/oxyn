---
name: piege-chrono-fromstr-separateur
description: What `chrono::FromStr` really accepts for NaiveDate/NaiveTime/NaiveDateTime/DateTime<Utc> (checked, not copied from memory)
metadata:
  type: feedback
---

Before writing a `///` that describes a format accepted by `chrono` (or any
external library), first write a small disposable program (scratchpad, not the
repository) that calls `FromStr` on the plausible variants and reads the real
result. `chrono` 0.4.45 has surprises that cannot be guessed:

- `NaiveDate: FromStr` accepts `YYYY-M-D` (month/day not padded), rejects any
  time component.
- `NaiveTime: FromStr` accepts `HH:MM` **without seconds** (seconds at 0), in
  addition to `HH:MM:SS[.fraction]`.
- `NaiveDateTime: FromStr` accepts **only** the `T` separator — the SQL form
  with a space (`YYYY-MM-DD HH:MM:SS`) fails with `ParseError(Invalid)`. A
  trailing time zone offset fails too (`TooLong`): that is a
  `DateTime<Utc>`, not a `NaiveDateTime`.
- `DateTime<Utc>: FromStr` accepts `T` **and** space as separator, but requires
  an explicit offset (`Z` or `+HH:MM`) — without it, `TooShort`. That comes for
  free for the invariant "a timestamp without an offset must be refused rather
  than silently interpreted as UTC": no need to validate it by hand, `FromStr`
  already does it.

**Why**: documenting a hallucinated format rather than a checked one is exactly
what I-12 forbids, and it cannot be detected at compile time — the code
compiles, the tests pass if you only test the case you imagined.

**How to apply**: when a task asks to parse user text into a
`chrono`/`uuid`/`serde_json` type, first write the tests that prove the format,
run them, *then* write the `///` from the observed result — never the other way
round. See `crates/oxyn-core/src/value.rs`, `ParameterType::parse` and the tests
`date_accepts_the_iso_calendar_form` etc. for the pattern.
