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

(`experimental_install` is the CLI's actual restore-from-lockfile command —
verified 2026-07-13; don't confuse it with `skills add`, which fetches a fresh
copy rather than restoring the pinned versions.)

For **Claude Code** this is automated: the `SessionStart` hook in
`.claude/settings.json` runs `.claude/hooks/restore-skills.sh`, which restores
the store (only when missing) and then symlinks each skill into `.claude/skills/`
— the directory Claude Code indexes. The store lands in `.agents/skills/`, which
Claude does **not** scan, so the symlink bridge is what makes the pinned skills
discoverable. Both `.agents/` and the generated `.claude/skills/` are gitignored.
Other agents should run `npx skills experimental_install` once per fresh
clone/worktree.

### Issue tracker

Issues and specs live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five canonical role labels (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context — `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
