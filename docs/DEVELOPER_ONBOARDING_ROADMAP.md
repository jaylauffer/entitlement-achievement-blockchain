# Developer Onboarding Roadmap

Purpose: define the near-term operational path for onboarding a publisher or
trusted service into EAB without relying on built-in example credentials.

Related notes:

- [AUTHORIZATION_AND_OFFLINE_CLAIMS.md](AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
- [SIGNED_SERVICE_REQUESTS_ROADMAP.md](SIGNED_SERVICE_REQUESTS_ROADMAP.md)

## Why This Exists

EAB now fails closed when no developer tokens are configured.

That is the correct security posture, but it means the repo needs a clear
publisher onboarding path instead of hidden defaults.

The core rule is:

- a player may submit claims
- a publisher or trusted service requests authoritative grants
- EAB validates namespace ownership and scope before mutating reward state

## Near-Term Trust Model

Until signed service requests land, publisher authority is established through:

1. explicit namespace ownership
2. explicit trusted-service token issuance
3. explicit scope assignment
4. explicit environment or file-based configuration on the EAB service

In other words:

- no implicit developer identities
- no repo-bundled broad-scope secrets
- no player-session path to authoritative grant mutation

## Onboarding Steps

### 1. Create The Publisher Namespace

Before issuing any token, decide the namespace the publisher owns:

- `developer`
- permitted `game` ids under that developer

This namespace is the boundary EAB uses when checking:

- definition registration
- achievement awards
- entitlement grants
- claim review/promotion

### 2. Choose The Trusted Service Role

Decide what the service is allowed to do.

Minimum scopes are:

- `register:definitions`
- `award:achievements`
- `grant:entitlements`
- `manage:concepts`

Recommended least-privilege split:

- a registry-management token for definitions and concepts
- an award token for achievements
- a grant token for entitlements

Do not hand every integration a single broad token unless there is a concrete
operational reason.

### 3. Mint The Token

Generate a high-entropy random token outside the repo.

Current EAB accepts either:

- `DEVELOPER_TOKENS_FILE`
- `DEVELOPER_TOKENS`

Preferred shape is the JSON file because it is easier to audit and rotate.

Example:

```json
{
  "tokens": [
    {
      "developer": "zhoenus",
      "token": "replace-with-long-random-secret",
      "scopes": [
        "register:definitions",
        "award:achievements"
      ]
    },
    {
      "developer": "zhoenus",
      "token": "replace-with-second-long-random-secret",
      "scopes": [
        "grant:entitlements"
      ]
    }
  ]
}
```

### 4. Install The Token On The EAB Host

Place the token file outside the repository, for example:

- `/etc/eab/developer_tokens.json`

Then point EAB at it:

```bash
DEVELOPER_TOKENS_FILE=/etc/eab/developer_tokens.json
```

Operational rules:

- restrict file permissions to the service account or root
- never commit the token file
- do not reuse the same token across unrelated services or tenants

### 5. Verify The Namespace Boundary

Before onboarding is considered complete, verify:

1. the token can mutate only its own `developer` namespace
2. missing scopes are rejected
3. player-session tokens cannot hit authoritative mutation endpoints
4. mismatched developer tokens are rejected

This should be part of deployment acceptance, not an afterthought.

### 6. Register Definitions

Once the namespace is established, the publisher registers:

- achievement definitions
- entitlement definitions

Definitions should be treated as the canonical catalog for later mutation
requests.

Claims and awards should reference those definitions rather than inventing new
reward identity in each request.

### 7. Move Live Issuance Behind Publisher Infrastructure

The game client should not directly hold trusted-service credentials.

Instead:

- the game or game backend evaluates player state
- the publisher backend decides whether a reward mutation is warranted
- that backend calls EAB with the trusted-service credential

This is the current operational way a publisher becomes the authoritative
source for entitlement and achievement grants.

## Rotation And Revocation

Near-term token onboarding must include a rotation story.

Required practice:

- be able to add a new token before removing the old one
- give each token a known owner and purpose
- remove unused tokens promptly
- treat entitlements as the highest-risk scope

Current gap:

- the code does not yet model token ids, expiry, or revocation metadata

That should be handled operationally for now and moved into a first-class key
registry later.

## What This Does Not Solve

This roadmap establishes operational authority, not cryptographic provenance.

It does not yet provide:

- per-request signatures
- replay protection on mutation requests
- issuer key registration
- post-quantum request verification

Those belong to the signed-request roadmap.

## Recommended Next Stage

After token-based onboarding is stable, the next upgrade is:

1. issuer key registry
2. canonical signed mutation envelopes
3. PQ signature verification
4. request freshness and replay protection
5. deprecation of raw bearer-token mutation paths

That is the point where publisher authority moves from "whoever knows the
secret" to "a registered issuer key explicitly authorized this exact mutation."
