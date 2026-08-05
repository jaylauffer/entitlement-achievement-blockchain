# EAB UDP Transport Implementation Plan

Status: Phase 0 complete; Phase 1 in progress

Purpose: make the EAB UDP service plane a production-capable transport for
player claims, trusted-service operations, node status, and reconciliation
without moving private data onto multicast or duplicating authority policy in
each adapter.

Related documents:

- [EAB_CLAIM_TRANSPORT.md](EAB_CLAIM_TRANSPORT.md)
- [EAB_TRANSPORT_DESIGN_GOALS.md](EAB_TRANSPORT_DESIGN_GOALS.md)
- [EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [SIGNED_SERVICE_REQUESTS_ROADMAP.md](SIGNED_SERVICE_REQUESTS_ROADMAP.md)
- [EAB_UDP_PROTOCOL_DECISIONS.md](EAB_UDP_PROTOCOL_DECISIONS.md)

## Executive decision

The recommended transport shape is:

```text
IPv6 multicast UDP
    discovery only
          |
          v
trusted endpoint selection
          |
          v
authenticated QUIC unicast over UDP
    private EAB request/response and streams
          |
          v
transport-independent EAB runtime
```

QUIC remains UDP, but supplies the pieces that the current raw datagram layer
does not: authenticated encryption, reliable delivery, ordering where needed,
packetization, congestion control, connection migration, and multiple logical
streams.

Building those features directly over raw datagrams is possible, but it would
mean implementing and securing a custom transport protocol. That alternative
must not proceed without an explicit architecture decision and equivalent
acceptance criteria. Custom cryptography is not acceptable.

Multicast remains intentionally unauthenticated and low-trust. Discovery may
identify a candidate endpoint and authority certificate fingerprint; it never grants
trust or carries player data.

## Current baseline

Before consolidation, `rust/src/eab_node.rs` had:

- embedded IPv6 link-local multicast discovery at `ff02::4541:4200:1`
- optional static peers
- multicast `PresenceAnnounce`
- direct `NodeInfo`
- direct `StatusRequest` / `StatusResponse`
- a prototype direct `AchievementAwardRequest` / response
- a shared `loadngo-proactor` runtime pump
- loopback discovery, status, and award tests

That EAB1/JSON protocol has now been removed. The live node uses the bounded
`eab-wire` discovery exchange and the shared `loadngo-proactor` runtime pump.
Detailed status and private mutations will return only on authenticated secure
unicast contracts; they are not retained as raw-UDP compatibility messages.

The claim vertical slice now has authority authentication, encrypted TLS 1.3
payloads, transitional player-session binding, QUIC delivery/congestion
behavior, canonical submit/status messages, correlated acknowledgements, and
structured protocol errors. It does not yet have:

- automatic persistent authority startup tied to discovery advertisement
- player identity exchange, refresh, revocation, or logout over QUIC
- durable request-id replay prevention beyond claim-id idempotency
- persistent client retry/outcome-unknown recovery
- connection reuse and principal/source connection quotas
- broader role/capability negotiation and enforcement
- trusted-service signature verification
- key rotation/revocation distribution
- fault-injection and supported-platform coverage

`loadngo_network::send_frame_with_retries` retries a failed local socket send.
It does not retry a datagram that was lost after `sendto` succeeded and does not
prove the peer received or processed it.

The removed prototype award request sent a full caller-controlled achievement
definition and accepted an unauthenticated `player_id`. It is no longer a
runtime or wire capability.

## Definition of fully functional

The UDP transport is fully functional only when all required roles below use
the same transport-independent runtime policy as HTTP and pass common contract
tests.

### Player/game role

Required for the first production gate:

- discover or configure a trusted EAB authority
- establish an authenticated, confidential session
- bind that session to an EAB player/account
- submit `EabClaimEnvelope`
- receive `EabClaimAcknowledgement`
- query exact status by `claim_id`
- retry after timeout without creating a new claim
- read the authenticated player's reward state
- distinguish transport failure from authoritative rejection or conflict

Required before UDP can replace HTTP completely for player traffic:

- identity-provider token exchange over the encrypted channel
- profile creation and read
- allowed player-owned profile/concept mutations
- credential refresh, revocation, logout, and account relinking

### Trusted-service role

Required before authoritative service traffic can move off HTTP:

- register achievement definitions
- register entitlement definitions
- issue achievement awards by registered definition reference
- grant entitlements by registered definition reference
- review claims where manual/trusted review policy applies
- enforce developer/game namespace and least-privilege scopes
- verify canonical post-quantum signed requests
- persist nonce/request-id replay protection and key revocation state

Trusted-service requests may contain a full definition only when the operation
is definition registration. Award and grant operations reference definitions
already registered at the authority.

### Node/operator role

Required:

- minimal multicast presence
- direct version and capability negotiation
- trusted node identity
- sanitized public health and authenticated detailed status
- qcoin anchor lifecycle visibility
- bounded rate and amplification behavior
- static/DNS bootstrap when link-local multicast is unavailable

EAB state replication, EAB consensus, and multi-writer ledger semantics are not
implied by transport completeness. They require separate designs.

## Protocol boundaries

### Multicast discovery plane

Allowed fields:

- protocol version range
- ephemeral discovery request id
- node id
- direct UDP/QUIC endpoint
- authority certificate fingerprint
- capability identifiers or a capability digest
- short expiry

Forbidden fields:

- player or account identity
- session or bearer tokens
- claims, evidence, receipts, or rewards
- entitlement state
- detailed operational state

Discovery responses must be rate-limited, small, and no larger than the request
unless the requester first proves address reachability. IPv6 multicast remains
link-local; remote deployments use configured endpoints, DNS, or rendezvous.

### Secure unicast plane

Every connection negotiates:

- wire and schema version
- authenticated authority identity
- client role: player, trusted service, node, or operator
- capability set
- maximum request/evidence sizes
- idle timeout and keepalive policy

Every application request carries:

- message type and schema version
- unique `request_id`
- correlation id where applicable
- authenticated principal from the session, not the payload
- optional operation id such as `claim_id`
- bounded canonical payload

Every response carries:

- the original `request_id`
- operation identity where applicable
- success or a structured error code
- retry classification
- server-observed timestamp

The source IP address is routing information, never authorization identity.

## Wire contracts

Create a shared workspace crate, provisionally `eab-wire`, containing only
versioned protocol types, bounded encoding/decoding, and golden test vectors.
It may depend on `eab-core` domain contracts but not on Actix, ledger storage,
or server runtime code.

Initial application messages:

```text
ClientHello / ServerHello
IdentityExchangeRequest / Response
CreateProfileRequest / Response
GetProfileRequest / Response
GetRewardsRequest / Response
SubmitClaimRequest { request_id, envelope }
SubmitClaimResponse { request_id, acknowledgement }
ClaimStatusRequest { request_id, claim_id }
ClaimStatusResponse { request_id, acknowledgement? }
NodeStatusRequest / Response
ProtocolErrorResponse
```

Trusted-service messages follow after the player claim slice:

```text
RegisterAchievementDefinition
RegisterEntitlementDefinition
AwardAchievementByReference
GrantEntitlementByReference
ReviewClaim
```

Do not expose Rust enum layout or unconstrained Serde JSON as the permanent
wire contract. The protocol decision record must select a deterministic,
cross-language encoding, define numeric message identifiers, set maximum field
sizes, and publish golden byte vectors.

Control packets should fit within a conservative path-MTU budget. Claims and
evidence that exceed that budget use a QUIC stream with an explicit total-size
limit and content digest. If raw datagrams are chosen instead, authenticated
fragmentation, reassembly timeouts, memory quotas, missing-fragment recovery,
and congestion behavior become mandatory deliverables.

## Runtime and authorization refactor

UDP must not reproduce policy currently embedded in Actix handlers.

Introduce a transport-independent application gateway over `EabRuntime` that
owns:

- identity/session validation hooks
- profile ownership checks
- authoritative registry lookup
- claim acknowledgement
- trusted-service namespace/scope checks
- definition registration
- achievement/entitlement mutation by reference
- structured domain errors

HTTP and UDP adapters both translate their transport credentials and messages
into this gateway. Common tests run the same operations through both adapters
and compare domain results.

In particular, move authoritative achievement-definition resolution out of the
HTTP claim handler and behind the runtime boundary. The current canonical
claim service method is already transport-independent, but the HTTP adapter
still loads the registry before invoking it.

## Identity and cryptography

### Authority and channel identity

- EAB authorities have durable keys and stable fingerprints.
- Configured trust roots or pinned fingerprints decide whether a discovered
  endpoint is acceptable.
- Discovery claims are verified only after the secure unicast handshake.
- Certificate/key rotation supports overlap, expiry, and revocation.
- Missing trust configuration fails closed for private capabilities.

The QUIC/TLS library and certificate model require an architecture decision.
Use a maintained implementation and document its algorithm and platform
support. If post-quantum confidentiality is required at this stage, select a
reviewed hybrid/PQ-capable TLS or KEM implementation; the existing repository
contains PQ signatures but no production KEM/session-encryption layer.

### Player identity

The transitional path may exchange the existing player session credential
inside the encrypted channel. It must never appear in discovery or plaintext
UDP. The complete path adds encrypted identity exchange and refreshable UDP
session credentials so HTTP is not required for bootstrap.

The authenticated session supplies `player_id`. Client-controlled claim,
save, and installation fields cannot select the destination account.

### Trusted-service identity

Use canonical signed service requests with:

- signature scheme and issuer key id
- developer/game namespace
- operation and scope
- target player where applicable
- request id/nonce
- issued-at and expiry
- deterministic mutation payload

Reuse the qcoin PQ signature abstraction where practical. The current
`qcoin-crypto` crate supports Dilithium2 and Falcon512 signatures, but signature
reuse does not by itself provide channel encryption.

## Reliability and idempotency

QUIC handles packet delivery; EAB still owns operation semantics:

- `request_id` deduplicates transport-level retries per authenticated principal
- `claim_id` deduplicates the logical offline occurrence across every adapter
- once-per-account policy deduplicates the logical achievement independently
- signed-service nonces prevent mutation replay
- successful mutation responses are cached or reconstructible for a defined
  retention period
- a timeout is `outcome_unknown`, not rejection
- reconnect plus exact-id status lookup recovers the durable result

Client retry policy uses bounded exponential backoff with jitter and a deadline.
The game persists synchronization state separately from immutable offline EAB
records.

## Error model

Define one shared error envelope with stable codes grouped as:

- protocol: malformed, unsupported version, oversized, unsupported capability
- discovery/trust: no endpoint, untrusted authority, identity mismatch
- session: authentication failed, expired, revoked, replayed
- authorization: wrong player, namespace denied, scope denied
- domain: invalid envelope, claim not ready, definition conflict, policy reject
- availability: busy, retry later, timeout/outcome unknown
- internal: persistence failure, unavailable dependency

Authoritative domain decisions continue to use `EabClaimAcknowledgement` and
must not be flattened into transport errors.

## Implementation phases

### Phase 0: safety and architecture decisions

Deliverables:

- remove the full-definition award capability rather than carrying an unused
  compatibility path
- write protocol ADRs for QUIC versus custom datagrams, wire encoding, trust
  roots, certificate lifecycle, and PQ/hybrid requirements
- define payload, evidence, connection, and rate limits
- define supported deployment targets and minimum IPv4/IPv6 behavior

Gate:

- no production configuration can expose the unauthenticated prototype award
- security and wire choices are approved before private messages are added

Progress:

- the unauthenticated full-definition award handler and its public runtime
  entry point have been removed
- QUIC, deterministic CBOR, initial pinned-authority trust, player binding,
  PQ-signature posture, and idempotency decisions are recorded
- authoritative definition lookup for canonical claims now occurs inside
  `EabRuntime`, not the HTTP adapter
- Quinn/rustls is selected for the secure-unicast spike; final platform and
  measured operational limits remain open

Phase 0 is complete for the first claim slice. Deployment-target and final
operational-limit choices remain explicit Phase 1/2 measurements rather than
permission to expose private raw UDP messages.

### Phase 1: versioned discovery and shared wire crate

Deliverables:

- `eab-wire` crate with bounded V2 headers/messages and golden vectors
- V2 discovery carrying endpoint, capability, expiry, and authority fingerprint
- deterministic trusted endpoint selection
- static and DNS bootstrap alongside multicast
- capability names that are advertised only when configured and usable
- active probing by clients and providers rather than an unsolicited provider
  presence protocol

Gate:

- malformed, unknown-version, oversized, and amplified discovery traffic is
  safely rejected
- multicast captures contain no private EAB data
- IPv6 link-local scope ids and multi-interface hosts are tested

Progress:

- the `eab-wire` workspace crate implements the fixed V2 discovery header,
  deterministic CBOR payloads, numeric message ids, and bounded decoding
- probe/challenge/query/response messages make source-reachability proof an
  explicit prerequisite to the larger discovery response
- golden, round-trip, malformed, trailing-data, field-bound, and outer-size
  tests cover the codec
- the running node now uses this protocol exclusively, including keyed,
  source-bound cookie generation and validation
- active multicast discovery is the accepted provider-discovery model;
  provider presence/membership gossip is intentionally out of scope
- authority advertisement fails closed unless both a secure endpoint and its
  32-byte DER certificate fingerprint are configured; no capabilities are
  advertised before their secure services exist
- configured trust-pin parsing and deterministic selection now filter unknown,
  expired, and wire-incompatible candidates; pin order defines preference and
  missing trust fails closed
- broader DNS bootstrap and expanded IPv6 interface testing remain

### Phase 2: authenticated secure unicast

Deliverables:

- QUIC endpoint integrated with `loadngo-proactor`, or an approved equivalent
- authority authentication and pinning/trust roots
- role negotiation and encrypted player/service/node sessions
- connection, stream, request, and payload quotas
- key rotation, expiry, revocation, and fail-closed startup behavior
- structured protocol/session errors and safe observability

Gate:

- untrusted discovery responders cannot receive credentials or claim data
- packet capture exposes no private payload
- replayed handshakes and application requests fail deterministically
- loss, reorder, duplicate packets, MTU variation, and reconnect are tested

Progress:

- the implementation uses Quinn 0.11 with rustls TLS 1.3 and ALPN `eab/2`
- the client verifies the SHA-256 fingerprint of the complete presented DER
  certificate and then delegates certificate validity, hostname, chain, and
  handshake-signature checks to rustls/WebPKI
- the server is initiated through `loadngo-proactor`; a dedicated
  current-thread Tokio worker owns Quinn I/O because Quinn's high-level API is
  async rather than a native loadngo completion source
- authority identities can be loaded from a persistent DER certificate and
  PKCS#8 DER private key; ephemeral generation remains test-only
- loopback tests prove success with the configured pin, handshake failure with
  a wrong pin, and pre-network rejection of oversized requests
- bounded deterministic secure frames now carry encrypted session-bound claim
  submission and exact-id status reconciliation into the existing EAB runtime
- discovery/runtime auto-wiring, broader role/capability negotiation,
  connection quotas, key rotation, revocation, fault injection, and platform
  coverage remain Phase 2 work

### Phase 3: canonical claim vertical slice

Deliverables:

- `SubmitClaim` and exact `ClaimStatus` wire operations
- server adapter calling the existing canonical authority method
- `LoadngoEabClaimTransport` implementing `EabClaimTransport`
- persistent client retry/reconciliation state keyed by `claim_id`
- server response recovery after timeout and restart
- capability `achievement-claim` enabled only when secure sessions and
  registry lookup are operational

Gate:

- the common claim contract suite passes unchanged for HTTP and UDP
- identical HTTP/UDP submission converges on one acknowledgement and award
- tampered/non-ready claims fail before client transmission and again at the
  server boundary
- timeout followed by exact status returns the durable original result
- once-per-account behavior survives distinct claim ids and process restart

This is the first milestone that should be shipped to an offline Rust game.

Progress:

- `eab-wire` implements `SubmitClaim`, `ClaimStatus`, correlated responses, and
  structured protocol errors with deterministic CBOR and a 64 KiB frame bound
- the QUIC authority adapter authenticates the session, derives `player_id`,
  and invokes the transport-independent canonical runtime methods
- `eab-quic-client` and the game SDK's `QuicEabClaimTransport` provide a
  server-crate-independent static-endpoint client
- an end-to-end test submits an offline record, awards the session-bound
  account, reconciles the exact acknowledgement, and rejects an invalid session
- discovery-backed construction, durable client retry state, response recovery
  fault tests, and the full common HTTP/QUIC contract suite remain

### Phase 4: player bootstrap and read parity

Deliverables:

- encrypted identity exchange and session refresh
- profile create/read
- reward-state read
- explicitly allowed player profile/concept operations
- account relinking and credential revocation behavior

Gate:

- UDP can support a new player without first using HTTP
- one player cannot read or mutate another player's state
- reconnect and credential rotation do not duplicate profiles or claims

### Phase 5: signed trusted-service operations

Deliverables:

- issuer-key registry with scheme, namespace, scope, status, and rotation
- persistent nonce/request replay store
- PQ-signed definition registration, award-by-reference,
  entitlement-grant-by-reference, and claim-review messages
- runtime-owned authorization shared by HTTP and UDP
- audit records that identify issuer key and request id without logging secrets

Gate:

- wrong namespace/scope, expired, replayed, revoked, or scheme-mismatched
  requests cannot mutate state
- full definitions are accepted only by registration operations
- players cannot access trusted-service operations
- HTTP and UDP produce equivalent authoritative receipts

### Phase 6: operational hardening and rollout

Deliverables:

- per-message, principal, source, and connection rate limits
- load shedding, backpressure, and bounded queues
- metrics for discovery, handshake, request latency, retry, rejection, session,
  and qcoin anchor lifecycle without evidence or token leakage
- fuzzing and property tests for all decoders and state machines
- soak, load, restart, upgrade, and key-rotation tests
- deployment guide for LAN, static Internet endpoint, and multicast-disabled
  environments
- staged feature modes: disabled, discovery-only, claims, and full service

Gate:

- no unbounded allocation or task growth from unauthenticated traffic
- service recovers from restart with claims, replay state, and anchor state
  intact
- supported desktop/mobile platforms pass the network matrix
- HTTP can remain as compatibility without being required by the UDP path

## Test matrix

### Unit and protocol tests

- golden wire vectors for every versioned message
- encode/decode round trips
- unknown fields/version compatibility
- malformed/truncated/oversized payloads
- request/response correlation
- capability negotiation
- error-code stability
- signature, expiry, nonce, and scope verification

### Fault-injection tests

- packet loss, reorder, duplication, corruption, and delay
- path MTU changes
- response lost after durable mutation
- disconnect during streamed evidence
- authority restart during request
- client restart with pending synchronization state
- stale discovery and endpoint migration

### Integration tests

- IPv4 and IPv6 loopback
- real IPv6 multicast on each supported OS
- multicast disabled plus static/DNS bootstrap
- two nodes and multiple candidate authorities
- hostile discovery responder
- HTTP/UDP cross-adapter idempotency
- file, sled, and qcoin-backed runtime behavior
- qcoin unavailable, accepted, and durably included states

### Security and scale tests

- parser fuzzing and corpus regression
- unauthenticated amplification measurement
- connection and stream exhaustion
- repeated invalid authentication/signatures
- token/key/evidence redaction in logs
- sustained clients, burst claims, and large bounded evidence
- external security review before enabling trusted mutations

## Required decisions before implementation

1. Confirm QUIC-over-UDP for secure unicast, or explicitly fund the much larger
   custom datagram reliability/security scope.
2. Select the deterministic wire encoding and cross-language support target.
3. Define authority trust roots and certificate/key provisioning.
4. Decide whether the first channel must provide PQ/hybrid confidentiality or
   whether PQ signatures plus a documented channel migration are acceptable.
5. Set maximum claim/evidence sizes and retention periods.
6. Define UDP platform targets for the first release.
7. Confirm whether “full EAB” requires concept/vector operations in the first
   parity gate or only identity, profile, rewards, and claims.

None of these decisions block Phase 0 safety work or extracting shared runtime
authorization policy.

## Immediate focus

The next implementation slice should remain narrow and vertical:

Completed foundations:

1. deterministic endpoint selection and configured certificate-pin filtering
2. Quinn/rustls loopback spike initiated through `loadngo-proactor`
3. certificate-pinned TLS 1.3 secure-unicast handshake and bounded test exchange

Next vertical slice:

1. wire persistent authority identity startup into the main runtime and bind
   discovery advertisement to the loaded certificate fingerprint
2. construct `QuicEabClaimTransport` from deterministic trusted discovery
   selection, with static/DNS fallback
3. persist client retry/reconciliation state and fault-test the implemented
   `outcome_unknown` classification
4. pass the full HTTP/QUIC common claim contract and restart/fault suite
5. exercise multicast discovery and static bootstrap against the same secure
   authority path

This order proves the canonical EAB requirement end to end without letting
discovery, full API parity, or trusted-service administration obscure the
first useful deliverable.

## Completion statement

Do not describe the EAB UDP layer as production-capable merely because nodes
discover one another or a loopback award test passes.

It is production-capable when:

- discovery reveals no private data
- endpoint trust is cryptographically verified
- private traffic is confidential and authenticated
- requests survive realistic UDP failure modes
- replay and idempotency are durable
- the canonical claim and service policy remain transport-independent
- authorization is equivalent across HTTP and UDP
- common contract, security, fault, restart, and platform tests pass
