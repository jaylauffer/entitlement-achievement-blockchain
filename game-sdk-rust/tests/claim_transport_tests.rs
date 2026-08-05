use std::io;
use std::sync::Mutex;

use eab_game_sdk::{
    record_offline_achievement, AchievementAccomplishment, AchievementDefinition,
    AchievementIssuanceMode, EabClaimAcknowledgement, EabClaimDecisionCode, EabClaimDisposition,
    EabClaimTransport, EabClient, MemoryOfflineAchievementStorage, OfflineAchievementContext,
    OfflineAchievementEvent, OfflineAchievementRecord, OfflineAwardOutcome, SdkError,
    EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION,
};

fn offline_record(direct_award_only: bool) -> OfflineAchievementRecord {
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

#[derive(Default)]
struct RecordingClaimTransport {
    submitted_claim_ids: Mutex<Vec<String>>,
}

impl EabClaimTransport for RecordingClaimTransport {
    type Error = io::Error;

    fn submit_claim(
        &self,
        record: &OfflineAchievementRecord,
    ) -> Result<EabClaimAcknowledgement, Self::Error> {
        self.submitted_claim_ids
            .lock()
            .expect("recording transport lock")
            .push(record.claim_id.clone());
        Ok(claim_from_record(record))
    }

    fn claim_status(&self, claim_id: &str) -> Result<Option<EabClaimAcknowledgement>, Self::Error> {
        let seen = self
            .submitted_claim_ids
            .lock()
            .expect("recording transport lock")
            .iter()
            .any(|known| known == claim_id);
        Ok(seen.then(|| {
            let mut record = offline_record(false);
            record.claim_id = claim_id.to_string();
            claim_from_record(&record)
        }))
    }
}

#[test]
fn transport_contract_preserves_offline_claim_identity_and_supports_status_lookup() {
    let record = offline_record(false);
    let transport = RecordingClaimTransport::default();

    let submitted = transport.submit_claim(&record).expect("submit claim");
    assert_eq!(submitted.claim_id, record.claim_id);
    assert_eq!(submitted.disposition, EabClaimDisposition::Acknowledged);

    let status = transport
        .claim_status(&record.claim_id)
        .expect("claim status")
        .expect("known claim");
    assert_eq!(status.claim_id, record.claim_id);

    assert_eq!(
        transport
            .submitted_claim_ids
            .lock()
            .expect("recording transport lock")
            .as_slice(),
        &[record.claim_id]
    );
}

#[test]
fn http_transport_owns_player_binding_and_refuses_non_ready_record_before_io() {
    let client = EabClient::new("http://127.0.0.1:1");
    let transport = client.claim_transport("player-123", "session-secret");
    let record = offline_record(true);

    assert_eq!(transport.player_id(), "player-123");
    let error = transport
        .submit_claim(&record)
        .expect_err("non-ready record should fail before network I/O");
    assert!(matches!(error, SdkError::OfflineClaimNotReady(_)));
}

fn claim_from_record(record: &OfflineAchievementRecord) -> EabClaimAcknowledgement {
    EabClaimAcknowledgement {
        schema_version: EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION,
        developer: record.developer.clone(),
        game: record.game.clone(),
        achievement_id: record.achievement_id.clone(),
        version: record.version,
        claim_id: record.claim_id.clone(),
        disposition: EabClaimDisposition::Acknowledged,
        code: EabClaimDecisionCode::Acknowledged,
        first_observed_at: "2026-08-05T12:01:00Z".into(),
        decided_at: Some("2026-08-05T12:01:00Z".into()),
        award: None,
    }
}
