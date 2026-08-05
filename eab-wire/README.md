# EAB Wire Contracts

`eab-wire` defines bounded, versioned protocol values shared by EAB transport
adapters. It does not open sockets, select trusted authorities, authenticate
players, or invoke the EAB runtime.

The current implementation contains both the raw UDP discovery codec used by
the running EAB node and the secure-stream contracts for canonical claim
submission and status reconciliation. There is no legacy compatibility
decoder. Networking remains outside this crate.

Discovery is active rather than announcement-driven: clients and providers
both emit `Probe`, and configured authorities respond through the direct cookie
exchange. There is no EAB `PresenceAnnounce`. This still supports
provider-to-provider discovery without treating an unauthenticated multicast
role announcement as authority. It intentionally does not provide passive
membership observation or gossip.

## Discovery sequence

```text
client                          candidate authority
  |--- multicast Probe -------------------------->|
  |<-- unicast Challenge (source-bound cookie) ---|
  |--- unicast Query (nonce + cookie) ------------>|
  |<-- unicast Response (public metadata) --------|
  |                                               |
  |=== authenticated QUIC handshake ==============|
  |    fingerprint must match configured trust    |
```

The challenge cannot be larger than the probe. A candidate must validate the
cookie against the query source, request id, client nonce, and a short expiry
before sending the larger response. Cookie construction is intentionally not a
wire concern; implementations should use a keyed MAC and rotate the secret.

A discovery response is untrusted routing input. The client must compare its
fingerprint with configured trust and then require the QUIC peer to prove that
identity. Discovery never carries credentials, claims, evidence, account ids,
receipts, reward state, or detailed status.

## Datagram framing

All multi-byte header integers are unsigned big-endian values:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII magic `EAB2` |
| 4 | 2 | discovery wire version (`2`) |
| 6 | 2 | numeric message kind |
| 8 | 2 | CBOR payload length |
| 10 | bounded | deterministic CBOR payload |

The complete datagram is limited to 1,200 bytes. Decoding rejects an invalid
magic, unknown version or kind, inconsistent length, indefinite arrays,
trailing CBOR data, and out-of-range fields.

Message kinds are stable:

| Id | Name | Deterministic CBOR array fields |
|---:|---|---|
| 1 | `Probe` | request id, client nonce, minimum version, maximum version |
| 2 | `Challenge` | request id, opaque cookie |
| 3 | `Query` | request id, client nonce, opaque cookie |
| 4 | `Response` | request id, node id, QUIC endpoint, authority fingerprint, minimum version, maximum version, sorted capability ids, expiry |

CBOR arrays are definite-length and field order is the schema. Integers use
their shortest representation. Golden-vector tests pin the interoperable bytes
without exposing a Rust enum memory layout.

## Current limits

- request id: exactly 16 bytes and not all zero
- client nonce/cookie: exactly 16 bytes and not all zero
- authority fingerprint: exactly 32 bytes (the first profile is SHA-256 over
  the complete DER authority certificate)
- node id: 1–64 UTF-8 bytes, without control characters
- endpoint: 1–255 UTF-8 bytes, without control characters
- capabilities: at most 32 non-zero, unique ids in ascending order
- expiry: non-zero Unix seconds

These are discovery limits. Secure frames have separate bounds below.

## Secure stream framing

After the authority certificate is authenticated by QUIC/TLS, each
bidirectional stream carries exactly one request frame and one response frame.
All header integers are unsigned big-endian values:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII magic `EABS` |
| 4 | 2 | secure wire version (`2`) |
| 6 | 2 | numeric message kind |
| 8 | 4 | deterministic CBOR payload length |
| 12 | bounded | deterministic CBOR payload |

The complete frame is limited to 64 KiB. Secure message kinds are:

| Id | Name | Account binding |
|---:|---|---|
| 1 | `SubmitClaimRequest` | encrypted session token, then canonical envelope |
| 2 | `SubmitClaimResponse` | transport-neutral acknowledgement |
| 3 | `ClaimStatusRequest` | encrypted session token and claim id |
| 4 | `ClaimStatusResponse` | exact claim id and optional acknowledgement |
| 5 | `ProtocolErrorResponse` | structured code, retry classification, bounded detail |

The request payload never contains an online `player_id`. The authority derives
the account from the encrypted session token. Offline `local_player_id`, save,
installation, and session fields remain provenance only and cannot select the
destination account.

Current secure limits include a 2 KiB session token, 32 KiB evidence string,
256-byte identifiers, 64-byte timestamps, and 128-byte hashes/digests. Definite
CBOR arrays, numeric enums, preferred-width integers, outer bounds, and golden
vectors keep the contract deterministic and language-independent.

## Rust use

```rust
use eab_wire::{DiscoveryMessage, DiscoveryProbe};

let frame = DiscoveryMessage::Probe(DiscoveryProbe {
    request_id: [1; 16],
    client_nonce: [2; 16],
    min_wire_version: 2,
    max_wire_version: 2,
})
.encode()?;

let decoded = DiscoveryMessage::decode(&frame)?;
# Ok::<(), eab_wire::WireError>(())
```

See
[`docs/EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md`](../docs/EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md)
for the complete delivery plan and
[`docs/EAB_UDP_PROTOCOL_DECISIONS.md`](../docs/EAB_UDP_PROTOCOL_DECISIONS.md)
for the security and encoding decisions.
