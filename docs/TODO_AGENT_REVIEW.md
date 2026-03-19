# Entitlement Achievement Blockchain Agent Review TODO

Purpose: give future agents a concrete checklist for reviewing, securing, and extending the entitlement/achievement service and its QCoin integration.

## Current snapshot

This service currently provides:
- player profile creation and mutation
- concept registry and profile concept merging
- achievement definition registration and award flow
- entitlement definition registration and grant flow
- file-backed ledger storage
- optional sled-backed ledger storage
- optional QCoin mirroring backend
- identity exchange and session-token based player auth
- REST API using `actix-web`

## Priority 0: fix security model

### 1. Lock down award/grant authorization
Current risk:
- authenticated players can currently invoke award/grant endpoints for their own profile
- server-side authorization boundaries need review

Tasks:
- review all achievement and entitlement award flows
- decide authoritative model:
  - server-only award path
  - developer-signed award requests
  - trusted service token model
- prevent players from self-awarding rewards unless explicitly intended
- document the authorization model end-to-end

Required tests:
- authenticated player attempts to self-award achievement
- authenticated player attempts to self-grant entitlement
- authorized service/developer successfully awards reward
- unauthorized developer token is rejected

### 2. Review developer-token handling
Tasks:
- review token loading from env/file
- confirm secrets are not logged
- define rotation expectations
- consider stronger token representation or hashing at rest if needed

Required tests:
- malformed token file
- empty token config
- duplicate developer entries
- mismatched developer/token pair

## Priority 1: make storage and replay semantics explicit

### 3. Clarify chain model
Current concern:
- service verifies per-player logs individually
- then merges verified blocks into one in-memory ledger view
- this may not represent a coherent global chain model

Tasks:
- define whether this system is:
  - one append-only ledger per player
  - one global chain
  - an event log with block-style integrity only
- align in-memory replay behavior with the intended model
- document what receipts and block hashes actually mean

Required tests:
- multiple players with interleaved histories
- replay order invariance checks
- duplicate block-hash handling across player logs

### 4. Handle partial-commit behavior in QCoin mirroring
Current risk:
- topic log append happens before QCoin mirroring
- if mirroring fails, persistence becomes partially committed

Tasks:
- define desired consistency model:
  - local log is source of truth and qcoin mirror is best-effort
  - or append must fail unless mirror succeeds
  - or use an outbox/retry mechanism
- implement explicit retry/outbox behavior if needed
- document operational recovery steps

Required tests:
- topic append succeeds and qcoin mirror fails
- retry completes successfully without duplicate logical award
- remote node rejects mirrored block

## Priority 2: correctness and API behavior

### 5. Make profile creation idempotent or reject duplicates
Current concern:
- `create_profile` inserts unconditionally
- repeated create calls can reset state or create confusing history

Tasks:
- decide behavior for repeated create attempts
- implement either:
  - `409 Conflict`
  - idempotent return of existing profile
- update API docs accordingly

Required tests:
- repeated create for same player
- create after restart with existing logs

### 6. Review receipt generation correctness
Current concern:
- API currently returns latest matching player transaction as receipt
- verify this is always the intended just-created transaction

Tasks:
- confirm receipt lookup cannot accidentally return the wrong latest transaction under concurrency
- consider returning the created transaction directly from service methods
- add explicit receipt integrity checks

Required tests:
- concurrent achievement and entitlement operations for same player
- receipt contains expected transaction id and type

### 7. Validate request payload invariants
Tasks:
- review all REST payloads for missing validation
- enforce dimension size sanity for profile vectors
- validate quantity and expiration fields for entitlements
- validate version fields and identifier lengths/formats if needed

Required tests:
- malformed dimension payload
- oversized vector payload
- zero or invalid entitlement quantity policy
- unsupported provider identifiers

## Priority 3: identity and session model review

### 8. Harden identity/session lifecycle
Current concern:
- session tokens live only in memory
- restart behavior clears sessions
- no expiration policy is defined

Tasks:
- define session lifetime and revocation behavior
- decide whether sessions should persist across restart
- add expiration or cleanup if needed
- document trust model for identity exchange

Required tests:
- restart invalidates sessions as expected or persists them intentionally
- invalid token rejected
- unsupported provider rejected
- configured provider token mapping honored correctly

### 9. Review provider token fallback behavior
Current concern:
- if no provider mapping is configured, incoming token becomes subject directly
- verify whether this is acceptable outside local development

Tasks:
- define safe production behavior
- consider explicit development-only mode
- fail closed in production configuration if appropriate

Required tests:
- empty provider config in production mode
- direct token-as-subject behavior in development mode only

## Priority 4: QCoin integration contract

### 10. Stabilize dependency and integration strategy
Current concern:
- current Cargo setup uses relative path dependencies into a sibling `qcoin` repo
- this is fragile for CI, deployment, and outside contributors

Tasks:
- choose one dependency strategy:
  - git dependency
  - workspace/super-repo
  - vendored submodule
  - published internal crates
- update build and CI docs

Required tests:
- fresh clone build in CI environment
- build with and without qcoin backend enabled if feature-gating is introduced

### 11. Revisit mirrored QCoin block semantics
Current concern:
- mirrored blocks anchor EAB block metadata hash into QCoin output metadata
- verify whether this is the long-term intended anchoring model

Tasks:
- document exactly what is anchored and how verification should work
- define external verifier workflow
- decide whether mirroring should use transactions, blocks, or a dedicated anchoring primitive

Required tests:
- deterministic metadata hash generation
- verifier recomputes same anchor from stored EAB block
- mirror replay after restart remains consistent

## Priority 5: operational quality

### 12. Add structured error handling and operator signals
Tasks:
- improve error bodies returned by API where appropriate
- separate client errors from server errors clearly
- add logging for authorization failures, replay quarantine, and mirror failures

### 13. Add deployment and environment docs
Tasks:
- create operator-facing doc for env vars
- define safe production defaults
- document storage backend tradeoffs: `file` vs `sled` vs `qcoin`

## Suggested execution order
1. award/grant authorization fix
2. duplicate profile policy
3. receipt correctness under concurrency
4. chain model clarification
5. partial-commit + qcoin mirror strategy
6. identity/session hardening
7. dependency/build cleanup
8. operator docs and logging

## Deliverables expected from future agents
For each completed task, future agents should produce:
- code changes
- tests for both success and failure paths
- short notes under `docs/` explaining decisions
- explicit statement of unresolved tradeoffs
