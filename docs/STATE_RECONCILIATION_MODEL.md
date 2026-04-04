# State Reconciliation Model

Purpose: define which categories of game state should remain local, which
should reconcile privately through EAB, and which should become publicly
auditable ledger state.

Guiding philosophy:

- save as many lives as we may, even our own

Applied here, that means:

- preserve meaningful player effort
- preserve recoverable continuity across offline and online play
- avoid needless erasure of claims or progression
- avoid overexposing private life, play history, or identity
- grant authoritative credit responsibly rather than casually or dismissively

This note exists because "offline-capable single-player game progression"
contains several different kinds of state:

- instant local feedback
- durable player progression
- economic or access-granting mutations
- publicly meaningful achievements

Those should not all be treated the same.

## Core Distinction

There are three different state categories:

1. local gameplay state
2. authoritative reconciled state
3. publicly reconciled ledger state

These are related, but not identical.

The stance behind this model is:

"A player can absolutely make a meaningful claim. The system does not need to
call them a liar. And it also does not need to immediately convert that claim
into fully authoritative credit."

## 1. Local Gameplay State

This is state the game may keep and use immediately without server or ledger
involvement.

Typical examples:

- acknowledgments
- temporary counters
- combo chains
- local stats
- local-only XP pacing
- temporary unlock progress
- tutorial progress

Properties:

- should work offline
- can update instantly
- may be discarded, resynced, or recomputed later
- does not inherently need public proof

Not all local gameplay state needs to leave the device.

## 2. Authoritative Reconciled State

This is state that should become canonical for the player's account, but does
not necessarily need to be public.

Typical examples:

- levels
- skills
- durable XP totals
- character/account progression
- unlocked story milestones
- private account statistics

Properties:

- may be built from offline session bundles or claims
- should support cross-device continuity
- should be server-validated before becoming canonical
- does not automatically imply public visibility

This is the most likely destination for "bundle of activity" reconciliation in
offline-capable single-player games.

## 3. Publicly Reconciled Ledger State

This is state that should be durably recorded in a verifiable way because other
systems or observers may rely on it.

Typical examples:

- authoritative achievements intended as proof
- durable entitlements
- economic rights or access rights
- season, tournament, or competitive outcomes
- rewards whose validity matters outside the local save

Properties:

- must be authoritative
- should not be directly client-issued
- may be mirrored to a public or auditable chain such as QCoin
- should be a subset of broader gameplay/account state, not a dump of all local activity

## Recommended Default Policy

For EAB-integrated single-player games:

- local acknowledgments stay local
- progression metrics may reconcile privately
- achievements may reconcile privately first, then become authoritative awards
- entitlements remain authoritative service-issued state
- only the subset requiring public proof should be mirrored or publicly anchored

So the answer to "does the bundle of offline activity need public
reconciliation?" is:

- no, not by default
- yes, for the subset with shared, durable, or economically meaningful consequences

## Privacy Model

Player privacy should be treated as a first-class design requirement.

Not every player wants:

- their progression visible publicly
- their detailed play history exposed
- their narrative choices exposed
- their device/session evidence exposed

So the default privacy rule should be:

- keep state private unless there is a strong reason to make it public

### Default Privacy Boundaries

#### Local State

Local gameplay state should remain private by default.

This includes:

- immediate acknowledgments
- local counters
- temporary progress
- private play patterns

#### Reconciled Private State

Most durable progression should reconcile privately, not publicly.

This includes:

- XP
- levels
- skills
- chapter progress
- private statistics
- detailed claim evidence

This state may be canonical for the player's account without being public to
other players or third parties.

#### Publicly Auditable State

Only the subset that truly needs public proof should be publicly auditable.

Examples:

- entitlements with interoperability or economic consequences
- achievements intended as external proof
- rewards another service must verify independently

Even then, the public record should reveal as little as possible.

## Privacy Design Principles

### 1. Data Minimization

Do not publish more than is necessary to prove the relevant fact.

Prefer:

- award ids
- timestamps
- opaque player identifiers
- hashes
- receipts

Over:

- raw play history
- verbose evidence payloads
- detailed narrative choices
- device telemetry

### 2. Default-Private Progression

Progression state should be private unless a product requirement explicitly says
otherwise.

That means:

- account level does not imply public level board
- skill progression does not imply public skill history
- session bundles do not imply public ledger publication

### 3. Selective Public Proof

When public verification is needed, publish the minimum proof-bearing outcome,
not the entire underlying evidence trail.

Examples:

- publish that an achievement was awarded
- do not publish the full offline claim journal by default

### 4. Pseudonymous Public Identity

If something becomes public, it should use a game/player identifier or public
handle, not real-world identity by default.

Public proof should not require exposing:

- legal name
- email
- external provider identity

### 5. Private Evidence, Public Outcome

Raw claim evidence should usually remain private even when the resulting award
becomes public.

That allows:

- private verification inputs
- public proof of the verified result

without exposing the player's full behavioral record.

## Offline Session Bundles

When a player comes back online, the game may submit a bundle containing:

- pending achievement claims
- progression deltas
- session metadata
- optional evidence or attestation material

The service should then decide which parts become:

- ignored or purely informational
- reconciled private account state
- authoritative reward/entitlement mutations
- publicly mirrored ledger state

The client should not decide this categorization unilaterally.

## Example Categorization

### Keep Local

- "Nice combo!"
- current streak
- temporary score
- immediate UI acknowledgments

### Reconcile Privately Through EAB

- player XP total
- skill progress
- account level
- chapter completion state
- private completion statistics

### Promote To Authoritative Award

- achievement earned after policy verification
- durable milestone unlock

### Promote To Publicly Auditable Ledger State

- entitlement grants
- proof-bearing achievements
- rewards with economic or interoperability consequences

## Design Rule

Do not model all player progression as public blockchain state.

Instead:

- keep local state local when possible
- reconcile private account state when useful
- promote only selected authoritative outcomes to public or auditable ledger state

That keeps offline play practical and avoids over-ledgering routine single-player
activity.

## Immediate Implication For EAB

The current `achievement-claims` path should be treated as the start of this
pipeline:

- player submits pending claim or session-derived event
- service stores that claim durably and later verifies and classifies it
- only then does it become authoritative award state

Future design work should add an explicit model for:

- private reconciled progression state
- claim verification and promotion
- public mirror policy

## Related Notes

- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [SIGNED_SERVICE_REQUESTS_ROADMAP.md](SIGNED_SERVICE_REQUESTS_ROADMAP.md)
