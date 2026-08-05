use eab_core::{
    record_offline_achievement, AchievementAccomplishment, AchievementDefinition,
    AchievementIssuanceMode, EabClaimEnvelope, EabClaimEnvelopeError,
    MemoryOfflineAchievementStorage, OfflineAchievementContext, OfflineAchievementEvent,
    OfflineAwardOutcome,
};

fn record(direct_award_only: bool) -> eab_core::OfflineAchievementRecord {
    let mut definition = AchievementDefinition::new(
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
    if direct_award_only {
        definition.policy.issuance_mode = AchievementIssuanceMode::DirectAwardOnly;
    }
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
    record
}

#[test]
fn canonical_envelope_preserves_the_complete_offline_record() {
    let record = record(false);

    let envelope = EabClaimEnvelope::try_from(&record).expect("canonical envelope");

    assert_eq!(envelope.claim_id(), record.claim_id);
    assert_eq!(envelope.record, record);
    assert_eq!(
        serde_json::from_str::<EabClaimEnvelope>(
            &serde_json::to_string(&envelope).expect("serialize envelope")
        )
        .expect("deserialize envelope"),
        envelope
    );
}

#[test]
fn canonical_envelope_rejects_tampered_record() {
    let mut record = record(false);
    record.game_build = "tampered-build".into();

    let error = EabClaimEnvelope::try_from(&record).expect_err("tampered record must fail");

    assert!(matches!(error, EabClaimEnvelopeError::InvalidRecord(_)));
}

#[test]
fn canonical_envelope_rejects_record_that_is_not_claim_ready() {
    let record = record(true);

    let error = EabClaimEnvelope::try_from(&record).expect_err("non-ready record must fail");

    assert!(matches!(error, EabClaimEnvelopeError::NotReady(_)));
}
