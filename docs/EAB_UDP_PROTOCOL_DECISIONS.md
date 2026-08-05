# EAB UDP Protocol Decisions

Status: initial decisions accepted; discovery, configured endpoint selection,
bounded claim/status messages, the Quinn/rustls authority adapter, and the Rust
game client are implemented; production discovery integration and operational
limits remain open

Purpose: record the protocol choices that unblock the first secure EAB UDP
claim transport and keep unresolved security questions visible.

Related plan:

- [EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md](EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md)

## D1: Multicast discovery plus QUIC secure unicast

Decision: accepted

- Raw IPv6 multicast UDP remains discovery-only.
- Private request/response traffic uses authenticated QUIC unicast over UDP.
- Static and DNS endpoints remain available when multicast is unavailable.
- A discovered address is a candidate, not an authority decision.

Reason:

QUIC supplies reliable delivery, authenticated encryption, congestion control,
packetization, connection migration, and multiplexed streams. Implementing
equivalent behavior directly over raw datagrams would create a custom transport
and security protocol whose cost and risk do not advance EAB domain behavior.

Quinn 0.11 is selected for Rust secure unicast, using its rustls TLS 1.3
backend. The implemented slice proves:

- startup initiated through the existing `loadngo-proactor` ownership model
- an authenticated TLS 1.3 QUIC connection on IPv4 loopback
- exact SHA-256 certificate pin verification before application data
- rejection of a mismatched pin during the handshake
- bounded bidirectional streams, disabled unidirectional streams, bounded
  request/response bytes, and explicit idle/operation timeouts
- deterministic claim submission/status frames, encrypted player-session
  binding, and transport-independent authority acknowledgement

The high-level Quinn API is asynchronous and Tokio-based. The spike therefore
uses the proactor for lifecycle initiation and a dedicated current-thread Tokio
worker for QUIC I/O. This is an explicit integration bridge, not a claim that
Quinn is driven natively by the loadngo completion loop.

Still to prove before production selection is final:

- intended desktop and mobile platform matrix
- automatic persistent certificate/key startup and rotation
- connection/principal quotas and overload behavior
- loss, reconnect, migration, MTU, and timeout fault injection
- IPv6 and scoped link-local secure-unicast tests

No new private wire message should depend on a specific QUIC library API.

## D2: Versioned numeric messages with deterministic CBOR

Decision: accepted at the protocol level

- V2 messages use explicit numeric message and schema identifiers.
- Structured application payloads use deterministic CBOR as defined by RFC
  8949 deterministic encoding rules.
- Signed request bytes are the deterministic encoded payload, never a Rust
  memory layout or ordinary JSON serialization.
- Every decoder receives an outer byte limit before allocating fields.
- Each string, byte array, list, evidence body, and nesting level has its own
  protocol limit.
- Golden byte vectors define interoperability independent of Rust.

QUIC stream messages use an explicit bounded length prefix. Raw discovery
datagrams use a small fixed header followed by one bounded CBOR payload.

The V2 discovery codec uses `minicbor` with explicit, definite-length arrays
and manual field ordering. Golden vectors demonstrate preferred-width integer
encoding and stable bytes. The same choice is provisional for signed/private
messages until a non-Rust decoder validates the claim vectors.

The discovery frame and field limits are documented in
[`eab-wire/README.md`](../eab-wire/README.md). The raw discovery sequence uses a
small probe, a no-larger cookie challenge, a cookie-bearing query, and only then
the larger public authority response. Cookie validation is required before the
response to limit spoofed-source amplification.

There are no deployed EAB peers, so the prototype EAB1/JSON decoder and its
presence, status, and full-definition award messages are deleted rather than
maintained as a compatibility track. The numeric version remains in the current
header for future incompatible protocol evolution; it does not imply that two
implementations are active.

## D3: Pinned authority identity for the first claim slice

Decision: accepted for the first delivery gate

- Each EAB authority has a durable certificate/key identity.
- The first profile configures one or more SHA-256 fingerprints of complete
  DER authority certificates.
- Discovery advertises a certificate fingerprint, but the QUIC handshake must
  present that exact certificate and pass TLS certificate, validity, hostname,
  chain, and handshake-signature verification before application data is sent.
- Multiple simultaneous pins permit planned key rotation.
- Unknown and expired authority identities fail closed. Operators revoke the
  current first-profile identity by removing its pin; a durable revocation
  distribution mechanism remains production work.

A broader CA or federated trust model may follow. It is not required to prove
the first configured-authority claim transport.

The `generate_for_spike` helper creates an ephemeral self-signed certificate
only for tests and local experiments. It is not an authority provisioning or
rotation mechanism and must not be used for a provider identity that must
survive restart.

