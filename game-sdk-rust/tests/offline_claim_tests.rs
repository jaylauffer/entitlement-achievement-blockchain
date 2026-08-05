use eab_game_sdk::{
    record_offline_achievement, AchievementAccomplishment, AchievementDefinition,
    AchievementIssuanceMode, MemoryOfflineAchievementStorage, OfflineAchievementContext,
    OfflineAchievementEvent, OfflineAwardOutcome, SdkError, SubmitAchievementClaimRequest,
};

#[test]
fn offline_record_maps_to_online_claim_without_changing_identity() {
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
        evidence: Some("local run receipt".into()),
    };
    let context = OfflineAchievementContext {
        local_player_id: "player-slot-1".into(),
        save_id: "save-1".into(),
        installation_id: "installation-1".into(),
        session_id: "session-1".into(),
        client_sequence: 7,
        game_build: "1.0.0".into(),
    };
    let mut storage = MemoryOfflineAchievementStorage::new();

    let outcome = record_offline_achievement(&mut storage, &definition, &event, &context)
        .expect("offline award");
    let OfflineAwardOutcome::Awarded(record) = outcome else {
        panic!("expected award");
    };
    let request = SubmitAchievementClaimRequest::try_from(&record).expect("claim-ready record");

    assert_eq!(request.claim_id, record.claim_id);
    assert_eq!(request.session_id, record.session_id);
    assert_eq!(request.client_sequence, record.client_sequence);
    assert_eq!(request.claimed_at, record.earned_at_local);
    assert_eq!(request.achievement_id, record.achievement_id);
    assert_eq!(request.evidence, record.evidence);
}

#[test]
fn sdk_refuses_to_submit_a_local_record_that_is_not_claim_ready() {
    let mut definition = AchievementDefinition::new(
        "pudding",
        "solo-flight",
        "local-only-flight",
        1,
        "Local Flight",
        "Complete a local-only flight",
    )
    .with_accomplishment(AchievementAccomplishment {
        summary: "Complete one local-only flight".into(),
        event_key: Some("flight_completed".into()),
        threshold: Some(1),
        requires_evidence: false,
    });
    definition.policy.issuance_mode = AchievementIssuanceMode::DirectAwardOnly;
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
        client_sequence: 8,
        game_build: "1.0.0".into(),
    };
    let mut storage = MemoryOfflineAchievementStorage::new();
    let outcome = record_offline_achievement(&mut storage, &definition, &event, &context)
        .expect("local award");
    let OfflineAwardOutcome::Awarded(record) = outcome else {
        panic!("expected local award");
    };

    let error = SubmitAchievementClaimRequest::try_from(&record)
        .expect_err("direct-only local record must not become a claim");

    assert!(matches!(error, SdkError::OfflineClaimNotReady(_)));
}
