# AGENTS.md

## Task granularity

Every dispatched task (Jules, Cursor agents, Pi lanes, subagents) is one step: roughly one file or function, a diff under ~5k characters, done in minutes.

- Independent steps run concurrently; dependent steps run in sequence.
- A spec slice (e.g. `docs/traces/SPEC-usable.md` T1, B1) is not a task. Split it into steps before dispatch.
- Before dispatching, list the steps and their expected diff size. Split any step that would run long or touch many files.
- Never send one large task that runs for hours when it can be split into concurrent slices.
