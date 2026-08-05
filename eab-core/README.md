# eab-core

Transport-neutral EAB definitions and embedded offline achievement support.

This crate is suitable for linking into a stand-alone game. It does not depend
on Actix, qcoin, or `loadngo`, and it contains no trusted-service authority.

## Offline flow

`record_offline_achievement` evaluates one structured achievement definition
against one game event. On success it appends an immutable
`OfflineAchievementRecord` and returns that same record as the immediate local
receipt.

The record's `claim_id` is generated at local award time and must be preserved
when the game later submits it to an EAB service.

```rust,no_run
use eab_core::{
    record_offline_achievement, AchievementAccomplishment,
    AchievementDefinition, FileOfflineAchievementStorage,
    OfflineAchievementContext, OfflineAchievementEvent,
};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let definition = AchievementDefinition::new(
    "pudding",
    "solo-flight",
    "first-flight",
    1,
    "First Flight",
    "Complete a successful flight",
)
.with_accomplishment(AchievementAccomplishment {
    summary: "Complete one successful flight".into(),
    event_key: Some("flight_completed".into()),
    threshold: Some(1),
    requires_evidence: false,
});

let event = OfflineAchievementEvent {
    event_key: "flight_completed".into(),
    value: 1,
    occurred_at: "2026-08-05T12:00:00Z".into(),
    evidence: None,
};
let context = OfflineAchievementContext {
    local_player_id: "player-slot-1".into(),
    save_id: "save-1".into(),
    installation_id: "installation-1".into(),
    session_id: "session-1".into(),
    client_sequence: 7,
    game_build: "1.0.0".into(),
};

let mut storage =
    FileOfflineAchievementStorage::open("offline-achievements.jsonl")?;
let outcome = record_offline_achievement(
    &mut storage,
    &definition,
    &event,
    &context,
)?;
# let _ = outcome;
# Ok(())
# }
```

## Current boundary

- one-time achievements only
- event-key and threshold evaluation
- once-per-player deduplication across definition versions
- optional evidence and explicit claim readiness
- in-memory and single-writer JSON-lines storage
- SHA-256 definition digest and local record integrity checking

Local integrity hashes detect corruption; they are not an anti-cheat boundary.
See
[the stand-alone design](../docs/STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md)
for the trust and online-reconciliation model.
