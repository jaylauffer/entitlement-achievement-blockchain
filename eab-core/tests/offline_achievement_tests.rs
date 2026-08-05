use std::fs;

use eab_core::{
    record_offline_achievement, verify_offline_record_integrity, AchievementAccomplishment,
    AchievementDefinition, AchievementIssuanceMode, AchievementRepeatability,
    AchievementVisibility, FileOfflineAchievementStorage, MemoryOfflineAchievementStorage,
    OfflineAchievementContext, OfflineAchievementError, OfflineAchievementEvent,
    OfflineAchievementStorage, OfflineAwardOutcome, OfflineClaimReadiness,
};
use uuid::Uuid;

fn definition(version: u32) -> AchievementDefinition {
    AchievementDefinition::new(
        "pudding",
        "solo-flight",
        "first-flight",
        version,
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

fn context() -> OfflineAchievementContext {
    OfflineAchievementContext {
        local_player_id: "player-slot-1".into(),
        save_id: "save-1".into(),
        installation_id: "installation-1".into(),
        session_id: "session-1".into(),
        client_sequence: 7,
        game_build: "1.0.0".into(),
    }
}

fn event(value: u64) -> OfflineAchievementEvent {
    OfflineAchievementEvent {
        event_key: "flight_completed".into(),
        value,
        occurred_at: "2026-08-05T12:00:00Z".into(),
        evidence: None,
    }
}

#[test]
fn matching_event_creates_native_offline_eab_record() {
    let mut storage = MemoryOfflineAchievementStorage::new();

    let outcome = record_offline_achievement(&mut storage, &definition(1), &event(1), &context())
        .expect("record offline achievement");
    let OfflineAwardOutcome::Awarded(record) = outcome else {
        panic!("expected an offline award");
    };

    assert_eq!(record.achievement_id, "first-flight");
    assert_eq!(record.client_sequence, 7);
    assert_eq!(record.earned_at_local, "2026-08-05T12:00:00Z");
    assert!(!record.local_award_id.is_empty());
    assert!(!record.claim_id.is_empty());
    assert!(!record.definition_digest.is_empty());
    assert!(verify_offline_record_integrity(&record).expect("verify record"));
    assert_eq!(storage.records(), &[record]);
}

#[test]
fn event_below_threshold_does_not_write_a_record() {
    let mut definition = definition(1);
    definition.accomplishment.threshold = Some(3);
    let mut storage = MemoryOfflineAchievementStorage::new();

    let outcome = record_offline_achievement(&mut storage, &definition, &event(2), &context())
        .expect("evaluate event");

    assert_eq!(
        outcome,
        OfflineAwardOutcome::ThresholdNotMet {
            observed: 2,
            required: 3
        }
    );
    assert!(storage.records().is_empty());
}

#[test]
fn once_per_player_is_idempotent_across_definition_versions() {
    let mut storage = MemoryOfflineAchievementStorage::new();
    let first = record_offline_achievement(&mut storage, &definition(1), &event(1), &context())
        .expect("first award");
    let OfflineAwardOutcome::Awarded(first) = first else {
        panic!("expected first award");
    };

    let second = record_offline_achievement(&mut storage, &definition(2), &event(1), &context())
        .expect("idempotent second award");
    let OfflineAwardOutcome::AlreadyAwarded(second) = second else {
        panic!("expected existing award");
    };

    assert_eq!(second.claim_id, first.claim_id);
    assert_eq!(second.version, 1);
    assert_eq!(storage.records().len(), 1);
}

#[test]
fn missing_required_evidence_preserves_local_award_but_blocks_claim_readiness() {
    let mut definition = definition(1);
    definition.accomplishment.requires_evidence = true;
    let mut storage = MemoryOfflineAchievementStorage::new();

    let outcome = record_offline_achievement(&mut storage, &definition, &event(1), &context())
        .expect("local award should still be recorded");
    let OfflineAwardOutcome::Awarded(record) = outcome else {
        panic!("expected local award");
    };

    assert_eq!(
        record.claim_readiness,
        OfflineClaimReadiness::MissingRequiredEvidence
    );
    assert_eq!(storage.records().len(), 1);
}

#[test]
fn direct_award_only_definition_preserves_local_award_but_blocks_claim_readiness() {
    let mut definition = definition(1);
    definition.policy.issuance_mode = AchievementIssuanceMode::DirectAwardOnly;
    let mut storage = MemoryOfflineAchievementStorage::new();

    let outcome = record_offline_achievement(&mut storage, &definition, &event(1), &context())
        .expect("local award should still be recorded");
    let OfflineAwardOutcome::Awarded(record) = outcome else {
        panic!("expected local award");
    };

    assert_eq!(
        record.claim_readiness,
        OfflineClaimReadiness::NotAllowedByIssuancePolicy
    );
    assert_eq!(storage.records().len(), 1);
}

#[test]
fn file_storage_preserves_the_exact_claim_across_restart() {
    let path = std::env::temp_dir().join(format!("eab-offline-{}.jsonl", Uuid::new_v4()));
    let mut storage = FileOfflineAchievementStorage::open(&path).expect("open storage");
    let outcome = record_offline_achievement(&mut storage, &definition(1), &event(1), &context())
        .expect("award");
    let OfflineAwardOutcome::Awarded(original) = outcome else {
        panic!("expected award");
    };
    drop(storage);

    let mut reopened = FileOfflineAchievementStorage::open(&path).expect("reopen storage");
    assert_eq!(reopened.records().len(), 1);
    assert_eq!(reopened.records()[0].claim_id, original.claim_id);

    let retry = record_offline_achievement(&mut reopened, &definition(1), &event(1), &context())
        .expect("retry after restart");
    let OfflineAwardOutcome::AlreadyAwarded(retried) = retry else {
        panic!("expected existing award");
    };
    assert_eq!(retried.claim_id, original.claim_id);
    assert_eq!(reopened.records().len(), 1);

    fs::remove_file(path).expect("remove test journal");
}

#[test]
fn file_storage_rejects_tampered_records() {
    let path = std::env::temp_dir().join(format!("eab-offline-{}.jsonl", Uuid::new_v4()));
    let mut storage = FileOfflineAchievementStorage::open(&path).expect("open storage");
    record_offline_achievement(&mut storage, &definition(1), &event(1), &context()).expect("award");
    drop(storage);

    let contents = fs::read_to_string(&path).expect("read journal");
    let tampered = contents.replace("\"event_value\":1", "\"event_value\":999");
    assert_ne!(tampered, contents, "test must alter the journal");
    fs::write(&path, tampered).expect("tamper journal");

    let error = match FileOfflineAchievementStorage::open(&path) {
        Ok(_) => panic!("tampered journal should fail"),
        Err(error) => error,
    };
    assert!(matches!(error, OfflineAchievementError::Integrity));

    fs::remove_file(path).expect("remove test journal");
}
