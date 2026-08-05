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

When the player later links an account, a claim-ready record converts directly
into the current HTTP request without changing that identity:

```rust
let request = SubmitAchievementClaimRequest::try_from(&offline_record)?;
client.submit_achievement_claim(player_id, player_token, &request)?;
```

Records whose policy forbids claim review, or which lack required evidence,
remain valid local EAB acknowledgements but cannot be converted for online
submission.

See
[STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md](../docs/STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md)
for the complete lifecycle and current limitations.

## Quick example

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
