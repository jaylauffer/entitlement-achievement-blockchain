# Entitlement Achievement Blockchain

This repository contains a small Rust project exploring how player profiles, entitlements and achievements could be tracked on a simple blockchain. Hyper-dimensional vectors are used to represent profiles and concepts for similarity searches.

## Purpose

The project provides:

- A lightweight blockchain implementation for recording profile changes.
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

## API Overview

The HTTP API is implemented with `actix-web`. All endpoints expect an `Authorization` header containing one of the pre-defined developer tokens:

```
developer token pairs:
- dev1 / token1
- dev2 / token2
```

Available routes:

- `POST /profiles` – Create a player profile. Body: `{ "name": "Player" }`
- `GET /profiles/{id}` – Retrieve a profile by id.
- `POST /profiles/{id}/dimensions` – Set the complete profile vector. Body: `{ "lanes": [...], "dim": N }`
- `POST /profiles/{id}/concepts` – Merge a concept vector into a profile. Body: `{ "developer": "dev", "game": "g", "concept": "c" }`
- `POST /concepts` – Create or fetch a concept vector. Body: `{ "developer": "dev", "game": "g", "concept": "c", "dim": N? }`
- `GET /concepts/{developer}/{game}/{concept}` – Fetch an existing concept vector.
- `POST /achievements` – Register an achievement definition.
- `POST /profiles/{id}/achievements` – Award a defined achievement to a profile.
- `POST /entitlements` – Register an entitlement definition.
- `POST /profiles/{id}/entitlements` – Grant a defined entitlement to a profile.

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

### Environment Variables

| Variable  | Purpose                           | Default |
|-----------|-----------------------------------|---------|
| `BIND_IP` | IP address the server binds to    | `0.0.0.0` |
| `BIND_PORT`| Port for the HTTP server          | `8080` |

### Deployment

For production deployments build the release binary and optionally serve it behind a reverse proxy:

```bash
cargo build --release --manifest-path rust/Cargo.toml
./target/release/rust_blockchain
```
Blockchain logs are stored under the `player_logs` directory relative to the working directory.

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

## Building Blocks

- **hyper dimensional vectors (`hd.rs`)** – provides seeded vector generation, bit operations and distance functions.
- **concept registry (`concept_registry.rs`)** – persists deterministic vectors for developer/game concepts.
- **blockchain (`blockchain.rs`)** – stores transaction blocks which are logged to disk via `ledger_storage.rs`.
- **player profiles (`player_profile.rs`)** – maintains player vectors and writes profile changes to the blockchain.

The repository is intended as a demonstration and starting point for further experimentation.

