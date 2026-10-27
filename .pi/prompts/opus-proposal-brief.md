---
description: Plan one spec slice into reviewed step briefs in .pi/briefs/<slice>/
argument-hint: "<slice> | <verify command> | <slice text>"
---
Split the arguments below on ` | ` into `slice`, `verify`, and `sliceText`. Your first tool call is exactly one `subagent` call with `workflow: "./.pi/workflows/opus-proposal-brief.js"`, `async: false`, and `args: { slice, sliceText, verify }`. Do nothing before it.

Arguments:
$@

When it returns, report `ok` and each step's path and verdict, then stop.
