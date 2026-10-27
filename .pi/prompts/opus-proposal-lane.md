---
description: Run every step brief of a slice in order, committing after each pass
argument-hint: "<slice>"
---
Read `.pi/briefs/$1/index.md`. It holds the slice text, a `Verify:` line, and the ordered step briefs. Set `sliceText` to the whole index.md contents and `verify` to the `Verify:` command.

For each step in order, read its brief file, then make exactly one `subagent` call with `workflow: "./.pi/workflows/opus-proposal-lane.js"`, `async: false`, and `args: { brief: <that brief's full contents>, verify, sliceText }`. One call per step; do nothing else between steps except the commit below.

After a step returns `ok: true`, commit that step with a message ending in `Co-authored-by: pi <model you ran as>`, then run the next step. Stop at the first step that returns `ok: false` and report its step id, `stage`, and `reason` or gate output. After the last step, report each commit hash and verdict.
