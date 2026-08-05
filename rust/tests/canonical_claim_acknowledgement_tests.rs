use eab_core::{
    record_offline_achievement, AchievementAccomplishment, AchievementDefinition,
    AchievementIssuanceMode, AchievementRepeatability, AchievementVisibility, EabClaimDecisionCode,
    EabClaimDisposition, EabClaimEnvelope, MemoryOfflineAchievementStorage,
    OfflineAchievementContext, OfflineAchievementEvent, OfflineAwardOutcome,
};
use loadngo_eab::ledger_storage::FileTopicLedgerStorage;
use loadngo_eab::player_profile::profile_service::PlayerProfileService;
use uuid::Uuid;

fn definition() -> AchievementDefinition {
    AchievementDefinition::new(
        "pudding",
        "solo-flight",
        "first-flight",
        1,
        "First Flight",
        "Complete a successful flight",
    )
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

fn envelope(local_player_id: &str, sequence: u64) -> EabClaimEnvelope {
    let context = OfflineAchievementContext {
        local_player_id: local_player_id.into(),
        save_id: format!("save-{sequence}"),
        installation_id: format!("install-{local_player_id}"),
        session_id: format!("session-{sequence}"),
        client_sequence: sequence,
        game_build: "1.0.0".into(),
    };
    let event = OfflineAchievementEvent {
        event_key: "flight_completed".into(),
        value: 1,
        occurred_at: format!("2026-08-05T12:{sequence:02}:00Z"),
        evidence: Some("local-run-receipt".into()),
    };
    let mut storage = MemoryOfflineAchievementStorage::new();
    let OfflineAwardOutcome::Awarded(record) =
        record_offline_achievement(&mut storage, &definition(), &event, &context)
            .expect("record offline achievement")
    else {
        panic!("expected offline award");
    };
    EabClaimEnvelope::try_from(&record).expect("canonical envelope")
}

fn service_and_player(test_name: &str) -> (String, String, PlayerProfileService) {
    let directory = std::env::temp_dir()
        .join(format!("eab-{test_name}-{}", Uuid::new_v4()))
        .to_string_lossy()
        .into_owned();
    let player_id = Uuid::new_v4().to_string();
    let mut service = PlayerProfileService::new(Box::new(FileTopicLedgerStorage::new(&directory)));
    service
        .create_profile(&player_id, "Offline Pilot")
        .expect("create profile");
    (directory, player_id, service)
}

#[test]
fn valid_canonical_claim_is_acknowledged_awarded_and_idempotent() {
    let (directory, player_id, mut service) = service_and_player("canonical-idempotent");
    let envelope = envelope("slot-1", 1);

    let first = service
        .acknowledge_canonical_claim(&player_id, envelope.clone(), Some(&definition()))
        .expect("acknowledge claim");
    let retry = service
        .acknowledge_canonical_claim(&player_id, envelope.clone(), Some(&definition()))
        .expect("retry claim");

    assert_eq!(first.disposition, EabClaimDisposition::Acknowledged);
    assert_eq!(first.code, EabClaimDecisionCode::Acknowledged);
    assert!(first.award.is_some());
    assert_eq!(retry, first);
    assert_eq!(
        service
            .get_reward_state(&player_id)
            .expect("reward state")
            .achievements
            .len(),
        1
    );
    assert_eq!(
        service
            .get_claim_acknowledgement(&player_id, envelope.claim_id())
            .expect("stored acknowledgement"),
        &first
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn acknowledgement_and_award_survive_restart_without_duplicate_reward() {
    let (directory, player_id, mut service) = service_and_player("canonical-restart");
    let envelope = envelope("slot-1", 2);
    let first = service
        .acknowledge_canonical_claim(&player_id, envelope.clone(), Some(&definition()))
        .expect("acknowledge claim");
    drop(service);

    let mut restarted =
        PlayerProfileService::new(Box::new(FileTopicLedgerStorage::new(&directory)));
    let retry = restarted
        .acknowledge_canonical_claim(&player_id, envelope, Some(&definition()))
        .expect("retry after restart");

    assert_eq!(retry, first);
    assert_eq!(
        restarted
            .get_reward_state(&player_id)
            .expect("reward state")
            .achievements
            .len(),
        1
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn separate_offline_occurrences_do_not_duplicate_once_per_player_award() {
    let (directory, player_id, mut service) = service_and_player("canonical-logical-dedupe");
    let first = service
        .acknowledge_canonical_claim(&player_id, envelope("slot-a", 3), Some(&definition()))
        .expect("first claim");
    let second = service
        .acknowledge_canonical_claim(&player_id, envelope("slot-b", 4), Some(&definition()))
        .expect("second claim");

    assert_eq!(first.code, EabClaimDecisionCode::Acknowledged);
    assert_eq!(second.disposition, EabClaimDisposition::Acknowledged);
    assert_eq!(second.code, EabClaimDecisionCode::AlreadyAcknowledged);
    assert_eq!(second.award, first.award);
    assert_eq!(
        service
            .get_reward_state(&player_id)
            .expect("reward state")
            .achievements
            .len(),
        1
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn authority_returns_structured_definition_conflicts_without_awarding() {
    let (directory, player_id, mut service) = service_and_player("canonical-conflicts");
    let missing = service
        .acknowledge_canonical_claim(&player_id, envelope("slot-a", 5), None)
        .expect("missing definition decision");
    assert_eq!(missing.disposition, EabClaimDisposition::Conflict);
    assert_eq!(missing.code, EabClaimDecisionCode::DefinitionNotFound);

    let mut changed_definition = definition();
    changed_definition.accomplishment.summary = "Authoritative definition changed".into();
    let digest_mismatch = service
        .acknowledge_canonical_claim(&player_id, envelope("slot-b", 6), Some(&changed_definition))
        .expect("digest mismatch decision");
    assert_eq!(digest_mismatch.disposition, EabClaimDisposition::Conflict);
    assert_eq!(
        digest_mismatch.code,
        EabClaimDecisionCode::DefinitionDigestMismatch
    );
    assert!(service
        .get_reward_state(&player_id)
        .expect("reward state")
        .achievements
        .is_empty());
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn authority_rejects_tampered_envelope_even_if_client_bypasses_sdk_validation() {
    let (directory, player_id, mut service) = service_and_player("canonical-tamper");
    let mut tampered = envelope("slot-a", 8);
    tampered.record.event_value = 99;

    let decision = service
        .acknowledge_canonical_claim(&player_id, tampered, Some(&definition()))
        .expect("tampered envelope decision");

    assert_eq!(decision.disposition, EabClaimDisposition::Rejected);
    assert_eq!(decision.code, EabClaimDecisionCode::InvalidEnvelope);
    assert!(decision.award.is_none());
    assert!(service
        .get_reward_state(&player_id)
        .expect("reward state")
        .achievements
        .is_empty());
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn reusing_claim_id_with_different_payload_returns_conflict() {
    let (directory, player_id, mut service) = service_and_player("canonical-claim-id-conflict");
    let original = envelope("slot-a", 7);
    service
        .acknowledge_canonical_claim(&player_id, original.clone(), Some(&definition()))
        .expect("original claim");

    let mut conflicting = original;
    conflicting.schema_version += 1;
    let decision = service
        .acknowledge_canonical_claim(&player_id, conflicting, Some(&definition()))
        .expect("conflicting claim");

    assert_eq!(decision.disposition, EabClaimDisposition::Conflict);
    assert_eq!(decision.code, EabClaimDecisionCode::ClaimIdPayloadMismatch);
    assert_eq!(
        service
            .get_reward_state(&player_id)
            .expect("reward state")
            .achievements
            .len(),
        1
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}
