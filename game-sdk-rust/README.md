# eab-game-sdk

Rust SDK for game developers integrating with the Entitlement Achievement Blockchain API.

Current authorization note:

- direct reward mutation endpoints are transitional
- long-term trusted-service authorization is expected to move toward
  post-quantum signed service requests
- see
  [SIGNED_SERVICE_REQUESTS_ROADMAP.md](../docs/SIGNED_SERVICE_REQUESTS_ROADMAP.md)

## What it provides

- Identity exchange (`/identity/exchange`)
- Profile create/query
- Reward state query (`/profiles/{id}/rewards`)
- Achievement/entitlement definition registration
- Achievement/entitlement award submission using trusted-service authorization
- Award receipt integrity verification (`data_hash` vs serialized receipt details)

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
