# QW1: expanded tool rows show commands as text, never bare numbers

Repo: `skchaudr/grok-build`, branch off `main`. `P` = `crates/codegen/xai-grok-pager/src`.
Audit: `docs/traces/ISSUES-ux-audit.md` issues 2 and 3.

## Problem

Expanding a tool row in `/traces` shows the tool input as pretty-printed JSON, so an
inline script (`python3 -c "…"`) renders with literal `\n` escapes instead of newlines,
and bookkeeping objects leak bare numbers (`usage.input` renders as a line reading
`3005`).

Root cause, `P/views/traces_modal.rs` `collect_text` (~line 2863): the third match arm
`_ if INPUT_KEYS.contains(&key)` pretty-prints the whole `input`/`arguments` object.
On the recursion path, `usage`/`cost` objects are descended into, and their numeric
`input`/`output` fields hit the same arm and are pretty-printed as bare numbers.

## Change (two small edits)

1. `P/views/traces_body.rs`: extend `TEXT_KEYS` with `"command"`, `"cmd"`, `"script"`.
2. `P/views/traces_modal.rs` `collect_text`: replace the third arm
   (`_ if INPUT_KEYS.contains(&key) => pretty-print x`) with `collect_text(x, out)`
   (recurse instead of pretty-printing). Scalar values under input keys that have no
   text meaning (numbers, booleans, `file_path`, `description`) then contribute
   nothing; a call whose input yields no text still falls back to `pretty(raw)` via
   `body_text`, so Edit/Read calls render as today.

Do not touch anything else in `collect_text` (dedupe via `push_chunk`, the
`content`/`message` arms, array recursion).

## Tests (in `traces_modal.rs` tests module)

- Update `body_text_reads_each_harness_shape` (~5079) and
  `body_text_dedupes_and_falls_back_to_pretty` (~5097) to the new behavior.
- New test: a Claude `tool_use` Bash record whose `input.command` is a 5-line
  `python3 -c` script yields body text containing the script with real `\n` newlines
  and no `\\n` escape sequences and no `{`/`}` JSON braces.
- New test: a record carrying `usage: {input: 3005, output: 0}` and no other text
  yields a body with no line equal to `3005` or `0` (and falls back to pretty JSON
  only when there is genuinely no text anywhere).

## Constraints

- Verify: `RUST_MIN_STACK=33554432 GROK_HOME= cargo test -p xai-grok-pager --lib traces_`
- Synthetic fixtures only; never commit real session text.
- Diff budget: under ~5k chars excluding tests. No refactors outside these two functions.
- End every commit with trailer `Co-authored-by: jules <model>`.
- Open a PR against `main` when green.
