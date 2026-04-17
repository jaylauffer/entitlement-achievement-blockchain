# EAB QCoin Anchor Acceptance Gate

Purpose: define the exact point at which `entitlement-achievement-blockchain`
can honestly claim that qcoin-backed anchor work is usable for the lab proof of
concept.

This note is narrower than the full EAB roadmap. It is only about the
authoritative-local-write plus qcoin-anchor path.

## What "qcoin-backed anchor work" means

For the current proof of concept:

- EAB remains the player-facing authority
- EAB writes the authoritative local record first
- EAB enqueues a qcoin anchor outbox item for that record
- a background runtime component submits the anchor transaction to qcoin
- EAB tracks anchor progress separately from the local authoritative receipt

The qcoin-side contract is documented in
[`qcoin/docs/EAB_ANCHOR_TRANSACTION_MODEL.md`](../../qcoin/docs/EAB_ANCHOR_TRANSACTION_MODEL.md).

## Acceptance criteria

The gate is passed only when all of the following are true.

### 1. Local authority does not depend on qcoin availability

An authoritative EAB mutation must still succeed locally when qcoin is
unreachable.

Pass condition:
- the local player log append succeeds
- the qcoin anchor remains pending in the outbox
- the API/runtime does not roll back the authoritative local write just because
  qcoin is down

### 2. Anchor work survives restart

Pending anchor work must persist across process restart.

Pass condition:
- after restart, EAB reloads the same pending outbox entries
- pending count is unchanged until the worker successfully drains them

### 3. A reachable qcoin node reaches durable inclusion

When a real qcoin node target is configured and reachable, EAB must submit the
pending anchor transaction and keep it visible until the exact anchor reaches
durable qcoin inclusion.

Pass condition:
- the exact anchor transaction becomes visible in qcoin block history
- pending count falls to zero only after that inclusion is visible
- last anchor success timestamp is recorded
- no anchor error remains for the successful case

### 4. Runtime status is externally visible

The EAB node/service plane must expose enough state to understand anchor health
without reading source.

Pass condition:
- status reports the configured qcoin target
- status reports pending outbox count
- status reports last anchor success and failure timestamps
- status is queryable over the `loadngo` service plane

### 5. One real authoritative EAB action reaches qcoin

At least one real EAB operation should produce a qcoin anchor through the
normal EAB path, not through a hand-crafted storage-only shortcut.

Examples:
- authoritative achievement award
- authoritative entitlement grant

Pass condition:
- EAB operation succeeds locally
- qcoin anchor work is enqueued
- the exact anchor becomes visible in qcoin block history

### 6. The lab can observe the result on all three devices

For the three-device lab:

- qcoin remains the stable substrate
- EAB nodes discover each other over multicast
- EAB nodes exchange direct status over unicast

Pass condition:
- each EAB node reports sane qcoin anchor status
- qcoin remains healthy while anchor work is flowing

## Explicitly out of scope for this gate

This gate does not require:

- EAB state replication
- multi-writer EAB semantics
- EAB consensus
- player traffic moving off HTTP
- full cryptographic inclusion-proof material beyond "the anchor transaction is
  visible in qcoin block history"

Those can become later gates.

## Test scaffold

### Automated local tests

These should run in normal `cargo test` without requiring a live qcoin node:

- `qcoin_anchor_outbox_survives_storage_restart`
  - proves local append plus outbox persistence across restart

### Live qcoin integration tests

These require an explicit lab target and are ignored by default:

- `qcoin_anchor_outbox_drains_against_live_qcoin_node`
  - requires `EAB_QCOIN_TEST_TARGET=host:port`
  - proves a real qcoin node reaches durable inclusion for the anchor before
    the outbox clears

### Manual lab checks

These still need to be executed as part of the acceptance gate:

1. run a real authoritative EAB action through the normal API/service path
2. observe pending anchor work on the local EAB node
3. observe the exact anchor transaction in the live qcoin cluster
4. query the EAB node plane and confirm status is consistent across the three
   devices

## Current status

Current state after the first qcoin-anchor/runtime slices:

- criteria `1` and `2` have implementation support
- automated scaffolding exists for local restart persistence and live qcoin
  inclusion behavior
- live lab validation now shows the remaining gap clearly:
  - local authoritative EAB actions succeed
  - qcoin accepts anchor submissions
  - some anchors become visible in qcoin blocks
  - but authoritative award anchors are not yet tracked through durable qcoin
    inclusion as a first-class lifecycle state
- criteria `3` through `6` are therefore still not passed

So the gate is defined, but not yet passed.
