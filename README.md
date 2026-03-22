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

- **Hyper-dimensional vectors (`hd.rs`)** – deterministic seeded vector generation and distance functions used to represent profiles and concepts.
- **Concept registry (`concept_registry.rs`)** – stores deterministic vectors for developer/game concepts on disk.
- **Blockchain (`blockchain.rs`)** – records `Transaction` blocks for profile changes, entitlements and achievements.
- **Player profiles (`player_profile.rs`)** – manages profile data and writes updates to the blockchain via an abstract `LedgerStorage`.
- **Ledger storage (`ledger_storage.rs`)** – a simple file based persistence mechanism for blocks.

The `api` module exposes the REST endpoints that operate on these components. All state changes are logged to a per-player append-only ledger to enable reconstruction and verification of profile history.

Important policy note:

- [Authorization And Offline Claims Model](/Users/jay/pudding/entitlement-achievement-blockchain/docs/AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [State Reconciliation Model](/Users/jay/pudding/entitlement-achievement-blockchain/docs/STATE_RECONCILIATION_MODEL.md)
- [Signed Service Requests Roadmap](/Users/jay/pudding/entitlement-achievement-blockchain/docs/SIGNED_SERVICE_REQUESTS_ROADMAP.md)

## API Overview

The HTTP API is implemented with `actix-web` and supports two authentication flows:

- **Player authentication** uses an identity exchange endpoint to issue short-lived session tokens. The session token is then supplied via `Authorization: Bearer <token>` for all `/profiles/...` endpoints.
- **Developer / trusted-service authentication** uses static developer tokens for registry operations and authoritative reward mutation endpoints. Tokens are loaded at runtime from either a JSON file specified by `DEVELOPER_TOKENS_FILE` or from the `DEVELOPER_TOKENS` environment variable. Tokens now carry explicit scopes, and the default configuration includes two example broad-scope entries:

```
developer token pairs:
- dev1 / token1
- dev2 / token2

default scopes:
- manage:concepts
- register:definitions
- award:achievements
- grant:entitlements
```

Available routes:

- `POST /identity/exchange` – Exchange a provider token for a session token. Body: `{ "provider": "steam", "token": "provider-token" }`
- `POST /profiles` – Create a player profile. Body: `{ "name": "Player" }`
- `GET /profiles/{id}` – Retrieve a profile by id.
- `POST /profiles/{id}/dimensions` – Set the complete profile vector. Body: `{ "lanes": [...], "dim": N }`
- `POST /profiles/{id}/concepts` – Merge a concept vector into a profile. Body: `{ "developer": "dev", "game": "g", "concept": "c" }`
- `POST /profiles/{id}/achievement-claims` – Submit a pending achievement claim for the authenticated player. Body: `{ "developer": "dev", "game": "g", "achievement_id": "a", "version": 1, "claim_id": "c", "session_id": "s", "client_sequence": 1, "claimed_at": "...", "evidence": "..."? }`
- `POST /concepts` – Create or fetch a concept vector. Body: `{ "developer": "dev", "game": "g", "concept": "c", "dim": N? }`
- `GET /concepts/{developer}/{game}/{concept}` – Fetch an existing concept vector.
- `POST /achievements` – Register an achievement definition.
- `POST /profiles/{id}/achievements` – Award a defined achievement to a profile. Requires trusted-service authorization, not a player session token.
- `POST /entitlements` – Register an entitlement definition.
- `POST /profiles/{id}/entitlements` – Grant a defined entitlement to a profile. Requires trusted-service authorization, not a player session token.

### Identity Exchange

The identity exchange endpoint maps external provider tokens to an internal `player_id`. Provider tokens are verified using one of the following mechanisms:

- `IDENTITY_PROVIDER_TOKENS_FILE` containing a JSON map of `{ "tokens": { "<provider>": { "<token>": "<subject>" } } }`
- `IDENTITY_PROVIDER_TOKENS` environment variable with comma-separated `provider:token:subject` entries

If no provider token mappings are configured, the service treats the incoming token as the subject identifier. The identity map is persisted to `IDENTITY_MAP_PATH` (default `identity_map.json`), and a new `player_id` is generated when a provider/subject pair is first seen.

Player-owned `/profiles/...` read/update endpoints require the session token issued by `/identity/exchange`, and the `{id}` in the path must match the `player_id` tied to that session token.

The first achievement-claim submission path is player-facing:

- `POST /profiles/{id}/achievement-claims`

Current behavior for this first pass:

- claims are accepted as pending player-submitted records
- claims do not mutate authoritative rewards
- duplicate `claim_id` submissions for the same player are idempotent
- claims are not yet persisted across restart or converted into awards automatically

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
QCOIN_STATE_PATH=qcoin_chain_state.json \
QCOIN_NODE_URL=http://127.0.0.1:9710 \
cargo run --manifest-path rust/Cargo.toml
```

### Environment Variables

| Variable  | Purpose                           | Default |
|-----------|-----------------------------------|---------|
| `BIND_IP` | IP address the server binds to    | `0.0.0.0` |
| `BIND_PORT`| Port for the HTTP server          | `8080` |
| `DEVELOPER_TOKENS_FILE` | Path to JSON file containing developer token entries | `None` |
| `DEVELOPER_TOKENS` | Comma separated list `dev:token[:scope1+scope2]` entries. Legacy `dev:token` entries still get the default broad scopes. | `"dev1:token1,dev2:token2"` |
| `LEDGER_BACKEND` | `file`, `sled`, or `qcoin` ledger storage implementation | `file` |
| `LEDGER_DB_PATH` | Directory for sled database when `LEDGER_BACKEND=sled` | `ledger_db` |
| `LEDGER_TOPICS_PATH` | Directory for per-player append-only logs when `LEDGER_BACKEND=qcoin` | `player_logs` |
| `QCOIN_STATE_PATH` | Path for mirrored QCoin chain state when `LEDGER_BACKEND=qcoin` | `qcoin_chain_state.json` |
| `QCOIN_NODE_URL` | Optional remote qcoin-node endpoint for submitting mirrored blocks (`POST /blocks`) | `None` |
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
When `LEDGER_BACKEND` is set to `qcoin`, per-player logs remain in `player_logs` and each block append is mirrored to local QCoin chain state (`qcoin_chain_state.json` by default). If `QCOIN_NODE_URL` is set, the mirrored block is also submitted to that qcoin-node over HTTP.

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
- capabilities: identity exchange, profile/rewards query, definition registration, award submission, and receipt integrity verification
- see [game-sdk-rust/README.md](game-sdk-rust/README.md) for quick-start usage

## Building Blocks

- **hyper dimensional vectors (`hd.rs`)** – provides seeded vector generation, bit operations and distance functions.
- **concept registry (`concept_registry.rs`)** – persists deterministic vectors for developer/game concepts.
- **blockchain (`blockchain.rs`)** – stores transaction blocks which are logged to disk via `ledger_storage.rs`.
- **player profiles (`player_profile.rs`)** – maintains player vectors and writes profile changes to the blockchain.
