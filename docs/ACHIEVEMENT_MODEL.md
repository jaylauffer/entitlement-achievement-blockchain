# Achievement Model

Purpose: define what an achievement means in `entitlement-achievement-blockchain`
so future work can enable achievements without confusing player-facing copy,
award policy, and durable proof requirements.

Related notes:

- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [STATE_RECONCILIATION_MODEL.md](STATE_RECONCILIATION_MODEL.md)
- [EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [QCOIN_ANCHOR_ACCEPTANCE_GATE.md](QCOIN_ANCHOR_ACCEPTANCE_GATE.md)
- [EAB_TRANSPORT_DESIGN_GOALS.md](EAB_TRANSPORT_DESIGN_GOALS.md)

## Why this note exists

The earlier achievement shape was too thin:

- `description` was doing double duty as both player-facing copy and success
  criteria
- there was no first-class model for whether an achievement is private or meant
  for public proof
- there was no explicit repeatability or issuance policy

That was enough to issue a basic award, but not enough to make achievement work
coherent as EAB moves onto the `loadngo` runtime and qcoin-backed anchoring.

## Core model

An achievement definition should answer four different questions.

### 1. Identity

This identifies the achievement within a developer/game namespace:

- `developer`
- `game`
- `achievement_id`
- `version`

### 2. Player-facing meaning

This is what the player or operator reads:

- `name`
- `description`
- `category`

`description` is for presentation and explanation, not the canonical success
rule.

### 3. Award policy

This answers how the achievement is meant to behave:

- `visibility`
  - `private`
  - `public_proof`
- `repeatability`
  - `once_per_player`
  - `repeatable`
- `issuance_mode`
  - `direct_award_only`
  - `claim_review_only`
  - `direct_award_or_claim_review`

These fields define intent even when the runtime does not yet enforce every
policy branch.

### 4. Accomplishment

This is the smallest structured statement of what counts as success:

- `summary`
- `event_key`
- `threshold`
- `requires_evidence`

Current meaning:

- `summary` is the canonical criteria text that should be preserved in an
  authoritative award record
- `event_key` and `threshold` are structured hints for future evaluation or
  review tooling
- `requires_evidence` signals whether a claim should be expected to carry more
  than basic idempotency/order fields

The current code does not implement a general-purpose automatic evaluator yet.
This model is for definition clarity first.

## Current runtime interpretation

The current implementation now treats an achievement definition like this:

- `description` remains player-facing copy
- `accomplishment.summary` becomes the authoritative award criteria text when
  it is present
- award policy fields are serialized into the achievement award metadata
- omitted policy/accomplishment fields default sanely within the current shape

That gives EAB enough structure to preserve achievement intent without carrying
historical-shape compatibility baggage in the prototype.

## Dev branch migration note

On April 17, 2026, commit `4644d1b` on `dev` reshaped
`AchievementDefinition` from one flat Rust struct into grouped sub-objects:

- `AchievementIdentity`
- `AchievementPresentation`
- `AchievementAwardPolicy`
- `AchievementAccomplishment`

Important clarification:

- `name` and `description` were not removed from the achievement model
- they moved into `AchievementPresentation`
- `#[serde(flatten)]` keeps those fields flat in serialized data
- accessor methods like `name()` and `description()` preserve read access

The practical compatibility break is in Rust construction sites that still use
the old flat struct literal form. Those callers must switch to
`AchievementDefinition::new(...)` and the builder-style helpers such as
`with_category(...)`, `with_policy(...)`, and `with_accomplishment(...)`.

## Definition source of truth

Achievement definitions are registry or service data, not runtime fixtures.

That means:

- do not hard-wire concrete achievements into production code
- do not export helper constructors for product achievements from runtime
  modules
- keep concrete examples in docs, test fixtures, or external registry data

Future node-plane and API requests should reference a registered achievement
definition by identity. They should not ship a full product definition as if
the caller were the source of truth.

## Rules of accomplishment

The intended rules are:

1. a registered definition identifies the accomplishment
2. a player claim does not by itself create an authoritative award
3. a trusted service or authoritative EAB node acknowledges the accomplishment
4. if `accomplishment.requires_evidence` is true, the claim/review path should
   carry evidence material before acknowledgement
5. if `visibility` is `public_proof`, the authoritative award is eligible for
   qcoin-backed proof anchoring
6. if `repeatability` is `once_per_player`, repeated acknowledgement should be
   treated as idempotent or rejected, not multiplied casually

Important:

- the current code now models these rules clearly
- the current code does not yet enforce every rule completely
- future work should enforce by reference to registered definitions, not by
  expanding request payload authority

## First supported achievement class

The first concrete supported class should be:

- one-time, developer-scoped progression achievements
- authoritative direct award or claim-review promotion
- optional public-proof posture

Example:

- `developer`: `dev1`
- `game`: `zhoenus`
- `achievement_id`: `first-flight`
- `version`: `1`
- `name`: `First Flight`
- `description`: `Complete your first successful run`
- `category`: `progression`
- `visibility`: `public_proof`
- `repeatability`: `once_per_player`
- `issuance_mode`: `direct_award_or_claim_review`
- `accomplishment.summary`: `Complete one successful run`
- `accomplishment.event_key`: `run_completed`
- `accomplishment.threshold`: `1`
- `accomplishment.requires_evidence`: `false`

This is intentionally simple:

- it is meaningful to players
- it can be awarded directly by a trusted service today
- it can also fit the claim-review path later
- it maps cleanly onto qcoin-backed anchoring when public proof is desired

## Success criteria for "achievements are enabled"

We should only say achievements are enabled in the current lab when all of the
following are true.

### 1. Structured definitions are accepted and persisted

Pass condition:

- the registry accepts the structured fields above
- registry round-trip preserves them
- omitted optional policy/accomplishment fields default within the current
  canonical shape

### 2. Claims remain non-authoritative

Pass condition:

- a player claim does not directly mutate authoritative reward state
- claim review or direct trusted-service award is still required

### 3. Authoritative awards preserve modeled criteria

Pass condition:

- a direct award or promoted claim records the achievement
- the award carries the canonical criteria summary
- the award metadata preserves category and policy fields

### 4. Qcoin-backed anchors reach inclusion

Pass condition:

- the authoritative achievement award creates qcoin anchor work
- the anchor remains visible until the exact qcoin transaction is included
- inclusion can be confirmed against qcoin history

### 5. Loadngo node status tells the truth

Pass condition:

- EAB node status exposes anchor lifecycle counts and timestamps
- the status is reachable over the `loadngo/network` node plane
- multicast is used only for discovery/presence, not for the authoritative
  reward payload itself

## Test scaffold

### Automated local tests

- registry save/load preserves achievement policy and accomplishment fields
- omitted optional fields default correctly within the current shape
- direct award preserves criteria summary and award metadata
- current trusted-service API path accepts a structured achievement definition

### Live qcoin-backed tests

- the qcoin anchor outbox test proves exact inclusion for an authoritative
  achievement anchor

### Manual lab checks

1. register a structured achievement definition
2. issue an authoritative award or promote a claim
3. confirm the local EAB receipt is correct
4. confirm the exact qcoin anchor reaches inclusion
5. confirm `loadngo` node status reflects that lifecycle truthfully

The former full-definition UDP award harness has been removed. The replacement
lab flow will discover the authority, establish authenticated secure unicast,
submit the canonical claim envelope, and confirm acknowledgement plus anchoring
without accepting caller-supplied definitions as authority.

## Explicitly out of scope for this note

This note does not define:

- a full achievement evaluator DSL
- anti-cheat guarantees for offline evidence
- multi-node EAB consensus
- player traffic moving off HTTP immediately

Those are later tasks. The goal here is a coherent achievement definition model
and a clear enablement bar for the current EAB/qcoin/loadngo architecture.
