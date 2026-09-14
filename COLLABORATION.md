# Collaboration

`entitlement-achievement-blockchain` follows the workspace-wide protocol in
[`../COLLABORATION.md`](../COLLABORATION.md), and claims live on the shared
board, [`../AGENT-BOARD.md`](../AGENT-BOARD.md). Both are at the `pudding`
root, next to this repository.

The March 2026 rules that used to be here assumed Codex sessions on separate
machines, one branch per device. Claude Code and Codex now share one checkout
on one machine, so claims, staging, and pushes follow the root protocol.

Still true for this repo:

- History is rebase and fast-forward only; no merge commits.
- Cross-repo changes land in dependency order: `qcoin`, then this repo, then
  its consumers. Its `dev` branch depends on `qcoin` and `loadngo` through
  sibling path dependencies.
