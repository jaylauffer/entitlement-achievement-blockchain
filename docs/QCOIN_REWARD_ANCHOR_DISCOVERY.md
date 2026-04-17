# QCoin Reward Anchor Discovery

Purpose: capture what the lab has actually proven about qcoin-backed anchoring
of authoritative EAB reward mutations, and where the current lifecycle still
breaks down.

This note is narrower than the full anchor acceptance gate. It is specifically
about the discovery made while testing authoritative achievement-award
anchoring.

## Terms

For this note, these states are distinct:

- `local authoritative success`
  - EAB accepted the authoritative mutation and appended the canonical local
    block
- `qcoin acceptance`
  - qcoin accepted the submitted anchor transaction, typically into mempool
- `qcoin inclusion`
  - the exact anchor transaction is visible in qcoin block history

The bug is in the transition from `qcoin acceptance` to `qcoin inclusion`, not
in the local authoritative mutation itself.

## Current anchor model

For `LEDGER_BACKEND=qcoin`, EAB currently does this:

1. append the authoritative local EAB block
2. derive a metadata-only qcoin transaction from that block
3. enqueue that transaction in a persisted outbox
4. let a `loadngo-proactor` worker submit the transaction to qcoin over the
   qcoin UDP wire
5. try to treat qcoin history visibility as the terminal success condition

This model is implemented in `rust/src/qcoin_ledger_storage.rs`.

## What the lab has proven

### 1. Local authoritative reward mutation works

The normal HTTP/service path for authoritative achievement awards works:

- identity/session exchange succeeds
- profile creation succeeds
- achievement definition registration succeeds
- authoritative achievement award succeeds
- reward readback reflects the local EAB mutation

So EAB is not failing to issue the authoritative reward locally.

### 2. Outbox persistence works

Pending anchor work survives restart and reload.

This is covered by:

- `qcoin_anchor_outbox_survives_storage_restart`

### 3. A single live anchor can reach qcoin

The lab has already shown that at least one anchor transaction can be submitted
to the live qcoin cluster and become durable enough for the outbox to drain.

This is covered by the ignored live integration scaffold:

- `qcoin_anchor_outbox_drains_against_live_qcoin_node`

### 4. Real EAB activity can produce real qcoin anchor traffic

The live HTTP path is not just enqueueing synthetic test data. Real EAB actions
do generate real qcoin anchor submissions.

## What the lab has reproduced

In repeated clean runs of the authoritative reward flow, the following pattern
was observed:

- the profile-create anchor became visible in qcoin block history
- the later authoritative achievement-award anchor did not appear in qcoin
  block history
- EAB had already dropped the pending outbox entry anyway

That means the current implementation can lose lifecycle visibility for an
authoritative reward anchor before durable inclusion is actually proven.

## What this discovery means

### 1. EAB currently models anchor success too coarsely

The current runtime/status model is effectively:

- pending
- success
- error

That is not enough.

The lab evidence shows EAB needs at least:

- `pending_submission`
- `accepted_not_included`
- `included`

Without that middle state, EAB cannot honestly say whether a reward anchor is
still waiting on qcoin inclusion or whether it is fully durable.

### 2. "qcoin accepted it" is not enough

For reward anchoring, qcoin acceptance must not be treated as the same thing as
durable inclusion.

The authoritative reward should remain operationally visible until the exact
anchor transaction is seen in qcoin block history.

### 3. The local reward and the qcoin proof are separate truths

Right now:

- the local EAB reward is real and visible to the player-facing system
- the qcoin proof of that reward may still be incomplete

That separation is acceptable for the proof of concept, but only if the system
tracks it honestly.

## What this discovery does *not* mean

It does not mean:

- the authoritative reward API is broken
- player claim acceptance is the current blocker
- qcoin is unusable as substrate
- local EAB writes should be rolled back just because qcoin inclusion is still
  pending

The failure is narrower: lifecycle tracking between accepted submission and
durable inclusion is incomplete.

## Required next changes

The next implementation slice should:

1. persist an `accepted_not_included` state for outbox entries
2. expose that state in the EAB status plane
3. keep authoritative reward anchors visible until inclusion is confirmed
4. record inclusion as a distinct terminal event
5. add a live regression test that proves the specific reward anchor reaches
   qcoin inclusion, not merely outbox drain

This is tracked as:

- `EW-021 QCoin inclusion lifecycle tracking`

## Required test shape

The test plan now needs four distinct levels:

### Local tests

- local authoritative append succeeds with qcoin unavailable
- outbox survives restart

### Live single-anchor tests

- one anchor reaches `included`

### Live multi-anchor tests

- two sequential anchors both remain visible until each is individually
  included

### Real reward-path tests

- a normal authoritative achievement award through the HTTP/service path
  produces a qcoin anchor that reaches `included`

## Relationship to the acceptance gate

This note explains why
[`QCOIN_ANCHOR_ACCEPTANCE_GATE.md`](QCOIN_ANCHOR_ACCEPTANCE_GATE.md) is not yet
passed.

The gate should not be considered complete until the reward-anchor lifecycle is
tracked through durable inclusion rather than inferred from coarse success.
