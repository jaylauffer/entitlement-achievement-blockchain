# Entitlement Achievement Blockchain Agent Handoff

Purpose: provide future agents with repo-specific guidance so work remains secure, consistent, and easy to review.

Guiding philosophy:

- save as many lives as we may, even our own

In this repo, that should be interpreted as:

- preserve meaningful player state carefully
- do not casually destroy or over-broaden reward authority
- support recovery, reconciliation, and continuity
- protect privacy while still allowing responsible credit

## What this repo is

This repo is a Rust service that currently combines:
- profile management
- concept registry operations
- achievement and entitlement definition management
- player reward state mutation
- block-style append-only logging
- optional QCoin anchoring/mirroring
- REST API via `actix-web`
- identity exchange and session-token player auth

It is currently both product logic and infrastructure glue. That means small changes can have security and semantic consequences.

## Read in this order
1. `docs/TODO_AGENT_REVIEW.md`
2. `docs/WORK_QUEUE.md`
3. `docs/AUTHORIZATION_AND_OFFLINE_CLAIMS.md`
4. `docs/STATE_RECONCILIATION_MODEL.md`
5. `docs/EAB_API_SURFACE.md`
6. `docs/EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md`
7. `docs/SIGNED_SERVICE_REQUESTS_ROADMAP.md`
8. this file

## Current engineering priorities

Near-term priority is:
- fix reward authorization boundaries
- make duplicate profile behavior explicit
- ensure receipts are correct under concurrency
- clarify the actual ledger/chain model
- stabilize QCoin mirroring semantics
- define and start the `loadngo` runtime migration path for background/qcoin work
- harden identity/session behavior
- clean up build/dependency assumptions for QCoin integration

Current policy note:

- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [STATE_RECONCILIATION_MODEL.md](STATE_RECONCILIATION_MODEL.md)
- [EAB_API_SURFACE.md](EAB_API_SURFACE.md)
- [EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [ACHIEVEMENT_MODEL.md](ACHIEVEMENT_MODEL.md)
- [LOADNGO_RUNTIME_MIGRATION.md](LOADNGO_RUNTIME_MIGRATION.md)
- [QCOIN_ANCHOR_ACCEPTANCE_GATE.md](QCOIN_ANCHOR_ACCEPTANCE_GATE.md)
- [QCOIN_REWARD_ANCHOR_DISCOVERY.md](QCOIN_REWARD_ANCHOR_DISCOVERY.md)

Current implementation note:

- achievement claims now persist across restart
- achievement definitions now separate display copy, award policy, and
  accomplishment
- qcoin remains a simple proof-ordering substrate; EAB is the higher-order
  acknowledgement layer between player evidence, developer definitions, and
  authoritative reward decisions
- concrete achievements must not be hard-wired into production runtime code;
  examples belong in docs, tests, or external registry data
- players may submit and list their own claims
- trusted services with `award:achievements` may review claims and promote them into authoritative awards
- claim review remains private service state; raw claims are not public ledger entries

## Rules of engagement

### 1. Do not casually change security semantics
Before changing any of the following, add a short note under `docs/`:
- who may award achievements
- who may grant entitlements
- developer-token behavior
- player session/token behavior
- provider token verification behavior
- receipt semantics

### 2. Treat authorization as a first-class invariant
For changes touching API or service logic, test:
- authorized caller path
- unauthorized caller path
- malformed token or mismatched identity path
- correct HTTP status code

### 3. Do not assume the current “blockchain” model is final
The current implementation mixes per-player logs and a merged in-memory chain view.
If you change replay, receipt lookup, or block semantics, document what model you are moving toward.

### 4. Do not silently change operational behavior
If you change:
- env vars
- storage backend semantics
- auth expectations
- API routes or payloads
- QCoin mirror behavior
then update docs in the same change.

### 5. Do not hard-code product reward definitions
Achievement and entitlement definitions are product data.

Do not:
- hard-wire concrete achievements or entitlements into runtime modules
- export product-specific helper constructors from library code
- treat an incoming request payload as the source of truth for reward policy

Do:
- keep concrete examples in docs or tests
- load real definitions from registry/storage/config
- have runtime requests reference registered definitions by identity
- keep evidence, policy, acknowledgement, and qcoin proof as separate lifecycle
  concepts

## Where to put things

### Tests
Preferred placement:
- unit tests near module logic for registry, identity, replay, and service invariants
- integration tests for HTTP auth, receipt behavior, backend selection, and QCoin mirror paths

Areas especially needing more tests:
- `api.rs`
- `player_profile.rs`
- `identity.rs`
- `qcoin_ledger_storage.rs`

### Docs
Add short notes in `docs/` for changes to:
- authorization model
- duplicate profile behavior
- receipt semantics
- chain/replay model
- QCoin mirror consistency model
- loadngo/runtime ownership
- dependency/build strategy

## Build and validation expectations

At minimum, future agents should run and report:
- `cargo build --manifest-path rust/Cargo.toml`
- `cargo test --manifest-path rust/Cargo.toml`

If the change touches HTTP behavior, also exercise representative API flows.
If the change touches QCoin mirroring, explicitly note whether a sibling/local QCoin checkout was required.

## Areas that are easy to break

### Reward authorization
Current award/grant flows are sensitive and should not be changed without explicit tests proving players cannot self-award unless that is intentional.

### Receipt lookup
Anything relying on “latest transaction for player” is easy to get wrong under concurrency or future async processing.
Prefer direct return of created transaction info when practical.

### Replay semantics
Be careful when replaying per-player logs into one in-memory structure.
A change that seems harmless can alter restart behavior, receipts, or visible player state.

### Identity/session behavior
Sessions are currently memory-resident. Restart behavior, expiration, and provider-token fallback all have trust implications.
Do not broaden trust silently.

### QCoin mirroring
The mirror path can fail after local append.
Do not assume local append and mirror are currently atomic.
If changing this, document source-of-truth and retry behavior explicitly.
Current implementation direction:
- local append remains the canonical EAB write
- qcoin anchor submission is now outbox-driven and background-owned
- acceptance by qcoin is not yet the same thing as durable receipt lifecycle completion
- the current acceptance gate for calling this usable in the lab is documented
  in [QCOIN_ANCHOR_ACCEPTANCE_GATE.md](QCOIN_ANCHOR_ACCEPTANCE_GATE.md)
- live lab validation has now reproduced the remaining gap:
  authoritative EAB actions can be accepted by qcoin without EAB yet tracking
  durable inclusion as a separate lifecycle stage
- the detailed discovery note for that gap is
  [QCOIN_REWARD_ANCHOR_DISCOVERY.md](QCOIN_REWARD_ANCHOR_DISCOVERY.md)

### Runtime ownership
Current process ownership still lives in `actix-web`.
The target direction is documented in [LOADNGO_RUNTIME_MIGRATION.md](LOADNGO_RUNTIME_MIGRATION.md):
- keep HTTP as an adapter for now
- move background/qcoin work toward a `loadngo-proactor`-owned runtime
- add `loadngo/network` node transport only when the core/runtime boundary is ready

Current runtime status:
- qcoin anchor outbox processing runs on `loadngo-proactor`
- EAB now also starts a `loadngo/network` UDP node service
- the node service uses IPv6 multicast `PresenceAnnounce` plus direct
  `NodeInfo` replies
- peers can request direct `StatusResponse` snapshots over unicast
- status includes qcoin target plus anchor outbox health counters
- no EAB state replication is implemented on that service plane yet

### Dependency layout
This repo currently assumes access to sibling QCoin crates through local path dependencies.
Any build-system change should improve reproducibility, not just local convenience.

## What not to change casually

Avoid casual changes to:
- block hash calculation inputs
- receipt response shape
- identity exchange semantics
- default storage backend semantics
- env var names relied on in docs
- QCoin anchor payload structure

If one of these must change, include:
- rationale
- compatibility/migration note
- updated docs
- updated tests

## Recommended working style for future agents

For non-trivial tasks:
1. update the relevant item in `docs/WORK_QUEUE.md`
2. add a design note if semantics or trust boundaries change
3. implement narrowly
4. add tests for both auth and correctness
5. update docs in the same patch

## Commit/change summary expectations

When handing off work, include:
- what changed
- why it changed
- files touched
- tests added
- remaining security or consistency risks
- deployment/build implications

## If working on QCoin integration

Document explicitly:
- whether QCoin mirroring is best-effort or required
- whether local append may succeed when mirror fails
- what is anchored into QCoin
- how an external verifier should confirm the anchor
- whether a real qcoin-node is expected to accept mirrored data directly

Current answer:
- qcoin anchoring is best-effort from the request path's point of view
- local append may succeed while the qcoin anchor remains pending in the outbox
- the anchor payload is a metadata-only qcoin transaction derived from the canonical EAB block
- EAB now expects a real qcoin-node to accept anchor transactions over the qcoin UDP wire
