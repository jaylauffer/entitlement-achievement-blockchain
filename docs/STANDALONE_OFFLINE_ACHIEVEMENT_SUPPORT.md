# EAB Stand-Alone Offline Achievement Support

Status: first embedded-offline slice implemented

Purpose: define how a single-player game uses EAB while entirely offline and
how the same achievement occurrence later continues into an authoritative
online EAB record.

Game developers looking for dependency setup and a copyable Rust lifecycle
should start with the
[`eab-core` integration guide](../eab-core/README.md). This document explains
the design and trust model behind that integration.

Related notes:

- [ACHIEVEMENT_MODEL.md](ACHIEVEMENT_MODEL.md)
- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [STATE_RECONCILIATION_MODEL.md](STATE_RECONCILIATION_MODEL.md)
- [EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [EAB_CLAIM_TRANSPORT.md](EAB_CLAIM_TRANSPORT.md)

## Core decision

Stand-alone support is an embedded EAB mode, not a local-only workaround.

An EAB-enabled game links `eab-core` and creates a native EAB offline record at
the moment the game acknowledges an accomplishment. That record already has:

- the registered definition identity and digest
- a stable local award id
- a stable online claim id
- save, installation, session, and sequence provenance
- local earned and recorded times
- optional evidence
- an integrity hash

When connectivity or account support exists, the game submits the claim from
that same record. It must not manufacture a separate online achievement or
replace the original claim id.

The achievement therefore gains progressively stronger acknowledgements:

1. the game observed the event
2. the embedded EAB runtime created a game-scoped local acknowledgement
3. the EAB service acknowledged it at account scope
4. qcoin optionally proved the ordered EAB acknowledgement

These are layers on one occurrence, not unrelated local and online awards.

## What remains local and what is shared

The shipped game contains:

- `eab-core`
- packaged, versioned achievement definitions
- game events and the local evaluator call
- an offline EAB record store
- optionally, the unprivileged EAB sync client

The shipped game does not contain:

- developer or trusted-service credentials
- the EAB HTTP service
- an EAB node
- qcoin
- `loadngo` networking
- authority to issue externally meaningful entitlements

The local EAB record is authoritative for the game's own player experience. It
is a claim with provenance when presented to an account-level EAB authority.

## Architecture

```text
Single-player gameplay event
          |
          v
  embedded eab-core evaluator
          |
          +-- immediate local EAB acknowledgement/receipt
          |
          +-- durable immutable EAB record
                         |
                         v
                 optional sync adapter
                         |
                         v
               EAB claim acknowledgement
                         |
                         v
                optional qcoin anchor
```

The game can stop at the durable record forever and remain a complete
stand-alone experience. Adding synchronization later continues the existing
record's lifecycle.

## Trust scopes

### Game scope

The unmodified game trusts its own evaluator for local UI, progression, and
save behavior. It may immediately show and retain the achievement.

This does not make the record cheat-proof. A player controls the executable
and storage environment.

### Account scope

EAB resolves the registered definition and applies account policy before
creating an authoritative account acknowledgement. Ordinary one-time
single-player achievements may be accepted automatically. Competitive or
economically meaningful outcomes may require stronger evidence or online
observation.

### Public proof scope

Only an authoritative EAB acknowledgement whose definition permits public
proof is eligible for qcoin anchoring. Raw evidence and local gameplay history
remain outside qcoin.

## Shared achievement definition

The server and embedded runtime now use the same `eab_core::AchievementDefinition`.
It contains:

- identity:
  - `developer`
  - `game`
  - `achievement_id`
  - `version`
- presentation:
  - `name`
  - `description`
  - `category`
- policy:
  - `visibility`
  - `repeatability`
  - `issuance_mode`
- accomplishment:
  - `summary`
  - `event_key`
  - `threshold`
  - `requires_evidence`

Definitions are packaged game data. Concrete product achievements do not
belong in the shared runtime code.

The embedded runtime computes a deterministic SHA-256 `definition_digest` over
the serialized structured definition and stores it in the offline record. The
canonical online acknowledgement path compares that digest with the registered
definition version before acknowledgement.

## Implemented embedded function

The first API is:

```rust
record_offline_achievement(
    storage,
    definition,
    event,
    context,
) -> Result<OfflineAwardOutcome, OfflineAchievementError>
```

It performs the following work:

1. validates definition and local context identifiers
2. matches the event against `accomplishment.event_key`
3. checks the event value against `accomplishment.threshold`
4. enforces the current one-time evaluator boundary
5. deduplicates once-per-player awards across definition versions
6. generates stable local award and claim UUIDs
7. computes the definition digest and record integrity hash
8. durably appends the record through `OfflineAchievementStorage`
9. returns the stored record as the immediate game-scoped receipt

Possible non-error outcomes are:

- `NoMatchingEvent`
- `ThresholdNotMet`
- `Awarded`
- `AlreadyAwarded`

`AlreadyAwarded` returns the original record and claim id. This makes repeated
game events and restart retries idempotent.

## Example

```rust
use eab_core::{
    record_offline_achievement, AchievementAccomplishment,
    AchievementDefinition, FileOfflineAchievementStorage,
    OfflineAchievementContext, OfflineAchievementEvent,
};

let definition = AchievementDefinition::new(
    "pudding",
    "solo-flight",
    "first-flight",
    1,
    "First Flight",
    "Complete a successful flight",
)
.with_accomplishment(AchievementAccomplishment {
    summary: "Complete one successful flight".into(),
    event_key: Some("flight_completed".into()),
    threshold: Some(1),
    requires_evidence: false,
});

let event = OfflineAchievementEvent {
    event_key: "flight_completed".into(),
    value: 1,
    occurred_at: "2026-08-05T12:00:00Z".into(),
    evidence: None,
};

let context = OfflineAchievementContext {
    local_player_id: "player-slot-1".into(),
    save_id: "save-1".into(),
    installation_id: "installation-1".into(),
    session_id: "session-1".into(),
    client_sequence: 7,
    game_build: "1.0.0".into(),
};

let mut storage =
    FileOfflineAchievementStorage::open("offline-achievements.jsonl")?;
let outcome = record_offline_achievement(
    &mut storage,
    &definition,
    &event,
    &context,
)?;
```

The game should present an `Awarded` result only after the storage append
succeeds. A persistence error must not be displayed as a durable unlock.

## Offline record

The implemented `OfflineAchievementRecord` contains:

```text
schema_version
local_award_id
claim_id
developer
game
achievement_id
version
definition_digest
local_player_id
save_id
installation_id
session_id
client_sequence
earned_at_local
recorded_at_local
game_build
event_key
event_value
evidence
claim_readiness
local_record_hash
```

Identifier behavior:

- `local_award_id` identifies the immutable game-scoped acknowledgement.
- `claim_id` identifies the same occurrence during online submission and
  retry.
- `save_id` survives ordinary save copy and restore.
- `installation_id` distinguishes installations without exposing a raw
  hardware identifier.
- `session_id` and `client_sequence` provide ordering without trusting the
  device clock.
- copied saves preserve all existing award and claim ids.

The current first evaluator treats a logical achievement as once per local
player across all versions. Updating definition presentation or criteria does
not accidentally award the same logical achievement again.

## Local award versus claim readiness

Whether the game may acknowledge an accomplishment locally is different from
whether the record is ready for online claim review.

The record therefore carries `OfflineClaimReadiness`:

- `ready`
- `not_allowed_by_issuance_policy`
- `missing_required_evidence`

A matching event still creates a local EAB record when:

- the definition is `direct_award_only`, or
- required online evidence is missing

Those conditions block conversion to an online claim; they do not erase or
prevent the game-scoped acknowledgement.

`EabClaimEnvelope::try_from(&record)` and the game SDK claim transport refuse
records that are not `ready` before transmission.

## Storage

`eab-core` exposes `OfflineAchievementStorage` so a game can integrate EAB
records with its existing save transaction boundary.

The first slice includes:

- `MemoryOfflineAchievementStorage` for tests and transient use
- `FileOfflineAchievementStorage` as a single-writer JSON-lines reference
  implementation

The file implementation:

- appends immutable records
- flushes and synchronizes a successful append
- reloads records after restart
- rejects duplicate award or claim identities
- verifies every record's integrity hash on load
- fails closed on malformed or tampered records

The integrity hash detects corruption and unsophisticated modification. It is
not an anti-cheat boundary because a player-controlled executable can
recalculate unkeyed hashes.

Production games may implement the storage trait over their normal save system
so game progression and the EAB record share one durability boundary. The
reference file store should not be opened by multiple writers concurrently.

## Continuing online

For a game using the embedded runtime, online continuation is:

1. the player explicitly links a local player slot to an EAB account
2. the sync adapter selects claim-ready offline records
3. it wraps each complete record in a versioned `EabClaimEnvelope` without
   changing `claim_id`
4. it persists upload/reconciliation state separately from the immutable record
5. it submits idempotently
6. EAB resolves the registered definition and evaluates policy/provenance
7. EAB attaches an account acknowledgement or a structured rejection/conflict
8. optional qcoin anchoring begins only after EAB acknowledgement

Network failure never blocks local play. A timed-out request is retried using
the same claim id.

The local record remains the game-scoped acknowledgement if EAB rejects the
online claim. The game may display local and account status separately.

## Claim transport boundary

Game sync code targets `EabClaimTransport`, not HTTP directly. The transport
owns endpoint selection, player/session binding, authentication, and wire
behavior while preserving the immutable record's claim id.

The intended local-network adapter uses IPv6 multicast for discovery only,
then authenticated direct unicast for private claim work. See
[EAB_CLAIM_TRANSPORT.md](EAB_CLAIM_TRANSPORT.md).

### Current canonical HTTP adapter

`eab-game-sdk` re-exports the core types. `HttpEabClaimTransport` validates a
claim-ready record, wraps the complete immutable record in the canonical
envelope, and returns the transport-neutral authority acknowledgement:

```rust
let transport = client.claim_transport(player_id, player_token);
let acknowledgement = transport.submit_claim(&record)?;
```

The canonical envelope preserves:

- the complete `OfflineAchievementRecord`
- definition identity, version, and digest
- local award and claim ids
- local player, save, installation, and session provenance
- client sequence and local award time
- game build, event/value, and optional evidence
- readiness and the local record integrity hash

The authenticated online account binding remains deliberately outside the
client-controlled envelope and is supplied by the transport session.

The HTTP routes are:

```text
POST /profiles/{id}/achievement-claim-envelopes
GET  /profiles/{id}/achievement-claims/{claim_id}/acknowledgement
```

The original thin `SubmitAchievementClaimRequest` path remains available as a
legacy pending/manual-review path. New embedded-offline integrations should use
`EabClaimTransport` and the canonical envelope path.

The authority now validates envelope integrity/readiness, resolves the
registered definition, checks its digest and accomplishment policy, and
returns a structured acknowledged/rejected/conflict result. This establishes
the semantic contract independently of whether HTTP or authenticated loadngo
unicast carries it.

## Historical import

Historical import is a separate compatibility path for games that shipped
before the embedded EAB runtime existed.

Those saves do not contain native claim ids, definition digests, or session
provenance. An importer may create EAB claims from them, but it must label the
provenance as historical rather than pretending the records were created by
the embedded runtime at accomplishment time.

New EAB-enabled games should not use the historical-import path for ordinary
offline play.

## Assurance and provenance

EAB should retain how an account acknowledgement was established. Candidate
assurance classes are:

- `historical_self_asserted`
- `embedded_eab_continuity`
- `client_evidence_reviewed`
- `platform_attested`
- `server_observed`

These describe provenance rather than player honesty. Product policy can use
them to determine eligibility for leaderboards, transferable rewards, or public
proof.

Ordinary single-player achievements can usually accept
`embedded_eab_continuity`. Economic entitlements and competitive outcomes
should require stronger authority.

## Save copies, reinstalls, and clocks

- A copied save preserves `save_id`, `local_award_id`, and `claim_id`.
- A new save receives a new `save_id`.
- A reinstall may create a new `installation_id` while restored records keep
  their original identities.
- EAB deduplicates retries by claim id and one-time awards by account plus
  logical achievement identity.
- `earned_at_local` is a claimed device time.
- EAB separately records first-observed and acknowledged times.
- Session sequence is more trustworthy for ordering than an offline wall
  clock, but is still client-originated provenance.

## Privacy

- Local history remains local unless the player enables synchronization.
- Account linking should explain what is uploaded.
- Evidence remains private EAB service data.
- Installation ids must be random/pseudonymous, not hardware fingerprints.
- Public proof contains only the minimum proof-bearing acknowledgement data.
- Raw evidence and detailed gameplay telemetry never belong in qcoin.

## Entitlements

The embedded runtime may model local story or gameplay unlocks, but it cannot
self-issue externally meaningful entitlements.

Platform purchases, transferable items, account currency, and interoperable
access rights still require a platform, publisher service, or authoritative
EAB grant. Trusted-service credentials must never be embedded in the game.

## Implemented tests

The embedded and online-continuation tests cover:

- a matching event creates a native offline EAB record
- a below-threshold event writes nothing
- one-time evaluation is idempotent across definition versions
- missing required evidence preserves the local record but blocks claim readiness
- direct-award-only policy preserves the local record but blocks claim readiness
- the exact claim id survives file storage restart
- retry after restart returns the existing record
- tampered file records fail integrity verification
- SDK conversion preserves the original claim identity and ordering fields
- canonical envelope round-trip and full-record preservation
- tampered and non-ready envelope rejection before transport
- automatic authority acknowledgement and award creation
- exact idempotent result across retry and service restart
- claim-id payload conflicts and definition digest/not-found conflicts
- once-per-account deduplication across distinct offline occurrences
- HTTP submission plus exact claim-id status reconciliation

## Current limitations and next work

The first slice intentionally supports only one-time achievements. Next work:

1. Add a persistent reconciliation/outbox state store separate from immutable
   records.
2. Add an explicit assurance/provenance classification to authoritative
   acknowledgements when product policy requires it.
3. Add bounded batch submission; exact per-claim status reconciliation already
   exists.
4. Define repeatable occurrence identity before supporting repeatable awards.
5. Add crash-tail recovery or an atomic game-save storage adapter for products
   that cannot fail closed on a partial JSON-lines tail.
6. Add optional stronger evidence/attestation without treating client keys as
   cheat-proof authority.
7. Define authenticated/confidential loadngo unicast and implement a second
   adapter against the same envelope and acknowledgement contract.

Qcoin proof is deliberately not part of the next embedded-client slice. It
remains downstream of authoritative EAB acknowledgement.

## Acceptance criteria

Embedded stand-alone EAB support is viable when:

- the game awards and reloads achievements with no service dependency
- the stored record is a native EAB occurrence with its final claim identity
- restart and retry cannot duplicate a one-time award
- a later sync adapter submits the original claim id
- online rejection cannot erase the game-scoped acknowledgement
- no privileged credential exists in the client
- the shared core has no Actix, qcoin, or `loadngo` dependency
- the service validates the richer offline provenance before automatic
  acknowledgement
