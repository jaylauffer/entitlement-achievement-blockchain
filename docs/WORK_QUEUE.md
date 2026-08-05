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
- the node uses the single bounded `eab-wire` discovery protocol with a
  source-bound cookie exchange before larger responses
- raw-UDP detailed status and mutation messages were removed; they await the
  authenticated secure-unicast contract
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
Status: `done`

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

Result:
- anchor progress is now persisted as `pending_submission`,
  `accepted_not_included`, and `included`
- status reporting exposes lifecycle counts plus accepted/included timestamps
- local and live tests now require qcoin inclusion truth instead of treating
  submission acceptance as terminal success

---

## EW-022 Achievement definition model and accomplishment rules
Status: `done`

Goal:
- separate achievement display copy, award policy, and accomplishment rules so
  achievement enablement has a coherent target

Tasks:
- define a structured achievement model note
- add policy/accomplishment fields to the registry model
- preserve modeled criteria and policy in authoritative award records
- add regression coverage for registry round-trip and award mapping

Definition of done:
- a source-of-truth achievement model note exists under `docs/`
- structured achievement definitions are accepted and persisted
- awards preserve criteria summary separately from player-facing description
- tests cover round-trip and award behavior

Result:
- documented in `docs/ACHIEVEMENT_MODEL.md`
- achievement definitions now support category, visibility, repeatability,
  issuance mode, and structured accomplishment rules
- authoritative awards now preserve criteria summary and serialized award
  policy metadata
- tests cover registry round-trip, structured API registration, current-shape
  default behavior, and award mapping

---

## EW-023 Achievement evaluator and proof posture
Status: `todo`

Depends on:
- EW-022
- EW-021

Goal:
- define how EAB should actually evaluate achievement claims/events and decide
  which achievements warrant public-proof posture

Tasks:
- choose the first evaluator shape for event-based and review-based
  achievements without hard-wiring product achievements into runtime code
- define how `event_key`, `threshold`, and `requires_evidence` should be
  interpreted
- define which achievements should remain private versus qcoin-proof oriented
- add tests for the first supported evaluator path

Definition of done:
- evaluator behavior is explicit for the first supported achievement class
- proof posture is product-policy driven rather than implied by transport
- tests cover at least one fully modeled achievement from event/claim to award

---

## EW-024 Replace node-plane award payloads with definition references
Status: `todo`

Depends on:
- EW-022

Goal:
- ensure node-plane award/acknowledgement requests reference registered
  definitions by identity instead of shipping full product definitions over the
  wire

Tasks:
- replace full achievement-definition payloads in the loadngo node plane with a
  reference shape:
  - `developer`
  - `game`
  - `achievement_id`
  - `version`
- require the receiving authoritative node to resolve the registered
  definition locally
- update tests so product fixtures remain test-local only
- keep the docs aligned with the "definitions are data, not code" rule

Definition of done:
- node-plane requests are no longer the source of truth for reward policy
- authoritative nodes resolve registered definitions locally
- tests and docs reflect the reference-based contract

---

## EW-025 Enforce accomplishment rules from registered definitions
Status: `todo`

Depends on:
- EW-022
- EW-024

Goal:
- make the modeled accomplishment rules operational instead of merely
  descriptive

Tasks:
- enforce `issuance_mode` when claims are reviewed or direct awards are issued
- enforce `requires_evidence` on claim-review paths where policy demands it
- enforce `once_per_player` as idempotent/reject behavior for duplicate awards
- limit qcoin anchor eligibility to the intended proof-bearing award classes
- remove any remaining product-specific proof-of-concept reward assumptions
  from runtime code paths

Definition of done:
- achievement acknowledgements respect registered accomplishment rules
- duplicate or policy-violating awards no longer slip through casually
- tests cover the first enforced rule set

---

## EW-026 Clarify acknowledgement and anchor architecture
Status: `done`

Goal:
- document the actual layered contract between player claims, developer
  definitions, authoritative EAB acknowledgement, and qcoin proof anchoring so
  future implementation work does not collapse those responsibilities together

Tasks:
- write a source-of-truth architecture note under `docs/`
- align transport/runtime notes with the acknowledgement-by-reference target
- align repo guidance so future agents do not treat full wire payloads as
  reward-policy authority

Definition of done:
- one architecture note explains the layer split clearly
- README/handoff/runtime docs point at that note
- near-term work is clearly aimed at reference-based acknowledgement rather
  than full-definition wire payloads

Result:
- documented in `docs/EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md`
- surrounding docs now explicitly distinguish:
  - qcoin ordering
  - EAB acknowledgement
  - definition registry authority
  - proof anchoring
- EW-024 and EW-025 are now framed as the direct implementation follow-ons

---

## Suggested near-term execution order
1. EW-027
2. EW-024
3. EW-025
4. EW-023
5. EW-001
6. EW-002
7. EW-003
8. EW-004
9. EW-005
10. EW-006
11. EW-008
12. EW-009
13. EW-010
14. EW-011
15. EW-012
16. EW-013

---

## EW-027 Production-capable EAB UDP transport
Status: `in progress`

Depends on:
- EW-019
- EW-026

Goal:
- implement secure, reliable UDP-based EAB transport without using multicast
  for private work or duplicating authority policy in transport adapters

Plan:
- [EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md](EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md)

Progress:
- the insecure prototype UDP award handler and remote-award runtime entry point
  have been removed
- canonical claim definition resolution moved behind `EabRuntime`
- initial protocol choices are recorded in
  [EAB_UDP_PROTOCOL_DECISIONS.md](EAB_UDP_PROTOCOL_DECISIONS.md)
- the shared `eab-wire` crate now supplies bounded V2 discovery framing,
  deterministic CBOR, anti-amplification challenge messages, and golden/error
  tests
- the live node now uses that discovery protocol exclusively; the unused EAB1
  JSON presence/status/award path was removed
- source-bound cookie validation is live and authority advertisements fail
  closed unless a secure endpoint and fingerprint are both configured
- active multicast probing is the accepted EAB provider-discovery model; an
  unsolicited presence/membership protocol is intentionally not retained
- deterministic trusted-pin selection filters unknown, expired, and
  wire-incompatible discovery candidates
- bounded `EABS` claim/status frames, persistent DER identity loading, the
  Quinn/rustls authority adapter, and the server-independent
  `eab-quic-client` crate are implemented
- `QuicEabClaimTransport` passes an end-to-end session-bound submit/status
  test against the same canonical runtime used by HTTP

First delivery gate:
- discovery-backed secure unicast construction passes the same canonical
  claim, idempotency, timeout reconciliation, restart, and policy tests as HTTP

Full delivery gate:
- player, trusted-service, and operator requirements in the implementation
  plan pass their security, reliability, parity, and platform test matrices
