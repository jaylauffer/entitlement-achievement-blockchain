# Signed Service Requests Roadmap

Purpose: define the intended long-term replacement for static trusted-service
bearer tokens when authorizing authoritative reward mutations.

This note is a roadmap target, not a statement that the feature exists today.

Related policy:

- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](/Users/jay/pudding/entitlement-achievement-blockchain/docs/AUTHORIZATION_AND_OFFLINE_CLAIMS.md)

## Problem

Static service or developer bearer tokens are acceptable for a near-term
trusted-service model, but they have clear limits:

- secret distribution is operationally awkward
- rotation is manual and error-prone
- request provenance is weak
- replay protection has to be layered on separately
- cross-service federation is clumsy

For the long term, the service should move toward signed structured requests.

## Core Model

The authoritative mutation should be a signed request, not a bearer token plus
an unsigned JSON body.

That means:

1. a trusted service constructs a canonical request payload
2. the trusted service signs the payload
3. EAB verifies the signature against a registered public key
4. EAB checks scope, freshness, replay protection, and namespace ownership
5. if valid, EAB performs the authoritative mutation

The service remains authoritative for applying the reward. The signature proves
that an authorized issuer requested the mutation.

## Post-Quantum Requirement

This feature should be post-quantum secure from the start.

That means:

- do not design the roadmap around ECDSA, Ed25519, or RSA
- do not assume a future migration from classical signatures later
- choose a PQ-safe signature abstraction at the protocol boundary now

The implementation should align with the QCoin stack where practical so the
same family of PQ signature abstractions can be shared or mirrored across the
ecosystem.

Practical requirement:

- request signatures must use a post-quantum scheme
- public-key registration must record the scheme identifier
- verification code must reject scheme mismatch explicitly

## Request Types Covered

The first signed-service-request rollout should cover:

- achievement award requests
- entitlement grant requests

It may later expand to:

- definition registration
- claim attestation
- cross-service reconciliation or reissue flows

## Canonical Request Shape

The signed payload should be a canonical structured message with fields such as:

- `request_type`
- `developer`
- `game`
- `target_player_id`
- mutation payload:
  - achievement id/version
  - or entitlement id/version/quantity/expiration
- `issued_at`
- `expires_at` or TTL
- `nonce` or request id
- `issuer_key_id`

The exact serialization can be decided later, but it must be deterministic.

Requirements:

- canonical serialization
- explicit versioning
- scheme-aware signature verification
- enough fields for replay protection and audit

## Verification Requirements

When EAB receives a signed service request, it should verify:

1. the signature is valid for the canonical payload
2. the key id exists and is registered
3. the registered key is authorized for the stated developer/game namespace
4. the request type is allowed for that key
5. the request is still fresh
6. the nonce/request id has not already been used

Only after those checks should the service perform the mutation.

## Replay Protection

Replay protection is mandatory.

Minimum viable replay protection:

- each signed request includes a unique `request_id` or nonce
- EAB stores consumed request ids
- duplicate request ids are rejected deterministically

Freshness should also be enforced:

- `issued_at`
- optional `expires_at`
- or bounded age relative to server time

## Key Registration Model

Signed requests require an issuer key registry.

Each registered service key should carry:

- `key_id`
- public key bytes
- signature scheme id
- developer or tenant ownership
- allowed games or namespaces
- allowed scopes
- status:
  - active
  - rotated
  - revoked

## Scope Model

Suggested initial scopes:

- `register:definitions`
- `award:achievements`
- `grant:entitlements`

Keys should be least-privilege where possible.

## Relationship To Offline Claims

Offline player achievement claims are separate from signed service requests.

The intended split is:

- player/client submits claims
- trusted service or EAB policy engine evaluates claims
- authoritative award may then be produced directly by EAB
- or by an internal trusted service using a signed service request model

So:

- signed service requests authorize trusted infrastructure
- offline claims describe player-side events

They solve different problems.

## Near-Term vs Long-Term

### Near-Term

Use scoped bearer-token authorization for:

- definition registration
- achievement awards
- entitlement grants

This gets the trust boundary fixed quickly.

### Long-Term

Replace or augment bearer tokens with PQ signed service requests for:

- stronger provenance
- better replay protection
- better rotation and federation
- better ecosystem alignment with QCoin

## Non-Goals For First Implementation

The first signed request implementation does not need:

- on-chain verification
- decentralized key discovery
- threshold signatures
- zero-knowledge proofs
- client-side entitlement issuance

Keep the first version narrow and deterministic.

## Recommended Rollout Sequence

1. lock down current bearer-token authorization boundaries
2. document signed request canonical payload format
3. add issuer key registry model
4. implement PQ signature verification for award/grant requests
5. add replay protection and freshness checks
6. migrate trusted services to signed requests
7. deprecate or narrow raw bearer-token mutation paths
