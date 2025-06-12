# Entitlement Achievement Blockchain

This repository contains a small Rust project exploring how player profiles, entitlements and achievements could be tracked on a simple blockchain. Hyper-dimensional vectors are used to represent profiles and concepts for similarity searches.

## Purpose

The project provides:

- A lightweight blockchain implementation for recording profile changes.
- A `PlayerProfileService` that manages player profiles and logs changes to the blockchain.
- A REST API server exposing endpoints to manage profiles and concepts.
- A command line tool for maintaining a concept registry.

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

## Setup

The source lives under the `rust` directory. To build everything you only need a recent Rust toolchain.

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

