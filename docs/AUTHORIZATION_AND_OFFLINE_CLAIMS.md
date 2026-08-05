# Authorization And Offline Claims Model

Purpose: define the intended trust model for achievement awards, entitlement
grants, and offline play synchronization.

Related policy boundary:

- [STATE_RECONCILIATION_MODEL.md](STATE_RECONCILIATION_MODEL.md)

This note exists because the service currently has an overly broad mutation
surface: a logged-in player can directly trigger reward-state changes through
the award/grant endpoints. That is not the intended long-term model.

## Core Distinction

There are two different concepts:

1. a **claim**
2. an **award**

A claim is user/device-submitted evidence that something happened.

An award is the authoritative server-side decision that a reward should be
recorded in the ledger.

Those are not the same thing.

Policy tone matters here:

"A player can absolutely make a meaningful claim. The system does not need to
call them a liar. And it also does not need to immediately convert that claim
into fully authoritative credit."

## Authoritative Rules

### Achievements

Players do **not** directly award achievements to themselves.

Instead:

- a player/client may submit an achievement claim
- the server verifies that claim according to policy
- if valid, the server issues the canonical achievement award

So the authoritative ledger contains awards, not raw client claims.

### Entitlements

Players do **not** directly grant entitlements to themselves.

Entitlements remain server-issued or trusted-service-issued only.

Reason:

- entitlements are closer to inventory, access rights, currency, or durable
  unlocks
- these are higher-risk economic/state mutations than ordinary achievements
- offline self-granting would be too easy to abuse

So the intended rule is:

- achievements may support offline claim submission
- entitlements remain authoritative online grants only

## Offline Single-Player Flow

This model is meant to support a player who is offline during gameplay, such as
airplane mode.

The intended flow is:

1. player plays offline
2. game records native local EAB achievement occurrences
3. game regains connectivity
4. client submits claim-ready occurrences in canonical envelopes
5. service verifies and deduplicates claims
6. service appends authoritative achievement awards to the ledger

Important:

- offline play should create **local game-scoped acknowledgements** whose
  immutable records may later be claimed online
- offline play should **not** directly mutate authoritative reward state

Current implementation note:

- the initial thin `achievement-claims` API path remains a pending/manual-review
  compatibility path
- those pending claims are separate from authoritative awards
- pending claims are now persisted across restart
- trusted-service review may later promote a pending claim into an authoritative award
- the canonical `achievement-claim-envelopes` path verifies the full offline
  record and registered policy, then returns a structured server
  acknowledgement/rejection/conflict
- an acknowledged canonical claim creates or references the once-per-account
  authoritative award; it does not turn the client into an award authority

## What An Offline Claim Represents

A claim should be understood as:

- authorship evidence
- ordering evidence
- idempotency input
- optional gameplay evidence

It is not proof in the strongest possible sense. For offline single-player
games, the practical target is tamper-evident and policy-checkable, not perfect
cheat-proofing.

## Minimum Claim Properties

An offline achievement claim should include enough information for:

- player binding
- device or client-key binding
- deduplication
- ordering within a session or claim stream
- policy review

Suggested fields:

- `player_id`
- `game`
- `achievement_id`
- `claim_id`
- `session_id`
- `client_sequence`
- `claimed_at`
- optional gameplay evidence payload
- client signature or authenticated proof material

The exact payload shape can evolve later. The important point is that the
service verifies claims and decides whether they become awards.

## Authorization Model

### Player Session Token

A player session token may authorize:

- profile creation for that player
- reading that player's profile
- reading that player's rewards
- player-owned profile mutations that are explicitly allowed
- submission of achievement claims

A player session token should **not** authorize:

- direct achievement award issuance
- direct entitlement grant issuance
- developer registry mutations

### Developer / Trusted Service Authorization

Developer or trusted service authorization is the near-term mechanism for:

- registering achievement definitions
- registering entitlement definitions
- issuing authoritative achievements directly, if desired
- issuing entitlements

Near term, a trusted service token or developer token model is acceptable.
Those tokens should be scope-based, not just developer-matched. Minimum scopes:

- `manage:concepts`
- `register:definitions`
- `award:achievements`
- `grant:entitlements`

That means "trusted-service authorization" should mean:

- developer namespace match
- appropriate mutation scope
- auditable bearer or later signed-request identity

Longer term, this may evolve toward signed service requests instead of static
developer bearer tokens.

Roadmap note:

- [SIGNED_SERVICE_REQUESTS_ROADMAP.md](SIGNED_SERVICE_REQUESTS_ROADMAP.md)

## Intended Endpoint Direction

The service should move toward this separation:

- player-facing:
  - identity exchange
  - profile read/update within player-owned bounds
  - reward read
  - achievement claim submission

- trusted-service-facing:
  - achievement definition registration
  - entitlement definition registration
  - authoritative achievement award
  - authoritative entitlement grant

That means the current award/grant endpoints should be treated as
trusted-service endpoints, not player endpoints.

## Why Client Signatures Alone Are Not Enough

A signed client request is useful, but not sufficient to justify direct offline
self-awards.

Reason:

- a compromised or modified client can still sign false claims if it controls
  the signing key
- therefore the server must retain final award authority

Client-side signing is still valuable for:

- proving continuity
- reducing claim tampering
- giving the server a structured verification target

But it should support claims, not replace authoritative award policy.

## Immediate Implementation Consequences

Near-term code changes should follow this model:

1. direct player self-award of achievements should be removed
2. direct player self-grant of entitlements should be removed
3. award/grant endpoints should require trusted-service authorization
4. a separate achievement-claim path can be introduced later
5. entitlements should remain server-issued only

## Open Questions

These are still unresolved and should be decided explicitly in follow-up notes:

- what exact proof format an offline claim should use
- whether claim verification is purely policy-based or includes attestation
- how device keys are provisioned and rotated
- how claim replay and deduplication are stored
- whether claim logs themselves should be ledgered or kept separate from awards

## Summary

The intended model is:

- players submit claims
- the service issues awards
- entitlements remain server-issued only

Offline play is supported through delayed claim submission, not through direct
offline self-awarding.
