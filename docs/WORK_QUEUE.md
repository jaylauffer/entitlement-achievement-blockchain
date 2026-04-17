# Entitlement Achievement Blockchain Work Queue

Purpose: break the broader review TODO into small, independently executable work items for future agents.

Status labels to use in updates:
- `todo`
- `in-progress`
- `blocked`
- `done`

Each item should be updated with:
- owner/agent
- date started
- date completed
- files touched
- tests added
- remaining risks

---

## EW-001 Reward authorization model note
Status: `todo`

Goal:
- define who is allowed to award achievements and entitlements

Tasks:
- review current API and service flows
- choose authoritative model
- write short design note under `docs/`

Definition of done:
- explicit security model documented
- future code work has a clear target

---

## EW-002 Achievement award authorization fix
Status: `todo`

Depends on:
- EW-001

Goal:
- prevent players from self-awarding achievements unless explicitly intended

Tasks:
- update API auth path
- add service-side checks
- return appropriate HTTP status codes

Definition of done:
- unauthorized player award is blocked
- authorized path remains functional
- tests cover both cases

---

## EW-003 Entitlement grant authorization fix
Status: `todo`

Depends on:
- EW-001

Goal:
- prevent players from self-granting entitlements unless explicitly intended

Tasks:
- update API auth path
- add service-side checks
- return appropriate HTTP status codes

Definition of done:
- unauthorized player grant is blocked
- authorized path remains functional
- tests cover both cases

---

## EW-004 Duplicate profile policy
Status: `todo`

Goal:
- decide and implement behavior for repeated profile creation

Tasks:
- choose `409 Conflict` vs idempotent return
- update service implementation
- update API behavior and docs

Definition of done:
- repeated create behavior is deterministic
- tests cover restart and repeat-create paths

---

## EW-005 Receipt correctness under concurrency
Status: `todo`

Goal:
- ensure returned receipts always correspond to the operation just performed

Tasks:
- review `latest_award_receipt` behavior
- consider returning transaction/block info directly from service layer
- add concurrency-sensitive tests

Definition of done:
- receipt lookup cannot drift to unrelated latest transaction

---

## EW-006 Chain model clarification
Status: `todo`

Goal:
- define what the EAB “blockchain” actually represents

Tasks:
- document whether model is per-player chain, global chain, or event-log-with-integrity
- align replay and receipt semantics with that model

Definition of done:
- clear design note exists
- replay behavior is no longer ambiguous

---

## EW-007 Multi-player replay correctness
Status: `todo`

Depends on:
- EW-006

Goal:
- verify restart replay works correctly across multiple players

Tasks:
- add tests with interleaved player histories
- verify merged in-memory representation matches intended design
- check duplicate block-hash handling assumptions

Definition of done:
- multi-player replay behavior is covered by tests

---

## EW-008 QCoin mirror consistency policy
Status: `todo`

Goal:
- define how local append and qcoin mirror failures should behave

Tasks:
- choose source-of-truth model
- decide whether mirror is best-effort, required, or retried via outbox
- write short design note

Definition of done:
- consistency model documented
- future implementation work has a clear target

---

## EW-009 Partial-commit handling for QCoin mirroring
Status: `todo`

Depends on:
- EW-008

Goal:
- eliminate or explicitly manage partial commits when local append succeeds and mirror fails

Tasks:
- implement chosen strategy
- add recovery or retry path if needed
- surface operator-visible failure state

Definition of done:
- mirror failure behavior is deterministic and test-covered

---

## EW-010 Identity/session lifecycle review
Status: `todo`

Goal:
- define session expiration, restart semantics, and trust boundaries

Tasks:
- review in-memory session storage
- decide expiration/revocation policy
- document restart expectations

Definition of done:
- session model is explicit
- tests cover invalidation or persistence behavior as intended

---

## EW-011 Provider token fallback hardening
Status: `todo`

Goal:
- make token-as-subject fallback safe and environment-appropriate

Tasks:
- decide whether fallback is development-only
- add explicit production guard if needed
- document safe deployment expectations

Definition of done:
- production behavior fails closed unless intentionally configured otherwise

---

## EW-012 Payload validation pass
Status: `todo`

Goal:
- enforce sane request validation across the API

Tasks:
- validate vector dimensions and payload size
- validate quantity and optional expiration fields
- review identifier and version field expectations

Definition of done:
- malformed input produces clear client errors
- oversized or invalid payloads are rejected safely

---

## EW-013 Dependency/build cleanup for QCoin integration
Status: `todo`

Goal:
- remove fragile local path dependency assumptions

Tasks:
- choose dependency strategy
- update Cargo configuration
- add build/setup notes for fresh environments

Definition of done:
- fresh clone build story is documented and reproducible

---

## EW-014 QCoin anchoring verification workflow
Status: `todo`

Goal:
- document how anchored EAB blocks should be verified against QCoin

Tasks:
- define exactly what metadata is hashed and anchored
- describe verifier steps
- add deterministic verification test cases

Definition of done:
- external verification workflow is documented and reproducible

---

## EW-015 Logging and operator diagnostics
Status: `todo`

Goal:
- improve operational visibility into auth, replay quarantine, and mirror failures

Tasks:
- standardize logging points
- separate client and server failure signals
- add notes for operators

