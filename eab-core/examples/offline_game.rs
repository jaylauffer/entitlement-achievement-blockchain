use std::path::Path;

use eab_core::{
    record_offline_achievement, AchievementAccomplishment, AchievementDefinition,
    AchievementIssuanceMode, AchievementRepeatability, AchievementVisibility,
    FileOfflineAchievementStorage, OfflineAchievementContext, OfflineAchievementEvent,
    OfflineAchievementRecord, OfflineAchievementStorage, OfflineAwardOutcome,
};

fn definition() -> AchievementDefinition {
    AchievementDefinition::new(
        "pudding",
        "solo-flight",
        "first-flight",
        1,
        "First Flight",
        "Complete a successful flight",
    )
    .with_category("progression")
    .with_policy(
        AchievementVisibility::Private,
        AchievementRepeatability::OncePerPlayer,
        AchievementIssuanceMode::DirectAwardOrClaimReview,
    )
    .with_accomplishment(AchievementAccomplishment {
        summary: "Complete one successful flight".into(),
        event_key: Some("flight_completed".into()),
        threshold: Some(1),
        requires_evidence: false,
    })
}

fn simulate_launch(
    path: &Path,
    session_id: &str,
    client_sequence: u64,
) -> Result<OfflineAchievementRecord, Box<dyn std::error::Error>> {
    let mut storage = FileOfflineAchievementStorage::open(path)?;
    println!("loaded {} existing record(s)", storage.records().len());

    let outcome = record_offline_achievement(
        &mut storage,
        &definition(),
        &OfflineAchievementEvent {
            event_key: "flight_completed".into(),
            value: 1,
            occurred_at: "2026-08-05T12:00:00Z".into(),
            evidence: None,
        },
        &OfflineAchievementContext {
            local_player_id: "player-slot-1".into(),
            save_id: "save-1".into(),
            installation_id: "installation-1".into(),
            session_id: session_id.into(),
            client_sequence,
            game_build: "1.0.0".into(),
        },
    )?;

    match outcome {
        OfflineAwardOutcome::Awarded(record) => {
            println!(
                "unlocked {} with claim {}",
                record.achievement_id, record.claim_id
            );
            Ok(record)
        }
        OfflineAwardOutcome::AlreadyAwarded(record) => {
            println!(
                "already unlocked {} with original claim {}",
                record.achievement_id, record.claim_id
            );
            Ok(record)
        }
        OfflineAwardOutcome::NoMatchingEvent => Err("example event did not match".into()),
        OfflineAwardOutcome::ThresholdNotMet { observed, required } => {
            Err(format!("example event did not reach threshold: {observed}/{required}").into())
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "offline-achievements.jsonl".to_string());
    let path = Path::new(&path);

    println!("first simulated launch");
    let first = simulate_launch(path, "session-1", 1)?;

    println!("\nsecond simulated launch");
    let second = simulate_launch(path, "session-2", 1)?;

    assert_eq!(second.local_award_id, first.local_award_id);
    assert_eq!(second.claim_id, first.claim_id);
    println!("\nrestart preserved the original award and claim identities");
    Ok(())
}
