# eab-game-sdk

Rust SDK for game developers integrating with the Entitlement Achievement Blockchain API.

Current authorization note:

- direct reward mutation endpoints are transitional
- long-term trusted-service authorization is expected to move toward
  post-quantum signed service requests
- see
  [SIGNED_SERVICE_REQUESTS_ROADMAP.md](../docs/SIGNED_SERVICE_REQUESTS_ROADMAP.md)

## What it provides

- Embedded offline achievement evaluation through re-exported `eab-core` types
- Durable offline EAB records with stable online claim identities
- Certificate-pinned QUIC claim submission and exact-id reconciliation
- Identity exchange (`/identity/exchange`)
- Profile create/query
- Reward state query (`/profiles/{id}/rewards`)
- Achievement/entitlement definition registration
- Achievement/entitlement award submission using trusted-service authorization
- Award receipt integrity verification (`data_hash` vs serialized receipt details)

## Offline first

Use `record_offline_achievement` while the game is disconnected. It evaluates
the shared structured definition and creates a native EAB offline record whose
`claim_id` is final from the moment of the local award.

When the player later links an account, game sync code targets
`EabClaimTransport` rather than HTTP directly:

```rust
use eab_game_sdk::EabClaimTransport;

let transport = client.claim_transport(player_id, player_token);
let acknowledgement = transport.submit_claim(&offline_record)?;
let reconciled = transport.claim_status(&offline_record.claim_id)?;
```

`HttpEabClaimTransport` is the compatibility adapter.
`QuicEabClaimTransport` is the direct secure-unicast adapter. It accepts an
already selected authority endpoint, its configured SHA-256 DER certificate
fingerprint, and the player's session token:

```rust
use eab_game_sdk::{EabClaimTransport, QuicEabClaimTransport};

let transport = QuicEabClaimTransport::new(
    "127.0.0.1:4542".parse()?,
    authority_certificate_fingerprint,
    player_session_token,
)?;
let acknowledgement = transport.submit_claim(&offline_record)?;
let reconciled = transport.claim_status(&offline_record.claim_id)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

This constructor is deliberately static-endpoint only. The remaining loadngo
adapter will feed it the result of IPv6 multicast/static/DNS discovery and
trusted endpoint selection; discovery will never carry the session or claim.

The transport sends a versioned `EabClaimEnvelope` containing the complete
immutable offline record and returns `EabClaimAcknowledgement`. The online
account id and credentials are adapter-owned and are not embedded in the
client-controlled record. Identical retries return the stored authority result;
exact-id status lookup supports recovery after timeouts and restarts.

The QUIC transport is synchronous at the trait boundary and creates a short
connection per operation in this first implementation. Run synchronization on
a game background worker. Connection reuse, persistent retry scheduling, and
discovery integration remain future work.

If submission loses the connection or times out, the adapter returns
`QuicClaimTransportError::OutcomeUnknown`; retain the offline record and call
`claim_status` with the same `claim_id` before deciding whether to resubmit.

Records whose policy forbids claim review, or which lack required evidence,
remain valid local EAB acknowledgements but cannot be converted for online
submission.

See
[STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md](../docs/STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md)
for the complete lifecycle and current limitations.

Transport behavior and security requirements are documented in
[EAB_CLAIM_TRANSPORT.md](../docs/EAB_CLAIM_TRANSPORT.md).

## Quick example

The example below uses the legacy thin claim path, which creates a pending
manual-review claim. Embedded offline games should use `EabClaimTransport` as
shown above.

```rust
use eab_game_sdk::{
    EabClient, RegisterAchievementRequest, SubmitAchievementClaimRequest,
};

fn flow() -> Result<(), Box<dyn std::error::Error>> {
    let client = EabClient::new("http://127.0.0.1:8080");
    let session = client.exchange_identity("steam", "local-dev-token")?;

    let _profile = client.create_profile(&session.access_token, "Player One")?;

    client.register_achievement(
        "token1",
        &RegisterAchievementRequest {
            developer: "dev1".into(),
            game: "my-game".into(),
            achievement_id: "first-win".into(),
            version: 1,
            name: "First Win".into(),
            description: "Win your first match".into(),
        },
    )?;

    let claim = client.submit_achievement_claim(
        &session.player_id,
        &session.access_token,
        &SubmitAchievementClaimRequest {
            developer: "dev1".into(),
            game: "my-game".into(),
            achievement_id: "first-win".into(),
            version: 1,
            claim_id: "claim-first-win".into(),
            session_id: "session-1".into(),
            client_sequence: 1,
            claimed_at: "2026-03-23T00:00:00Z".into(),
            evidence: Some("offline match win".into()),
        },
    )?;

    assert_eq!(claim.status, eab_game_sdk::AchievementClaimStatus::Pending);

    Ok(())
}
```
