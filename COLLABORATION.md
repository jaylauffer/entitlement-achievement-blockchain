# Multi-Device Collaboration Protocol

This file defines how two Codex sessions can work safely in the same workspace without stepping on each other.

## Scope
Applies to:
- `sng-rusty/`
- `qcoin/`
- `entitlement-achievement-blockchain/`

`starlight/` is out of scope unless explicitly claimed in the worklog.

## Branching Rules
1. Use one active branch per device per repo.
2. Branch naming format:
   - `jay/<device>/<topic>`
   - Examples: `jay/laptop/mic-config-ui`, `jay/desktop/qcoin-node-handshake`
3. Never edit from the same branch on two devices.
4. Keep `main` clean: no direct commits.

## Ownership Rules
1. Before editing, claim files/areas in `WORKLOG.md`.
2. Only one device can own a file subtree at a time.
3. If overlap is needed, coordinate and split by file boundaries.

Suggested `WORKLOG.md` row format:

```md
| UTC time | Repo | Branch | Device | Claimed paths | Status |
|---|---|---|---|---|---|
| 2026-03-05T10:20Z | sng-rusty | jay/laptop/mic-config-ui | laptop | src/ui/config.rs, assets/script.ron | in_progress |
```

## Required Git Config (Set Once Per Repo)
Run this once in each repo used by agents:

```bash
git config pull.rebase true
git config pull.ff only
git config merge.ff only
```

## Daily Git Loop (Rebase-Only)
Run in each active repo every 30-60 minutes:

```bash
git fetch origin
git rebase origin/main
cargo check
# run repo-specific tests if relevant
git push --force-with-lease
```

Rules:
- Rebase only. No merge commits.
- Only update remote history via `git pull --rebase --ff-only` or `git fetch` + `git rebase`.
- Do not run plain `git pull` and do not run `git merge`.
- Use `--force-with-lease` after rebasing published branches.
- If conflicts take more than 10 minutes, stop and split work into smaller branches.

## PR Strategy
1. One logical change per branch.
2. Use stacked PRs when dependent:
   - Base PR-B on PR-A branch.
   - Rebase PR-B after PR-A updates.
3. PR description must include:
   - Why this change exists
   - Files touched
   - Validation commands run

## Integration Between Repos
Treat cross-repo integration as coordinated releases, not a single commit across repos:
1. Land PRs in dependency order (e.g., `qcoin` -> `entitlement-achievement-blockchain` -> `sng-rusty`).
2. Pin dependency references explicitly (commit SHA/tag) where needed.
3. Update integration notes in PRs so both devices follow the same sequence.

## Quick Start Commands
Create new branches:

```bash
# sng-rusty
cd sng-rusty && git checkout -b jay/<device>/<topic>

# qcoin
cd ../qcoin && git checkout -b jay/<device>/<topic>

# entitlement-achievement-blockchain
cd ../entitlement-achievement-blockchain && git checkout -b jay/<device>/<topic>
```