Definition of done:
- key failure modes are visible without source inspection

---

## EW-016 Deployment/env documentation cleanup
Status: `todo`

Goal:
- make backend/storage/env configuration easy to reason about

Tasks:
- document `file`, `sled`, and `qcoin` backends
- document safe defaults
- add minimal production notes

Definition of done:
- operators can configure and run the service from docs alone

---

## EW-017 Loadngo runtime migration note
Status: `done`

Goal:
- define how EAB should migrate onto the same `loadngo` proactor/network substrate as qcoin

Tasks:
- document current `actix-web` ownership model
- define target layering for EAB core, runtime, and transport adapters
- define migration order relative to qcoin anchoring work

Definition of done:
- migration note exists under `docs/`
- runtime ownership target is explicit

Result:
- documented in `docs/LOADNGO_RUNTIME_MIGRATION.md`
- current direction is explicit: keep HTTP as an adapter while moving background/qcoin work toward a proactor-owned core

---

## EW-018 Proactor-owned qcoin anchor outbox
Status: `done`

Depends on:
- EW-008
- EW-009
- EW-017

Goal:
- move qcoin anchoring and retry behavior out of request-thread ownership and toward a runtime component suitable for `loadngo-proactor`

Tasks:
- introduce an explicit qcoin anchor outbox model
- define retry timing and failure classification
- make anchor progression callable independently of HTTP request paths
- prepare the runtime boundary for `loadngo-proactor`

Definition of done:
- qcoin anchoring is runtime-owned and retryable
- request completion does not have to own mirror progress
- tests cover retry and restart behavior

Result:
- `QCoinLedgerStorage` now persists anchor work into an outbox instead of proposing local dummy qcoin blocks inline
- a `loadngo-proactor` worker drains that outbox in the background when a qcoin node target is configured
- EAB now submits qcoin anchor transactions over the qcoin UDP wire contract instead of `POST /blocks`

---

## EW-019 Loadngo-backed EAB runtime
Status: `done`

Depends on:
- EW-018

Goal:
- begin migrating EAB background/runtime ownership onto `loadngo-proactor` and later `loadngo/network`

Tasks:
- create a transport-agnostic EAB runtime component
- integrate `loadngo-proactor` for background scheduling
- keep HTTP as an adapter during transition
- define whether any EAB node-to-node transport is actually needed for the first pass

Definition of done:
- a non-HTTP runtime core exists
- proactor-owned background work is live
- adapter boundaries are documented and testable

Result so far:
- qcoin anchoring already runs on a `loadngo-proactor` worker
- a transport-agnostic `EabRuntime` core now exists
- current adapters call into that runtime instead of owning
  `PlayerProfileService` directly
- EAB now starts a `loadngo-proactor`-owned UDP node service alongside HTTP
- the node service uses `loadngo/network` with embedded IPv6 multicast
  bootstrap by default
- multicast is currently limited to `PresenceAnnounce`; peers answer directly
  with `NodeInfo`
- peers can request direct `StatusResponse` snapshots over unicast
- status snapshots expose qcoin target, explicit outbox lifecycle counts, and
  last accepted/included/success/failure timestamps
- HTTP remains the public/player-facing adapter while the service plane moves
  onto loadngo

---

## EW-020 QCoin anchor acceptance gate
Status: `done`

Depends on:
- EW-018
- EW-019

Goal:
- define the exact conditions under which qcoin-backed EAB anchoring is good
  enough to rely on in the lab proof of concept

Tasks:
- write an explicit acceptance gate note
- map the gate to automated local tests, live qcoin tests, and manual lab
  checks
- add integration-test scaffolding for the live qcoin drain path

Definition of done:
- gate exists under `docs/`
- test scaffold exists under `rust/tests/`
- current "implemented but not yet passed" state is explicit

Result:
- documented in `docs/QCOIN_ANCHOR_ACCEPTANCE_GATE.md`
- gate criteria now distinguish local authority, outbox persistence, live qcoin
  inclusion, and multi-node status visibility
- integration tests now scaffold restart persistence and ignored live qcoin
  inclusion coverage

---

## EW-021 QCoin inclusion lifecycle tracking
Status: `todo`

Depends on:
- EW-018
- EW-020

Goal:
- distinguish qcoin mempool acceptance from durable block inclusion and keep EAB
  anchor state accurate until inclusion is actually visible

Tasks:
- persist an accepted-but-not-yet-included anchor state instead of treating
  acceptance as terminal success
- record inclusion metadata separately from submission acceptance
- surface inclusion-vs-acceptance state in status reporting
- add tests for the reproduced lab case where one anchor is accepted but not yet
  visible in qcoin history
- keep the discovery note current in `docs/QCOIN_REWARD_ANCHOR_DISCOVERY.md`

Definition of done:
- EAB does not clear anchor tracking purely on qcoin acceptance
- authoritative award anchors remain visible as pending until inclusion is
  confirmed
- status and tests make the distinction explicit

---

## Suggested near-term execution order
1. EW-001
2. EW-002
3. EW-003
4. EW-004
5. EW-005
6. EW-006
7. EW-008
8. EW-009
9. EW-010
10. EW-011
11. EW-012
12. EW-013
13. EW-018
14. EW-019
