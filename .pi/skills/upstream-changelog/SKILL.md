---
name: upstream-changelog
description: >-
  STEP 1: Build upstream changelog for grok-pi fork. Fetch changes from xai-org/grok-build, extract "Changes:", write to docs/upstream/UPSTREAM_CHANGELOG.md. No merge.
  Triggers:  upstream changelog.
---
# Upstream Changelog

Fill the upstream update record for the grok-pi fork. This is the **first step**
of an upstream sync: capture *what upstream changed* before any merge happens.

```text
upstream sync workflow:
  1. THIS SKILL  -> fill docs/upstream/UPSTREAM_CHANGELOG.md (the update list)
  2. (later, authorized) isolated-worktree merge of upstream
  3. (later) verification + delivery
```

## Core principle — commit messages are the source of truth

Upstream commits are titled `Synced from monorepo`, but each one carries a
**`Changes:` bullet list** in its message body plus a **`Source-Revision:`**
trailer with the monorepo SHA. These bullets ARE the complete, authoritative
list of what changed.

**Primary method: transcribe the `Changes:` lists from the upstream commit
messages.** Do not invent feature descriptions from scratch when the commit
message already states them. Lightly reword for clarity, group related bullets,
and triage each into Added / Changed / Fixed.

**Fallback only:** if a commit in the range has NO `Changes:` list (rare),
analyze the diff for that commit to derive descriptions (see Step 4b).

## Repository facts (do not re-derive)

- `upstream` remote = `https://github.com/xai-org/grok-build.git` (read-only).
- `origin` = the grok-pi fork. Never push/merge from this skill.
- The fork's last synced upstream commit == `git merge-base HEAD upstream/main`.
- `SOURCE_REV` (repo root) holds the upstream **monorepo** SHA of the last sync.
  The new value comes from the `Source-Revision:` trailer (or `git show
  <tip>:SOURCE_REV`).
- Output file: `docs/upstream/UPSTREAM_CHANGELOG.md` (English, newest on top).

## Inputs / overrides

Defaults are correct for the normal case. Accept explicit overrides only if the
user gives them (e.g. "record range 98c3b24..3af4d5d"):

- `BASE` = `git merge-base HEAD upstream/main` (last synced upstream commit).
- `HEAD` = `upstream/main` (latest upstream tip).

## Workflow

### Step 0 — Preflight

Confirm you are at the repo root and the `upstream` remote exists:

```bash
git rev-parse --show-toplevel
git remote get-url upstream
```

If `upstream` is missing, stop and tell the user — do not guess a URL.

### Step 1 — Fetch upstream (read-only, safe)

```bash
git fetch upstream main --prune
```

### Step 2 — Resolve the range

```bash
BASE=$(git merge-base HEAD upstream/main)
HEAD=$(git rev-parse upstream/main)
echo "$BASE..$HEAD"
```

- If `BASE == HEAD`: upstream is already fully synced. Report "no pending
  upstream changes" and stop (do not write an empty entry).
- Verify `BASE` is an ancestor of `HEAD`:
  `git merge-base --is-ancestor "$BASE" "$HEAD"`.

### Step 3 — Gather facts

Collect the raw material before writing anything:

```bash
# upstream commit messages — THE primary source (full body, not just subject)
git log --format='%H%n%B%n---' "$BASE..$HEAD"

# totals
git diff --shortstat "$BASE" "$HEAD"

# per-file change type (A/M/D) for triage and the Areas table
git diff --name-status "$BASE" "$HEAD"

# per-file +/- for the Areas-touched table
git diff --numstat "$BASE" "$HEAD"

# upstream monorepo SHA at the new tip, and the previously synced one
git show "$HEAD:SOURCE_REV"
cat SOURCE_REV
```

### Step 4 — Transcribe the commit `Changes:` lists (primary)

For each commit in `$BASE..$HEAD`, extract its `Changes:` bullets. Then:

1. **Deduplicate / merge** bullets that describe the same capability across
   commits (a range can span several monorepo syncs).
2. **Triage** each bullet into one of:
   - **Added** — new features, commands, APIs, crates, config knobs.
   - **Changed** — behavior changes, refactors, reworks, new defaults.
   - **Fixed** — bug fixes, security fixes, crash/error handling.
   (Security hardening that closes an exploit → **Fixed**; a new security
   capability → **Added**.)
3. **Lightly reword** for clarity and consistent tense; keep the upstream's
   meaning exactly. Do not add claims the message does not support.
4. Preserve the upstream's area prefix when useful (e.g. `Pager:`, `Shell:`,
   `Tools:`, `Security:`, `Worktree:`, `Voice:`) — it feeds the Areas table.

#### Step 4b — Diff fallback (only for commits WITHOUT a `Changes:` list)

If a commit message has no usable `Changes:` list, derive descriptions from its
diff. Group changed files into areas with `references/area-map.md`, then read
the diff of the most-changed files and describe the capability:

```bash
git diff "$BASE" "$HEAD" -- crates/codegen/<crate>
```

Mark such derived bullets `(from diff)` so the reader knows they were not
transcribed from a commit message.

### Step 5 — Build the Areas-touched table

Group the changed files by crate / area using `references/area-map.md`. For each
area, sum file count and +/- from `git diff --numstat`. Use the area prefixes
from the `Changes:` bullets to add a short Notes column (e.g. "new durable
workflow engine", "permission exec-risk model").

### Step 6 — Fill the changelog

Prepend a new entry to `docs/upstream/UPSTREAM_CHANGELOG.md`, directly under the
`<!-- entries below this line -->` marker, newest on top. Follow the schema
documented in that file's header and the template in
`assets/entry-template.md`. Use the real numbers from Step 3 and the transcribed
bullets from Step 4. Omit an empty section (e.g. drop `### Fixed` if nothing
was fixed).

### Step 7 — Report and stop

Summarize to the user: the range, diff size, top 3–5 features, and any
higher-risk seam areas (Pager `app/`, ACP, external profile, session/queue/
context). State clearly that this only **recorded** the upstream changes — the
merge itself is a separate, separately-authorized step. Do not start the merge,
push, or modify `SOURCE_REV` from this skill.

## Guardrails

- Git operations here are **read-only**: `fetch`, `diff`, `log`, `show`,
  `merge-base`, `rev-parse`. Never `merge`, `reset`, `checkout`, `push`,
  `rebase`, or anything that mutates the working tree or refs.
- The only file this skill writes is `docs/upstream/UPSTREAM_CHANGELOG.md`.
- Do not edit `SOURCE_REV`, `AGENTS.md` base, or any baseline metadata — those
  change only after a completed, verified merge.
- Do not fabricate features. If the commit message lists it, transcribe it; if
  not, either derive it from the diff (marked) or omit it.

## References

- `references/area-map.md` — crate/path → area mapping for grouping the
  Areas-touched table. Read it in Steps 4b/5.
- `assets/entry-template.md` — the entry skeleton to fill in Step 6.
