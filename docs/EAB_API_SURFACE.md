# EAB API Surface

Purpose: restate the intended EAB API surface in product terms so future work
does not confuse player acknowledgements, authoritative rewards, and proof
anchoring.

Related notes:

- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [STATE_RECONCILIATION_MODEL.md](STATE_RECONCILIATION_MODEL.md)
- [EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [ACHIEVEMENT_MODEL.md](ACHIEVEMENT_MODEL.md)
- [EAB_TRANSPORT_DESIGN_GOALS.md](EAB_TRANSPORT_DESIGN_GOALS.md)

## What EAB is responsible for

EAB is the authoritative player-facing service.

It owns:

- player profiles
- private progression state
- achievement claims
- authoritative achievement awards
- authoritative entitlement grants
- qcoin anchor initiation for the subset that needs durable proof

Qcoin is not the player-facing source of truth. Qcoin is the proof and ordering
substrate behind EAB.

## Main API categories

### 1. Identity and profile API

Current surface:

- `POST /identity/exchange`
- `POST /profiles`
- `GET /profiles/{id}`
- `POST /profiles/{id}/dimensions`
- `POST /profiles/{id}/concepts`

Purpose:

- establish player identity
- create and read profiles
- update player-owned profile data within allowed bounds

### 2. Achievement definition API

Current surface:

- `POST /achievements`

Purpose:

- register an achievement definition under a developer/game namespace

Important:

- definitions are registry data
- they are not meant to be hard-coded in runtime modules
- callers should register or reference definitions, not embed product truth into
  service code

### 3. Achievement claim API

Current surface:

- `POST /profiles/{id}/achievement-claims`
- `GET /profiles/{id}/achievement-claims`
- `POST /profiles/{id}/achievement-claims/{claim_id}/review`
- `POST /profiles/{id}/achievement-claim-envelopes`
- `GET /profiles/{id}/achievement-claims/{claim_id}/acknowledgement`

Purpose:

- accept player-submitted or client-submitted evidence that an accomplishment
  happened
- keep that evidence non-authoritative until trusted review or direct
  authoritative issuance

The original thin claim is:

- pending
- player/client-originated
- not authoritative rewards by themselves

The canonical envelope path is the embedded-offline continuation surface. It
preserves the complete immutable local EAB record. The authority validates it
against the registered definition and returns a transport-neutral
acknowledgement, rejection, or conflict. An acknowledged claim creates or
references the authoritative once-per-account award.

### 4. Authoritative achievement award API

Current surface:

- `POST /profiles/{id}/achievements`

Purpose:

- append an authoritative achievement award to the EAB ledger for that player

Important:

- this is not a player self-award surface
- this is a trusted-service or authoritative-node mutation surface
- the authoritative node should resolve the registered definition and then issue
  the award

### 5. Entitlement definition API

Current surface:

- `POST /entitlements`

Purpose:

- register durable inventory/access definitions

### 6. Authoritative entitlement grant API

Current surface:

- `POST /profiles/{id}/entitlements`

Purpose:

- issue durable inventory/access/account-rights state

Important:

- entitlements are authoritative service-issued state
- they are higher-risk than ordinary achievements
- offline player clients do not grant them to themselves

## What achievements are

Achievements are acknowledgements of accomplishments.

In EAB terms, an achievement is:

- namespaced by `developer`, `game`, `achievement_id`, and `version`
- described for players by `name` and `description`
- governed by award policy such as:
  - visibility
  - repeatability
  - issuance mode
- backed by structured accomplishment rules

An authoritative achievement award is a durable EAB fact:

- "the service acknowledges this player achieved this accomplishment"

That award may later be mirrored into qcoin if the accomplishment is meant to
have durable proof value.

## What entitlements are

Entitlements are durable rights or assets.

Typical examples:

- access grants
- inventory items
- unlock rights
- bundles
- account-linked economic or interoperability state

They are distinct from achievements because they are closer to inventory and
rights than to acknowledgement.

That is why entitlements remain server-issued or trusted-service-issued only.

## Offline single-player flow for a game like Zhoenus

For an offline-capable single-player game, the intended flow is:

1. the game records local accomplishment events while offline
2. the game stores native immutable EAB occurrences, not authoritative account
   awards
3. when connectivity returns, the client wraps a claim-ready occurrence in an
   `EabClaimEnvelope`
4. the authenticated transport binds it to an EAB account
5. EAB verifies the envelope and registered definition policy
6. EAB returns a structured acknowledgement, rejection, or conflict and, when
   acknowledged, issues or references the authoritative account award
7. if that achievement is proof-worthy, EAB enqueues a qcoin anchor

So the player/game client says:

- "this happened"

EAB decides:

- "this is now authoritative"

and qcoin later proves:

- "this authoritative award was durably ordered"

## What a Zhoenus offline claim should carry

The canonical offline envelope carries enough information for:

- player binding
- developer/game binding
- achievement identity
- idempotency
- ordering within a session
- optional gameplay evidence

Its complete `OfflineAchievementRecord` includes:

- `developer`
- `game`
- `achievement_id`
- `version`
- `claim_id`
- `session_id`
- `client_sequence`
- `claimed_at`
- optional `evidence`
- definition digest
- local award id
- local player/save/installation provenance
- game build and evaluated event value
- readiness and record integrity hash

The authoritative account id is intentionally excluded from client-controlled
record data and supplied by the authenticated transport session.

## Transport posture

Today:

- HTTP is still the main player/trusted-service adapter

Target direction:

- IPv6 multicast for node discovery and low-rate announcements
- unicast for deterministic node-to-node follow-up
- no product-specific achievements hard-wired into runtime code
- no assumption that a request payload defines the authoritative achievement
  policy

The correct long-term posture is:

- definitions are registered state
- claims reference definitions
- authoritative services or nodes acknowledge accomplishments

not:

- runtime code contains game-specific achievements as built-in facts
