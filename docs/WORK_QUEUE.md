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
