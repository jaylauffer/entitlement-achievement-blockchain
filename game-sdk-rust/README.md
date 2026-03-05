# eab-game-sdk

Rust SDK for game developers integrating with the Entitlement Achievement Blockchain API.

## What it provides

- Identity exchange (`/identity/exchange`)
- Profile create/query
- Reward state query (`/profiles/{id}/rewards`)
- Achievement/entitlement definition registration
- Achievement/entitlement award submission
- Award receipt integrity verification (`data_hash` vs serialized receipt details)

## Quick example

```rust
use eab_game_sdk::{
    AwardAchievementRequest, EabClient, RegisterAchievementRequest,
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

    let receipt = client.submit_achievement_award(
        &session.player_id,
        &session.access_token,
        &AwardAchievementRequest {
            developer: "dev1".into(),
            game: "my-game".into(),
            achievement_id: "first-win".into(),
            version: 1,
        },
    )?;

    let ok = EabClient::verify_receipt_integrity(&receipt)?;
    assert!(ok);

    Ok(())
}
```
