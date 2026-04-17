# EAB Acknowledgement And Anchor Architecture

Purpose: clarify the layered contract between player evidence, developer
definitions, authoritative EAB acknowledgement, and qcoin anchoring so future
work converges on a viable industrial architecture instead of treating EAB as a
thin wrapper around qcoin.

Related notes:

- [EAB_API_SURFACE.md](EAB_API_SURFACE.md)
- [ACHIEVEMENT_MODEL.md](ACHIEVEMENT_MODEL.md)
- [EAB_TRANSPORT_DESIGN_GOALS.md](EAB_TRANSPORT_DESIGN_GOALS.md)
- [LOADNGO_RUNTIME_MIGRATION.md](LOADNGO_RUNTIME_MIGRATION.md)
- [QCOIN_ANCHOR_ACCEPTANCE_GATE.md](QCOIN_ANCHOR_ACCEPTANCE_GATE.md)
- [`qcoin/docs/EAB_ANCHOR_TRANSACTION_MODEL.md`](../../qcoin/docs/EAB_ANCHOR_TRANSACTION_MODEL.md)

## Core distinction

At the qcoin layer, the contract is intentionally simple:

- a submitter presents a deterministic transaction to a qcoin node
- the qcoin cluster validates, orders, and includes it
- proof is about durable ordering and inclusion

At the EAB layer, the contract is higher-order:

- a player or client asserts that an accomplishment occurred
- a developer namespace defines what that accomplishment means
- an authoritative EAB service or node decides whether to acknowledge it
- EAB may then anchor that authoritative acknowledgement into qcoin

So qcoin is not the negotiation layer.
EAB is.

## Layered roles

### 1. Player or client

The player/client may:

- submit claims
- submit evidence
- submit offline session/order metadata

The player/client may not:

- define reward policy
- self-issue authoritative achievements or entitlements
- speak to qcoin directly for normal gameplay reward flow

### 2. Developer or publisher

The developer/publisher provides:

- namespaced definitions
- accomplishment rules
- visibility and proof posture
- entitlement semantics

These are registry data, not wire-time authority.

### 3. Authoritative EAB service or node

The authoritative EAB layer decides:

- whether a claim is valid enough to acknowledge
- whether direct award is allowed
- whether review is required
- whether an acknowledgement is idempotent, rejected, or repeatable
- whether the resulting authoritative record should be anchored into qcoin

This is the layer where product policy becomes durable service truth.

### 4. qcoin

qcoin does not decide whether a player deserves a reward.

qcoin only:

- accepts deterministic anchor transactions
- orders them
- persists them
- exposes inclusion/proof material back to EAB

## The actual contract surfaces

### A. Definition registration

Surface:

- developer/trusted-service to EAB

Purpose:

- register achievement and entitlement definitions under a developer/game
  namespace

Important:

- definitions are versioned data
- definitions are not hard-wired runtime facts

### B. Claim submission

Surface:

- player/client to EAB

Purpose:

- say "this happened"

Payload category:

- definition reference
- player binding
- idempotency
- session ordering
- optional evidence

Claims are non-authoritative.

### C. Acknowledgement or review

Surface:

- trusted-service adapter or authoritative node-to-node request

Purpose:

- resolve a definition reference locally
- apply accomplishment rules
- create an authoritative award or review outcome

This is the key EAB contract.

The request should carry:

- player id
- definition reference
- optional evidence context
- idempotency/source metadata

The request should not carry:

- a full achievement definition
- product display copy
- reward policy as caller-supplied truth

### D. qcoin anchor submission

Surface:

- EAB runtime to qcoin node

Purpose:

- anchor a deterministic hash of the authoritative EAB record

This contract should stay intentionally simple:

- EAB submits a transaction
- qcoin validates and includes it
- EAB tracks acceptance and inclusion lifecycle

### E. Receipt readback

Surface:

- EAB to clients/operators

Purpose:

- expose local authoritative receipt
- expose qcoin anchor state separately
- expose durable inclusion once confirmed

## Industrialization rules

For this architecture to scale beyond ad hoc prototypes, the following rules
must hold.

### 1. Definitions are data

- product achievements and entitlements belong in registry/config/state
- runtime code should not export built-in product reward definitions

### 2. Requests reference definitions

- wire payloads should use versioned definition references
- authoritative nodes resolve those references locally

### 3. Evidence is separate from policy

- claims may carry evidence
- definitions carry policy
- the caller does not get to redefine policy by shaping the request

### 4. Acknowledgement is distinct from anchoring

- EAB acknowledgement creates authoritative player-facing truth
- qcoin anchoring creates proof of ordered durability
- qcoin acceptance is not itself the business decision

### 5. Idempotency is mandatory

- claim ids, request ids, session ordering, and source metadata must prevent
  casual duplication
- `once_per_player` must behave deterministically

### 6. Privacy boundaries stay in EAB

- raw evidence and gameplay telemetry do not belong in qcoin
- qcoin should receive only deterministic anchor material derived from the
  authoritative EAB record

### 7. Transport responsibilities stay narrow

- IPv6 multicast: discovery and announcements only
- unicast: deterministic node-to-node follow-up
- HTTP: current client/trusted-service adapter

## What this means for loadngo

The `loadngo` node plane should ultimately carry:

- `PresenceAnnounce`
- `NodeInfo`
- `StatusRequest` / `StatusResponse`
- acknowledgement or review requests by definition reference
- anchor lifecycle visibility

It should not carry:

- full product definitions as the source of truth
- raw reward policy as caller authority
- broad state replication by default

## Practical near-term direction

Given the current prototype, the right next steps are:

1. replace node-plane full achievement payloads with definition references
2. resolve definitions locally on the authoritative node
3. enforce accomplishment rules from registered definitions
4. keep qcoin anchor lifecycle explicit and externally visible
5. expand evaluator logic only after the contract boundary is clean

## Short version

qcoin is the simple ordering substrate.

EAB is the negotiation and acknowledgement layer between:

- player evidence
- developer policy
- authoritative service judgement
- durable qcoin proof

Future work should strengthen that boundary, not blur it.