## D4: Deterministic trusted endpoint selection

Decision: implemented for discovered candidates

- Trust is empty by default; no configured pins means no selected authority.
- `EAB_TRUSTED_AUTHORITY_FINGERPRINTS` accepts one or more comma/whitespace
  separated 64-character hexadecimal certificate fingerprints.
- Selection discards unknown pins, expired responses, and incompatible wire
  version ranges.
- Pin configuration order is preference order. Ties are deterministically
  ordered by node id, advertised endpoint, and discovery source.
- The selected endpoint is still untrusted routing input until the QUIC
  handshake proves the selected certificate pin.

Discovery source address, node id, endpoint text, response latency, and
capability claims never override configured trust. Revocation beyond removing a
pin from configuration and production key rotation remain future work.

## D5: Player account binding

Decision: accepted for the transitional claim slice

- The existing player session credential may be presented only inside the
  authenticated encrypted QUIC connection.
- The server derives `player_id` from that credential/session.
- `player_id`, local player slot, save id, installation id, and UDP source
  address cannot independently authorize the destination account.
- Session expiry and revocation have the same meaning across HTTP and UDP.

Complete HTTP independence requires identity-provider exchange and session
refresh over QUIC in the later player-parity phase.

## D6: Post-quantum posture

Decision: accepted with an explicit limitation

- Trusted-service mutations require canonical post-quantum signatures.
- Reuse the qcoin signature abstraction where practical; it currently supports
  Dilithium2 and Falcon512.
- Those signatures provide issuer authentication, scopeable provenance, and
  replay-verifiable request bytes. They do not encrypt the channel.
- The first QUIC implementation may use the maintained TLS 1.3 algorithms
  available in the selected library, but it must not be described as providing
  post-quantum confidentiality.
- Before a deployment requires protection against harvest-now/decrypt-later,
  select and test a maintained hybrid/PQ TLS or KEM path and rotate sessions.

No custom KEM, cipher, handshake, or certificate scheme will be implemented in
EAB.

## D7: Request and operation idempotency

Decision: accepted

- `request_id` identifies one transport request under an authenticated
  principal.
- `claim_id` remains the cross-transport logical identity of an offline claim.
- signed-service request ids/nonces are durably consumed for replay protection.
- once-per-account achievement identity remains a separate domain rule.
- a response timeout means outcome unknown; the client reconnects and queries
  the exact operation identity.

## D8: Active multicast discovery, not provider presence announcements

Decision: accepted for the EAB node plane

- EAB participants emit a bounded multicast `Probe` at startup and on a
  low-rate interval.
- A configured authority responds directly with `Challenge`; the discoverer
  returns `Query`, and only a valid source-bound cookie permits the larger
  public `Response`.
- There is no unsolicited EAB `PresenceAnnounce` and no legacy presence
  compatibility decoder.
- Provider-to-provider discovery uses the same exchange. Because every
  provider also probes, two providers can discover one another without a
  separate multicast role announcement.
- Discovery responses are expiring, untrusted authority candidates. Provider
  role and identity are accepted only after configured trust filtering and the
  authenticated secure-unicast handshake.
- Qcoin's separate `QCN1 PresenceAnnounce` is outside this decision and is not
  an EAB discovery message.

Reason:

Active discovery preserves low-friction multicast bootstrap while keeping the
unauthenticated plane small. It avoids a redundant provider-announcement
message, does not ask listeners to trust a multicast role claim, and gives the
cookie exchange a single entry path for multicast, static peers, and later DNS
bootstrap.

Accepted tradeoff:

This is discovery, not a general membership or gossip protocol. Passive
listeners do not receive a provider roster, and provider startup may be noticed
on the next probe interval rather than through an unsolicited heartbeat.
Expiring responses provide discovery freshness but are not authoritative
provider-health evidence.

Revisit this decision only if a concrete requirement needs passive provider
observation, provider-start notification faster than the probe interval, or an
operator-visible multicast membership view. In that case, add a bounded
current-protocol provider-presence message that triggers the same cookie and
secure-unicast path; do not restore EAB1 or place private/provider-authoritative
state on multicast.

## Open measurements and limits

The following require a measured implementation spike before constants are
frozen:

- maximum claim envelope and evidence sizes
- discovery datagram size within the supported path-MTU target
- concurrent connections and streams per principal/source
- handshake, idle, request, and status timeouts
- retry deadlines and jitter
- response-cache and replay-record retention
- certificate size impact, particularly for any hybrid/PQ handshake
- supported OS, engine, and mobile runtime matrix

Defaults must be bounded and fail closed. Limits become part of the versioned
protocol contract and test vectors rather than undocumented implementation
details.
