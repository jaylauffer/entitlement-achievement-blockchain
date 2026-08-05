# Entitlement Achievement Blockchain

This repository contains a Rust project for a next generation player journey service which tracks player growth in terms of profiles, entitlements, and achievements tracked on a simple blockchain. Hyper-dimensional vectors are used to represent profiles and concepts for similarity searches.

## Purpose

The project provides:

- A blockchain ledger implementation for recording profile changes.
- A `PlayerProfileService` that manages player profiles and logs changes to the blockchain.
- A REST API server exposing endpoints to manage profiles and concepts.
- A command line tool for maintaining a concept registry.

## Architecture Overview

At a high level the project is composed of several cooperating modules:

- **Embedded EAB core (`eab-core/`)** – transport-neutral achievement
  definitions, offline event evaluation, immutable local EAB records, and
  durable reference storage for stand-alone games.
- **EAB wire contracts (`eab-wire/`)** – bounded, versioned discovery and
  secure-transport message contracts. The running node uses this single current
  discovery protocol; the prototype EAB1/JSON wire has been removed.
- **EAB QUIC client (`eab-quic-client/`)** – small certificate-pinned TLS 1.3
  client used by Rust games without depending on the authority/server crate.
- **Hyper-dimensional vectors (`hd.rs`)** – deterministic seeded vector generation and distance functions used to represent profiles and concepts.
- **Concept registry (`concept_registry.rs`)** – stores deterministic vectors for developer/game concepts on disk.
- **Blockchain (`blockchain.rs`)** – records `Transaction` blocks for profile changes, entitlements and achievements.
- **Player profiles (`player_profile.rs`)** – manages profile data and writes updates to the blockchain via an abstract `LedgerStorage`.
- **Ledger storage (`ledger_storage.rs`)** – a simple file based persistence mechanism for blocks.

The `api` module exposes the REST endpoints that operate on these components. All state changes are logged to a per-player append-only ledger to enable reconstruction and verification of profile history.

Important policy note:

