# EAB Claim Transport

Status: canonical envelope, transport-independent authority acknowledgement,
and HTTP adapter implemented; loadngo claim adapter not yet implemented

Purpose: define how an immutable offline EAB achievement record moves from a
stand-alone game to an account-level EAB authority without making HTTP the
architectural contract or misusing multicast for private player traffic.

Related notes:

- [STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md](STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md)
- [EAB_TRANSPORT_DESIGN_GOALS.md](EAB_TRANSPORT_DESIGN_GOALS.md)
- [EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [SIGNED_SERVICE_REQUESTS_ROADMAP.md](SIGNED_SERVICE_REQUESTS_ROADMAP.md)

## Decision

Offline EAB record creation is transport-neutral.

The game creates and persists `OfflineAchievementRecord` through `eab-core`.
When it later synchronizes, it hands that same record to an
`EabClaimTransport`. The transport owns:

- endpoint selection
- player/account binding
- authentication/session state
- wire encoding
- request/response correlation
- retryable transport errors

The transport must preserve the record's original `claim_id`.

HTTP is one compatibility implementation. The target local-network
implementation uses IPv6 multicast only for discovery, followed by direct,
authenticated unicast for claim submission and reconciliation.

## Implemented trait

The game SDK defines:

```rust
pub trait EabClaimTransport: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    fn submit_claim(
        &self,
        record: &OfflineAchievementRecord,
    ) -> Result<EabClaimAcknowledgement, Self::Error>;

    fn claim_status(
        &self,
        claim_id: &str,
    ) -> Result<Option<EabClaimAcknowledgement>, Self::Error>;
}
```

This is deliberately a small boundary:

- `submit_claim` idempotently presents one immutable local record
- both methods return a transport-neutral `EabClaimAcknowledgement`, never an
  HTTP response model
- `claim_status` reconciles the exact authoritative outcome after a timeout,
  restart, or later poll
- transport authentication is configured on the adapter, not passed with
  every local achievement call
- discovery is adapter construction/bootstrap work, not part of claim
  semantics
- `eab-core` has no networking dependency

The first trait is synchronous because the current HTTP SDK and game-facing
call sites are synchronous. A background game worker can own it. If a future
async API is needed, it should be added without changing the claim record or
wire semantics.

## HTTP compatibility adapter

`HttpEabClaimTransport` implements the trait over the current REST API.

It owns:

- an `EabClient`
- one EAB `player_id`
- the corresponding player session token

Construction:

```rust
let transport = client.claim_transport(player_id, player_token);
```

Submission and exact-id reconciliation:

```rust
let acknowledgement = transport.submit_claim(&offline_record)?;
let same_result = transport.claim_status(&offline_record.claim_id)?;
```

Before HTTP submission, the adapter:

1. verifies the offline record integrity hash
2. checks `OfflineClaimReadiness`
3. wraps the complete record in `EabClaimEnvelope`
4. submits that envelope without dropping provenance or integrity fields

`claim_status` calls the exact claim-id acknowledgement endpoint. It does not
download other evidence-bearing claims.

The adapter proves HTTP is behind the transport boundary; it does not establish
HTTP as the final protocol.

## Canonical claim envelope

`EabClaimEnvelope` is the versioned, transport-neutral submit payload:

```rust
pub struct EabClaimEnvelope {
    pub schema_version: u32,
    pub record: OfflineAchievementRecord,
}
```

The complete immutable record is preserved, including definition digest,
local award and claim identities, save/install/session provenance, sequence,
game build, evidence, readiness, and integrity hash.

The authenticated EAB `player_id` is intentionally outside this envelope. The
HTTP adapter derives it from the player session; another transport must provide
an equivalent authenticated binding. A client-controlled local slot or save id
must never select the authoritative account.

## Transport-independent acknowledgement

The authority service method accepts three inputs independent of any wire:

```text
authenticated player id
canonical claim envelope
locally resolved authoritative definition, if present
```

It returns `EabClaimAcknowledgement`, containing:

- the claim and definition identity
- `pending`, `acknowledged`, `rejected`, or `conflict` disposition
- a stable machine-readable decision code
- first-observed and decision timestamps
- an optional authoritative award transaction/block reference

Current automatic decisions include successful acknowledgement, already
acknowledged, invalid/non-ready envelope, claim-id payload mismatch, missing or
changed definition, disallowed issuance mode, missing evidence, event or
threshold mismatch, and unsupported repeatability.

The authority resolves policy from its registered definition and verifies the
record's definition digest. The client does not supply authoritative policy at
submission time.

The HTTP adapter exposes this core operation as:

```text
POST /profiles/{id}/achievement-claim-envelopes
GET  /profiles/{id}/achievement-claims/{claim_id}/acknowledgement
```

The original thin `POST /achievement-claims` endpoint remains a compatibility
path that creates a pending claim for manual/trusted-service review. It is not
the canonical embedded-offline path.

## Existing IPv6 multicast layer

The EAB node service already runs on `loadngo/network` and defaults to the
link-local IPv6 multicast group:

```text
ff02::4541:4200:1
```

Current node-plane messages are:

- multicast `PresenceAnnounce`
- direct `NodeInfo`
- direct `StatusRequest` / `StatusResponse`
- prototype direct `AchievementAwardRequest` / `AchievementAwardResponse`

Current `NodeInfo` advertises:

- wire compatibility versions
- software version
- node name
- optional HTTP base URL
- capabilities

The current achievement-award message sends a full definition and is a lab
prototype. It is not suitable for the game claim transport because:

- it represents an authoritative award rather than a player claim
- it treats caller-supplied definition data as authority
- it does not implement the offline-record provenance contract
- it does not provide the required retail-client authentication and privacy
  boundary

## Multicast responsibilities

Multicast may carry only low-rate, non-private discovery information:

- presence
- protocol versions
- service capability names
- a direct reply address
- an authority/key fingerprint when the trust model is defined

Multicast must not carry:

- achievement claims
- player or account identifiers
- access/session tokens
- gameplay evidence
- receipts
- entitlement or reward mutations
- detailed player state

Reasons:

- delivery is not reliable
- every listener on the link may observe the packet
- there is no single deterministic authority recipient
- multicast amplification must remain bounded
- request correlation and retry semantics belong in unicast

## Target loadngo flow

The planned `LoadngoEabClaimTransport` should behave as follows.

### 1. Bootstrap

The adapter starts with one or more of:

- the embedded IPv6 multicast group for same-link discovery
- a statically configured EAB endpoint
- a previously trusted endpoint
- DNS or a rendezvous/bootstrap service for Internet use

`ff02::` multicast is link-local and normally does not cross routers. It cannot
serve as general Internet discovery.

### 2. Discover capabilities

The adapter emits or listens for low-rate presence and receives `NodeInfo`
directly. A claim-capable node should eventually advertise a versioned
capability such as:

```text
achievement-claim-v1
```

Capability discovery says what a node claims to support. It does not establish
that the node is trusted.

### 3. Establish authority and session

Before sending player data, the adapter must:

- verify the selected EAB authority against configured trust material
- bind the transport session to the intended player/account
- negotiate an authenticated and confidential unicast session
- obtain replay-resistant session/request state

Multicast discovery must never be sufficient authority by itself. Otherwise a
hostile LAN participant could advertise a fake EAB node and collect player
claims or credentials.

The exact player-session cryptographic handshake is still to be designed. The
existing HTTP bearer token must not simply be broadcast or copied into a
plaintext UDP payload.

### 4. Submit over direct unicast

The adapter sends a versioned claim request containing:

- request id
- player/session binding or authenticated session reference
- definition identity and version
- definition digest
- original local award and claim ids
- save/install/session provenance appropriate for policy
- local sequence and claimed time
- game build
- evidence or evidence digest when permitted
- local record integrity material

The authoritative EAB node resolves the registered definition locally. The
request does not contain caller-controlled reward policy.

### 5. Correlate the response

The response should distinguish:

- accepted as pending
- already known/idempotent replay
- acknowledged
- rejected by policy
- definition conflict
- authentication/authorization failure
- malformed request
- retryable service failure

Responses correlate by request id and claim id. A timeout leaves the claim
outcome unknown; it does not authorize generation of a new claim id.

### 6. Reconcile status

After timeout or restart, the adapter queries the exact claim id over unicast.
It must not require downloading all evidence-bearing claims merely to learn one
status.

## Authentication and privacy requirements

The loadngo claim adapter is not usable for retail player traffic until it has:

- authenticated EAB authority selection
- confidential unicast payload protection
- player/account session binding
- replay protection
- request and claim idempotency
- bounded payload sizes
- evidence minimization
- explicit error classification
- safe key/session rotation

Client signatures alone do not make an offline accomplishment cheat-proof.
They may prove continuity and protect a request in transit, while EAB retains
the account-level acknowledgement decision.

Trusted-service credentials are not part of `EabClaimTransport`. The trait is
for player-originated claims only, never direct entitlement grants or
authoritative publisher awards.

## Endpoint selection policy

If discovery returns multiple claim-capable nodes, selection should be
deterministic and policy-driven. Candidate inputs include:

- trusted authority identity
- namespace/game support
- compatible wire version
- session/account home node
- configured priority
- recent reachability

Lowest latency alone must not override the trusted authority or account-home
policy.

## Idempotency

The offline `claim_id` is the logical idempotency identity across all transport
adapters.

- HTTP and loadngo submission of the same record must converge on one claim.
- A transport timeout is retried with the same claim id.
- Switching transports must not create a new claim.
- The service returns the existing status for duplicate submission.
- Once-per-account award policy is enforced separately from transport-level
  claim deduplication.

## Error model

Transport implementations should distinguish at least:

- invalid/tampered local record
- record not ready for online claim submission
- no trusted endpoint available
- authentication/session failure
- incompatible protocol or capability
- timeout/outcome unknown
- retryable network failure
- authoritative policy rejection
- definition conflict
- malformed request

The current `SdkError` remains HTTP-oriented. A future shared claim sync layer
should introduce structured transport/reconciliation errors without flattening
authoritative rejection into a network failure.

## Testing requirements

Common contract tests should run against every implementation:

- the submitted claim id equals the stored offline claim id
- non-ready and tampered records are refused before transmission
- duplicate submission is idempotent
- timeout followed by status lookup recovers the known result
- adapter authentication state is not part of the offline record
- authoritative rejection remains distinct from transport failure

Current transport tests cover:

- exact claim-id preservation through a transport implementation
- claim-id status reconciliation through the trait
- player binding owned by the HTTP adapter
- refusal of a non-ready record before HTTP I/O
- complete-envelope serialization and tamper detection
- automatic acknowledgement and authoritative award creation
- byte-for-byte equivalent acknowledgement on idempotent retry and restart
- claim-id/payload conflict detection
- authoritative definition missing/digest conflicts
- once-per-account deduplication across distinct offline occurrences
- HTTP submit plus exact-id acknowledgement reconciliation

Future loadngo tests must also cover:

- multicast discovery with no player data in multicast frames
- direct `NodeInfo` capability negotiation
- rejection of an untrusted discovered node
- authenticated unicast claim submission
- request correlation and timeout
- exact claim-id status reconciliation
- link-local discovery plus static/remote bootstrap behavior

## Implementation sequence

1. Keep `eab-core` transport-free.
2. Use `EabClaimTransport` at game sync call sites.
3. Retain `HttpEabClaimTransport` for compatibility and current integration
   testing.
4. Reuse the canonical envelope and acknowledgement in versioned loadngo
   submit/status wire messages.
5. Add the authority/session handshake and payload protection.
6. Advertise `achievement-claim-v1` only when the node can safely serve it.
7. Implement `LoadngoEabClaimTransport` with multicast bootstrap and unicast
   work.
8. Run common behavior tests against HTTP and loadngo adapters.
9. Remove any assumption that a discovered HTTP URL is the only game-facing
   route.

## Current boundary

The canonical semantic contract, authority method, trait, and HTTP adapter
exist now. They intentionally make no claim that the current UDP award
prototype is safe or complete for player claims.

Until authenticated claim unicast exists:

- embedded offline EAB remains fully functional
- HTTP remains the available online compatibility adapter
- multicast remains node discovery/status infrastructure
- games must not send private claim data through the prototype node award path
