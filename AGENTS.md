# AGENTS.md

## Agent skills

### Setup

The `.agents/skills/` tree (Matt Pocock's engineering skills — `grill-with-docs`,
`to-spec`, `to-tickets`, `implement`, `tdd`, etc.) is gitignored; it's restored
from the committed `skills-lock.json` via the [Vercel `skills`
CLI](https://github.com/vercel-labs/skills):

```
npx skills experimental_install
```

Run this once per fresh clone/worktree before using any of the skills below.
(`experimental_install` is the CLI's actual restore-from-lockfile command —
verified 2026-07-13; don't confuse it with `skills add`, which fetches a fresh
copy rather than restoring the pinned versions.)

### Issue tracker

Issues and specs live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five canonical role labels (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context — `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