- [Authorization And Offline Claims Model](docs/AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [State Reconciliation Model](docs/STATE_RECONCILIATION_MODEL.md)
- [EAB API Surface](docs/EAB_API_SURFACE.md)
- [EAB Acknowledgement And Anchor Architecture](docs/EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md)
- [Achievement Model](docs/ACHIEVEMENT_MODEL.md)
- [Stand-Alone Offline Achievement Support](docs/STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md)
- [`eab-core` Rust Game Integration Guide](eab-core/README.md)
- [`eab-wire` Protocol Guide](eab-wire/README.md)
- [Developer Onboarding Roadmap](docs/DEVELOPER_ONBOARDING_ROADMAP.md)
- [Signed Service Requests Roadmap](docs/SIGNED_SERVICE_REQUESTS_ROADMAP.md)
- [Loadngo Runtime Migration](docs/LOADNGO_RUNTIME_MIGRATION.md)
- [EAB Transport Design Goals](docs/EAB_TRANSPORT_DESIGN_GOALS.md)
- [EAB Claim Transport](docs/EAB_CLAIM_TRANSPORT.md)
- [EAB UDP Transport Implementation Plan](docs/EAB_UDP_TRANSPORT_IMPLEMENTATION_PLAN.md)
- [EAB UDP Protocol Decisions](docs/EAB_UDP_PROTOCOL_DECISIONS.md)

## API Overview

The HTTP API is implemented with `actix-web` and supports two authentication flows:

- **Player authentication** uses an identity exchange endpoint to issue short-lived session tokens. The session token is then supplied via `Authorization: Bearer <token>` for all `/profiles/...` endpoints.
- **Developer / trusted-service authentication** uses explicitly configured developer tokens for registry operations and authoritative reward mutation endpoints. Tokens are loaded at runtime from either a JSON file specified by `DEVELOPER_TOKENS_FILE` or from the `DEVELOPER_TOKENS` environment variable. If neither is configured, trusted-service mutation endpoints fail closed and return `401 Unauthorized`.

Trusted-service scopes:

- `manage:concepts`
- `register:definitions`
- `award:achievements`
- `grant:entitlements`

Operational note:

- do not ship broad example tokens in production config
- onboard each publisher namespace explicitly
- see [Developer Onboarding Roadmap](docs/DEVELOPER_ONBOARDING_ROADMAP.md)

Available routes:

- `POST /identity/exchange` – Exchange a provider token for a session token. Body: `{ "provider": "steam", "token": "provider-token" }`
- `POST /profiles` – Create a player profile. Body: `{ "name": "Player" }`
- `GET /profiles/{id}` – Retrieve a profile by id.
- `POST /profiles/{id}/dimensions` – Set the complete profile vector. Body: `{ "lanes": [...], "dim": N }`
- `POST /profiles/{id}/concepts` – Merge a concept vector into a profile. Body: `{ "developer": "dev", "game": "g", "concept": "c" }`
- `POST /profiles/{id}/achievement-claims` – Submit a pending achievement claim for the authenticated player. Body: `{ "developer": "dev", "game": "g", "achievement_id": "a", "version": 1, "claim_id": "c", "session_id": "s", "client_sequence": 1, "claimed_at": "...", "evidence": "..."? }`
- `GET /profiles/{id}/achievement-claims` – List persisted pending/reviewed achievement claims for the authenticated player.
- `POST /profiles/{id}/achievement-claims/{claim_id}/review` – Trusted-service review endpoint for a claim. Body: `{ "action": "promote" | "reject", "review_note": "..."? }`
- `POST /profiles/{id}/achievement-claim-envelopes` – Submit the complete canonical offline EAB record for authoritative validation and receive a transport-neutral acknowledgement.
- `GET /profiles/{id}/achievement-claims/{claim_id}/acknowledgement` – Reconcile the exact stored authoritative result for a canonical claim.
- `POST /concepts` – Create or fetch a concept vector. Body: `{ "developer": "dev", "game": "g", "concept": "c", "dim": N? }`
- `GET /concepts/{developer}/{game}/{concept}` – Fetch an existing concept vector.
- `POST /achievements` – Register an achievement definition. In addition to the
  core identity and display fields, the current model also accepts optional
  policy fields such as `category`, `visibility`, `repeatability`,
  `issuance_mode`, and `accomplishment`.
- `POST /profiles/{id}/achievements` – Award a defined achievement to a profile. Requires trusted-service authorization, not a player session token.
- `POST /entitlements` – Register an entitlement definition.
- `POST /profiles/{id}/entitlements` – Grant a defined entitlement to a profile. Requires trusted-service authorization, not a player session token.

### Identity Exchange

The identity exchange endpoint maps external provider tokens to an internal `player_id`. Provider tokens are verified using one of the following mechanisms:

- `IDENTITY_PROVIDER_TOKENS_FILE` containing a JSON map of `{ "tokens": { "<provider>": { "<token>": "<subject>" } } }`
- `IDENTITY_PROVIDER_TOKENS` environment variable with comma-separated `provider:token:subject` entries

If no provider token mappings are configured, the service treats the incoming token as the subject identifier. The identity map is persisted to `IDENTITY_MAP_PATH` (default `identity_map.json`), and a new `player_id` is generated when a provider/subject pair is first seen.

Player-owned `/profiles/...` read/update endpoints require the session token issued by `/identity/exchange`, and the `{id}` in the path must match the `player_id` tied to that session token.

Two player-facing claim paths now exist:

- `POST /profiles/{id}/achievement-claims`
- `POST /profiles/{id}/achievement-claim-envelopes`

The original thin path is retained for compatibility:

- claims are accepted as pending player-submitted records
- claims do not mutate authoritative rewards
- duplicate `claim_id` submissions for the same player are idempotent
- claims persist across restart
- trusted-service review may promote a claim into an authoritative achievement award
- claim listing is player-visible, but review/promotion is not player-authorized

The canonical envelope path is intended for games using embedded offline EAB:

- it transmits the complete immutable `OfflineAchievementRecord`
- the authenticated session binds the envelope to an online account; account
  identity is not client-controlled envelope data
- the authority resolves the registered definition and verifies integrity,
  readiness, identity, digest, and accomplishment policy
- it returns a versioned `EabClaimAcknowledgement` with a machine-readable
  acknowledged, rejected, or conflict result
- duplicate submission returns the original result, including across restart
- once-per-account award policy is enforced separately from claim-id
  idempotency
- HTTP is the current adapter; the service decision contract is transport
  independent

The authoritative reward mutation endpoints:

- `POST /profiles/{id}/achievements`
- `POST /profiles/{id}/entitlements`

require trusted-service authorization instead of player-session authorization.

Trusted-service authorization is now scope-based:

- `manage:concepts` for concept registry read/write
- `register:definitions` for achievement/entitlement definition registration
- `award:achievements` for authoritative achievement awards
- `grant:entitlements` for authoritative entitlement grants

## Setup

The source lives under the `rust` directory. Development is tested with **Rust 1.76** and requires a toolchain that supports the 2021 edition. Any modern stable release should work. If you use `rustup`, simply run `rustup default stable` to install the latest stable toolchain.

Clone the repository and run:

```bash
cargo build --manifest-path rust/Cargo.toml
```

### Running Tests

Execute the test suite with:

```bash
cargo test --manifest-path rust/Cargo.toml
```

### Running the API Server

The main binary `src/main.rs` starts the REST service. You can override the bind address using `BIND_IP` and `BIND_PORT` environment variables:

```bash
BIND_IP=127.0.0.1 BIND_PORT=8080 \
  cargo run --manifest-path rust/Cargo.toml
```

Run with QCoin mirroring enabled:

```bash
LEDGER_BACKEND=qcoin \
LEDGER_TOPICS_PATH=player_logs \
QCOIN_OUTBOX_PATH=qcoin_anchor_outbox.json \
QCOIN_NODE_TARGET=127.0.0.1:9700 \
cargo run --manifest-path rust/Cargo.toml
```

Run with the EAB discovery plane enabled on the same host. By default the node
reuses `BIND_PORT` for UDP, joins the embedded IPv6 multicast group, and emits
bounded discovery probes. It advertises an authority only when both a real
secure endpoint and its SHA-256 DER certificate fingerprint are configured:

```bash
BIND_IP=192.168.1.102 \
BIND_PORT=8080 \
cargo run --manifest-path rust/Cargo.toml
```

The repository now contains a Quinn/rustls certificate-pinned QUIC claim
service and Rust game client, but the main binary does not yet start it
automatically. Leave
`EAB_QUIC_ENDPOINT` and `EAB_AUTHORITY_FINGERPRINT_HEX` unset unless a real
secure endpoint with that exact persistent certificate is running. The node
then operates as a discovery client and never claims that the HTTP endpoint or
the test-only ephemeral QUIC identity is a secure EAB authority.

Clients fail closed unless an advertised certificate fingerprint appears in
`EAB_TRUSTED_AUTHORITY_FINGERPRINTS`. Pin order is preference order; an empty
list selects no authority. Discovery selection does not complete trust—the
selected QUIC peer must present the matching certificate during its TLS 1.3
handshake.

### Environment Variables

| Variable  | Purpose                           | Default |
|-----------|-----------------------------------|---------|
| `BIND_IP` | IP address the server binds to    | `0.0.0.0` |
| `BIND_PORT`| Port for the HTTP server          | `8080` |
| `DEVELOPER_TOKENS_FILE` | Path to JSON file containing developer token entries | `None` |
| `DEVELOPER_TOKENS` | Comma separated list `dev:token[:scope1+scope2]` entries. Legacy `dev:token` entries still get the default broad scopes. If unset, trusted-service auth is unavailable. | `None` |
| `LEDGER_BACKEND` | `file`, `sled`, or `qcoin` ledger storage implementation | `file` |
| `LEDGER_DB_PATH` | Directory for sled database when `LEDGER_BACKEND=sled` | `ledger_db` |
| `LEDGER_TOPICS_PATH` | Directory for per-player append-only logs when `LEDGER_BACKEND=qcoin` | `player_logs` |
| `QCOIN_OUTBOX_PATH` | Path for the persisted qcoin anchor outbox when `LEDGER_BACKEND=qcoin` | `qcoin_anchor_outbox.json` |
| `QCOIN_STATE_PATH` | Legacy fallback path for the qcoin anchor outbox if `QCOIN_OUTBOX_PATH` is unset | `qcoin_anchor_outbox.json` |
| `QCOIN_NODE_TARGET` | Preferred qcoin-node UDP target for anchor submission (`host:port`) | `None` |
| `QCOIN_NODE_URL` | Legacy fallback URL used to derive the qcoin UDP target when `QCOIN_NODE_TARGET` is unset | `None` |
| `EAB_NODE_DISABLE` | Disable the loadngo-backed EAB UDP node service plane | `false` |
| `EAB_NODE_BIND` | Explicit UDP bind address for the EAB node service plane | derived from `BIND_IP:BIND_PORT` |
| `EAB_NODE_PORT` | Override UDP port for the EAB node service plane | `BIND_PORT` |
| `EAB_NODE_PEERS` | Comma/space-separated static EAB peer endpoints used alongside multicast | `None` |
| `EAB_NODE_NAME` | Public node id used when an authority advertisement is enabled | hostname or `eab-node` |
| `EAB_QUIC_ENDPOINT` | Secure unicast authority endpoint advertised only when its fingerprint is also configured | `None` |
| `EAB_AUTHORITY_FINGERPRINT_HEX` | Exactly 32 fingerprint bytes encoded as 64 hexadecimal characters; required with `EAB_QUIC_ENDPOINT` | `None` |
| `EAB_TRUSTED_AUTHORITY_FINGERPRINTS` | Comma/whitespace-separated SHA-256 DER certificate fingerprints accepted for authority selection, in preference order | `None` (fail closed) |
| `EAB_DISABLE_DEFAULT_MULTICAST` | Disable the embedded IPv6 multicast discovery group | `false` |
| `EAB_MULTICAST_V6_GROUP` | Override the embedded IPv6 multicast discovery group | `ff02::4541:4200:1` |
| `EAB_MULTICAST_V6_INTERFACE` | Explicit IPv6 multicast interface index when auto-selection is not wanted | auto |
| `IDENTITY_MAP_PATH` | Path to the player identity mapping file | `identity_map.json` |
| `IDENTITY_PROVIDER_TOKENS_FILE` | JSON file containing per-provider token mappings | `None` |
| `IDENTITY_PROVIDER_TOKENS` | Comma separated `provider:token:subject` entries for local verification | `None` |
| `SUPPORTED_IDENTITY_PROVIDERS` | Comma separated list of allowed providers | `google_play_games,apple_id,epic,steam,oidc` |
| `CONCEPT_REGISTRY_PATH` | Path to the concept registry JSON file | `concept_registry.json` |
| `ACHIEVEMENT_REGISTRY_PATH` | Path to the achievement registry JSON file | `achievement_registry.json` |
| `ENTITLEMENT_REGISTRY_PATH` | Path to the entitlement registry JSON file | `entitlement_registry.json` |

### Deployment

For production deployments build the release binary and optionally serve it behind a reverse proxy:

```bash
cargo build --release --manifest-path rust/Cargo.toml
./target/release/rust_blockchain
```
Blockchain logs are stored under the `player_logs` directory relative to the working directory.
When `LEDGER_BACKEND` is set to `sled`, blocks are persisted in the `ledger_db` directory instead.
When `LEDGER_BACKEND` is set to `qcoin`, per-player logs remain in `player_logs` and each block append is turned into a qcoin anchor transaction queued in a persisted outbox (`qcoin_anchor_outbox.json` by default). If `QCOIN_NODE_TARGET` is set, a background `loadngo-proactor` worker drains that outbox by submitting transactions to the qcoin node over the qcoin UDP wire. `QCOIN_NODE_URL` remains a legacy fallback only for deriving the same target host and port.

The acceptance criteria for calling this path "usable" in the lab PoC are
tracked in [docs/QCOIN_ANCHOR_ACCEPTANCE_GATE.md](docs/QCOIN_ANCHOR_ACCEPTANCE_GATE.md).
The specific reward-anchor discovery and the remaining acceptance-vs-inclusion
gap are documented in
[docs/QCOIN_REWARD_ANCHOR_DISCOVERY.md](docs/QCOIN_REWARD_ANCHOR_DISCOVERY.md).
The structured achievement-definition target and enablement criteria are
documented in [docs/ACHIEVEMENT_MODEL.md](docs/ACHIEVEMENT_MODEL.md).
The layered contract between player evidence, developer definitions,
authoritative EAB acknowledgement, and qcoin proof anchoring is documented in
[docs/EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md](docs/EAB_ACKNOWLEDGEMENT_AND_ANCHOR_ARCHITECTURE.md).
The intended transport/runtime target beyond the current HTTP adapter is
documented in
[docs/EAB_TRANSPORT_DESIGN_GOALS.md](docs/EAB_TRANSPORT_DESIGN_GOALS.md).

The EAB process now also starts a lightweight `loadngo/network` discovery plane
by default. It has one wire implementation: bounded deterministic-CBOR
probe/challenge/query/response messages from `eab-wire`. The source-bound cookie
round trip occurs before the larger public authority response. Static peers and
IPv6 multicast are both bootstrap inputs.

The legacy EAB1 JSON presence, detailed status, and unauthenticated award
messages have been deleted. Bounded `eab-wire` claim submission and status
messages now run over certificate-pinned QUIC and bind the online account from
the encrypted player session. This adapter is currently started explicitly by
an embedding application; the default main binary still serves claims over
HTTP. Raw discovery carries neither credentials nor claims.

### Running in Docker

This repository includes a `Dockerfile` for convenience. Build the image and run the server using:

```bash
docker build -t entitlement-chain .
docker run -p 8080:8080 entitlement-chain
```

### Concept Registry Tool

A helper binary `concept_tool` adds new concepts to `concept_registry.json`:

```bash
cargo run --manifest-path rust/Cargo.toml --bin concept_tool -- <developer> <game> <concept> [--dim N]
```

## Rust Game SDK

A first-party Rust SDK lives in `game-sdk-rust/`:

- crate name: `eab-game-sdk`
- capabilities: embedded offline achievement evaluation, durable local EAB
  records, identity exchange, profile/rewards query, definition registration,
  claim/award submission, and receipt integrity verification
- see [game-sdk-rust/README.md](game-sdk-rust/README.md) for quick-start usage

## Building Blocks

- **hyper dimensional vectors (`hd.rs`)** – provides seeded vector generation, bit operations and distance functions.
- **concept registry (`concept_registry.rs`)** – persists deterministic vectors for developer/game concepts.
- **blockchain (`blockchain.rs`)** – stores transaction blocks which are logged to disk via `ledger_storage.rs`.
- **player profiles (`player_profile.rs`)** – maintains player vectors and writes profile changes to the blockchain.
