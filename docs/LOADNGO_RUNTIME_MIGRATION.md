# EAB Loadngo Runtime Migration

Purpose: define the intended migration of `entitlement-achievement-blockchain`
from its current `actix-web`-owned runtime toward the same `loadngo`
network/proactor substrate that now backs `qcoin-node`.

This note is about runtime architecture, not player-facing product semantics.

## Why this exists

Today:

- EAB exposes a player-facing and trusted-service-facing REST API using
  `actix-web`
- qcoin-node already runs its live peer core on `loadngo-proactor` and
  `loadngo/network`
- Zhoenus docs place qcoin behind EAB, not directly in the game client

If qcoin is the proof layer and EAB is the authoritative player-facing service,
then EAB should eventually share the same node/runtime substrate where that
helps:

- event scheduling
- async outbox work
- node-to-node coordination
- qcoin anchoring
- future signed-service request handling

## Current state

Current EAB runtime facts:

- process entrypoint is `actix_web::HttpServer` in `rust/src/main.rs`
- request handling is defined in `rust/src/api.rs`
- qcoin mirroring lives in `rust/src/qcoin_ledger_storage.rs`
- qcoin anchoring now uses a persisted outbox plus a `loadngo-proactor` worker
  for background submission
- EAB now submits qcoin anchor transactions over the qcoin UDP wire when a
  node target is configured
- HTTP still owns process startup, but request completion no longer has to own
  qcoin anchor progression

So the current system is:

- correct enough for bootstrap experiments
- partially aligned with the intended qcoin/loadngo node model

## Migration goal

The target architecture is:

- EAB domain logic stays repo-owned and transport-agnostic
- `loadngo-proactor` owns scheduling, wakeups, and async task execution
- `loadngo/network` owns node-oriented transport and discovery where needed
- HTTP remains a compatibility/player-facing adapter, not the core runtime

In other words:

- today: `HTTP server owns EAB`
- target: `EAB core owns state and work; HTTP is one adapter`

## Desired layering

### 1. EAB core

Owns:

- profile logic
- achievement claim review
- authoritative award / entitlement issuance
- receipt generation
- qcoin anchor outbox state
- replay / persistence decisions

This layer should not depend on `actix-web`.

### 2. EAB runtime/node

Owns:

- proactor-driven task scheduling
- retry loops
- qcoin anchor submission
- optional EAB node-to-node transport later
- background persistence and reconciliation work

This layer should be the natural home for `loadngo-proactor`.

### 3. EAB transport adapters

Adapters may include:

- HTTP/JSON for clients and trusted services
- future loadngo-native node messages
- future signed request transport

This is where `actix-web` may remain temporarily.

## Near-term transport stance

Do **not** remove HTTP first.

Reason:

- Zhoenus integration is already framed around EAB as the game-facing service
- public clients need a stable request/response surface
- trusted-service review flows are easier to evolve if HTTP remains available

So the correct near-term move is:

- keep HTTP
- stop letting HTTP own the whole runtime model
- move background/qcoin/network work under a shared loadngo-style core

## Relationship to qcoin

The desired contract is:

- EAB submits anchor transactions to qcoin
- qcoin orders them and returns receipt/proof material
- EAB tracks the outbox and receipt lifecycle

That means EAB should converge toward the same runtime posture as qcoin for:

- transport ownership
- async scheduling
- retry semantics
- node health / observability

It does **not** mean EAB should become a copy of qcoin.

EAB remains:

- player/account/reward authority
- claim review and entitlement logic
- higher-level receipt API

qcoin remains:

- proof / ordering layer

## Relationship to Zhoenus

Zhoenus docs already require:

- local-first gameplay
- asynchronous backend sync
- no qcoin node in the client
- no privileged trusted-service credentials in the client

Moving EAB onto `loadngo` supports that direction because it gives EAB a better
shared substrate for:

- outbox processing
- retries
- background anchoring
- future cluster behavior

without changing the fact that Zhoenus still talks to EAB, not qcoin.

## Recommended migration sequence

### Phase 1: Separate EAB core from HTTP ownership

Goals:

- isolate business logic from `actix-web`
- make background work callable from a runtime-owned scheduler

Concrete steps:

- keep `api.rs` as an adapter
- move any request-coupled mutation/retry behavior behind transport-agnostic
  service interfaces
- make qcoin anchor submission callable independently of an HTTP request path

### Phase 2: Introduce a proactor-owned background runtime

Goals:

- use `loadngo-proactor` for outbox retry and background work
- stop relying on ad hoc request-thread behavior for mirror progress

Concrete steps:

- create an EAB runtime component that owns:
  - qcoin anchor outbox processing
  - retry timing
  - failure classification
  - eventual health snapshots

Current status:

- first slice landed
- qcoin anchor outbox processing now runs on `loadngo-proactor`
- retry timing exists as a fixed delay and still needs refinement
- EAB now also starts a proactor-owned UDP node service for multicast
  `PresenceAnnounce` and direct `NodeInfo` replies
- the acceptance gate for calling qcoin-backed anchor work usable in the lab is
  tracked in [QCOIN_ANCHOR_ACCEPTANCE_GATE.md](QCOIN_ANCHOR_ACCEPTANCE_GATE.md)

### Phase 3: Move qcoin anchoring onto the real qcoin submission contract

Goals:

- remove local dummy block proposal as the intended integration model
- submit anchor transactions to qcoin instead

Concrete steps:

- align with `qcoin/docs/EAB_ANCHOR_TRANSACTION_MODEL.md`
- switch EAB from local block proposal toward transaction submission +
  receipt tracking

Current status:

- first slice landed
- EAB now submits qcoin anchor transactions instead of local `POST /blocks`
- receipt tracking and richer acceptance/inclusion lifecycle are still pending

### Phase 4: Add optional EAB node transport on `loadngo/network`

Goals:

- enable future EAB node coordination on the same substrate as qcoin
- support node-local discovery/replication if EAB evolves that way

Important:

- this is optional after the core/runtime cleanup
- it should not block the first qcoin-anchor proof of concept

Current status:

- first discovery slice landed
- EAB starts a `loadngo/network` UDP service by default
- the service uses embedded IPv6 multicast bootstrap unless disabled
- peers multicast `PresenceAnnounce` and reply with direct `NodeInfo`
- peers can now request direct `StatusResponse` snapshots over unicast
- status snapshots include qcoin target, outbox pending count, and last
  anchor success/failure
- this is a service plane only; it does not yet replicate profile state or
  replace HTTP

## What should not change first

Do not start by:

- rewriting the public API transport away from HTTP
- forcing Zhoenus to adopt qcoin or loadngo-native wire messages
- introducing open-network EAB cluster semantics before qcoin anchoring is stable
- coupling player request latency to qcoin availability

## Immediate implications for current work

For the current proof-of-concept, the priorities are:

1. stabilize qcoin core nodes
2. define and implement EAB anchor transaction submission
3. add EAB outbox/retry semantics
4. then migrate EAB runtime ownership toward `loadngo-proactor`

So the correct interpretation is:

- shared `loadngo` substrate is the target direction
- but the first concrete win is not “replace HTTP”
- the first concrete win is “make EAB's background/qcoin integration run on the
  same kind of proactor-owned runtime model as qcoin”

## Short version

Target architecture:

- `Zhoenus -> EAB HTTP/API adapter -> EAB core/runtime -> qcoin`

Runtime posture:

- EAB core and background work should migrate onto `loadngo-proactor`
- future EAB node transport should use `loadngo/network`
- HTTP should remain as an adapter until the node/runtime core is stable
