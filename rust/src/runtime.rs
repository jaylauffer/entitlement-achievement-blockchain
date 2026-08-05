use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use eab_core::{EabClaimAcknowledgement, EabClaimEnvelope};
use eab_wire::{
    ClaimStatusResponse, ProtocolErrorCode, ProtocolErrorResponse, SecureMessage,
    SubmitClaimResponse,
};
use loadngo_proactor::{ChannelPort, CompletionKind, Proactor, ProactorHandle};

use crate::achievement_registry::{AchievementDefinition, AchievementRegistry};
use crate::eab_node::{EabNodeService, EabNodeStatusProvider, StaticStatusProvider};
use crate::entitlement_registry::EntitlementDefinition;
use crate::hd::BitVec;
use crate::identity::player_id_from_session;
use crate::ledger_storage::{FileTopicLedgerStorage, LedgerStorage};
use crate::player_profile::profile_service::{
    AchievementClaim, AchievementClaimInput, AchievementClaimReviewAction, AwardRecord,
    PlayerProfile, PlayerProfileService, PlayerRewardState,
};
use crate::qcoin_ledger_storage::QCoinLedgerStorage;
use crate::quic_transport::{QuicSecureServer, QuicServerIdentity, SecureRequestHandler};
use crate::sled_ledger_storage::SledLedgerStorage;

pub struct EabRuntime {
    service: Arc<Mutex<PlayerProfileService>>,
    achievement_registry_path: PathBuf,
    handle: ProactorHandle<ChannelPort>,
    thread: Option<thread::JoinHandle<()>>,
    _node_service: Option<EabNodeService>,
    _status_provider: Arc<dyn EabNodeStatusProvider>,
}

impl EabRuntime {
    pub fn from_env(bind_ip: &str, bind_port: u16) -> io::Result<Self> {
        let storage_backend = env::var("LEDGER_BACKEND").unwrap_or_else(|_| "file".to_string());
        let (storage, status_provider): (
            Box<dyn LedgerStorage + Send + Sync>,
            Arc<dyn EabNodeStatusProvider>,
        ) = if storage_backend == "sled" {
            let path = env::var("LEDGER_DB_PATH").unwrap_or_else(|_| "ledger_db".to_string());
            (
                Box::new(SledLedgerStorage::new(path)),
                Arc::new(StaticStatusProvider::new("sled")),
            )
        } else if storage_backend == "qcoin" {
            let topic_path =
                env::var("LEDGER_TOPICS_PATH").unwrap_or_else(|_| "player_logs".to_string());
            let qcoin_outbox_path = env::var("QCOIN_OUTBOX_PATH")
                .or_else(|_| env::var("QCOIN_STATE_PATH"))
                .unwrap_or_else(|_| "qcoin_anchor_outbox.json".to_string());
            let storage = QCoinLedgerStorage::new(topic_path, qcoin_outbox_path);
            let status_provider = storage.status_provider();
            (Box::new(storage), status_provider)
        } else {
            (
                Box::new(FileTopicLedgerStorage::new("player_logs")),
                Arc::new(StaticStatusProvider::new("file")),
            )
        };

        Self::new(
            storage,
            status_provider,
            Some((bind_ip.to_string(), bind_port)),
        )
    }

    pub fn new(
        storage: Box<dyn LedgerStorage + Send + Sync>,
        status_provider: Arc<dyn EabNodeStatusProvider>,
        node_bind: Option<(String, u16)>,
    ) -> io::Result<Self> {
        let achievement_registry_path = env::var("ACHIEVEMENT_REGISTRY_PATH")
            .unwrap_or_else(|_| "achievement_registry.json".to_string());
        Self::new_with_achievement_registry_path(
            storage,
            status_provider,
            node_bind,
            achievement_registry_path,
        )
    }

    pub fn new_with_achievement_registry_path(
        storage: Box<dyn LedgerStorage + Send + Sync>,
        status_provider: Arc<dyn EabNodeStatusProvider>,
        node_bind: Option<(String, u16)>,
        achievement_registry_path: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let service = Arc::new(Mutex::new(PlayerProfileService::new(storage)));
        let proactor = Proactor::new(ChannelPort::new());
        let handle = proactor.handle();
        let thread = thread::spawn(move || {
            if let Err(err) = proactor.run_until_stopped() {
                eprintln!("EAB runtime core stopped with error: {err}");
            }
        });

        let node_service = match node_bind {
            Some((bind_ip, bind_port)) => EabNodeService::start_from_env_on_handle(
                bind_ip.as_str(),
                bind_port,
                handle.clone(),
            )
            .map_err(io_other)?,
            None => None,
        };

        Ok(Self {
            service,
            achievement_registry_path: achievement_registry_path.into(),
            handle,
            thread: Some(thread),
            _node_service: node_service,
            _status_provider: status_provider,
        })
    }

    fn exec<R, F>(&self, job: F) -> io::Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut PlayerProfileService) -> io::Result<R> + Send + 'static,
    {
        exec_on_runtime(&self.handle, Arc::clone(&self.service), job)
    }

    /// Starts the canonical claim/status QUIC adapter on this runtime's
    /// proactor. The caller owns the returned server guard.
    pub fn start_quic_claim_service(
        &self,
        bind_addr: SocketAddr,
        identity: QuicServerIdentity,
    ) -> io::Result<QuicSecureServer> {
        QuicSecureServer::start_secure_message_service_on_proactor(
            &self.handle,
            bind_addr,
            identity,
            self.secure_claim_handler(),
        )
        .map_err(|error| io_other(error.to_string()))
    }

    fn secure_claim_handler(&self) -> Arc<dyn SecureRequestHandler> {
        let handle = self.handle.clone();
        let service = Arc::clone(&self.service);
        let registry_path = self.achievement_registry_path.clone();
        Arc::new(move |request: SecureMessage| {
            handle_secure_claim_request(&handle, &service, &registry_path, request)
        })
    }

    pub fn create_profile(&self, player_id: &str, name: &str) -> io::Result<PlayerProfile> {
        let player_id = player_id.to_string();
        let name = name.to_string();
        self.exec(move |svc| svc.create_profile(&player_id, &name).cloned())
    }

    pub fn get_profile(&self, player_id: &str) -> io::Result<Option<PlayerProfile>> {
        let player_id = player_id.to_string();
        self.exec(move |svc| Ok(svc.get_profile(&player_id).cloned()))
    }

    pub fn get_reward_state(&self, player_id: &str) -> io::Result<Option<PlayerRewardState>> {
        let player_id = player_id.to_string();
        self.exec(move |svc| Ok(svc.get_reward_state(&player_id).cloned()))
    }

    pub fn get_achievement_claims(
        &self,
        player_id: &str,
    ) -> io::Result<Option<Vec<AchievementClaim>>> {
        let player_id = player_id.to_string();
        self.exec(move |svc| Ok(svc.get_achievement_claims(&player_id).cloned()))
    }

    pub fn get_achievement_claim(
        &self,
        player_id: &str,
        claim_id: &str,
    ) -> io::Result<Option<AchievementClaim>> {
        let player_id = player_id.to_string();
        let claim_id = claim_id.to_string();
        self.exec(move |svc| Ok(svc.get_achievement_claim(&player_id, &claim_id).cloned()))
    }

    pub fn set_vector(&self, player_id: &str, vec: BitVec) -> io::Result<()> {
        let player_id = player_id.to_string();
        self.exec(move |svc| svc.set_vector(&player_id, vec))
    }

    pub fn merge_vector(&self, player_id: &str, vec: BitVec) -> io::Result<()> {
        let player_id = player_id.to_string();
        self.exec(move |svc| svc.merge_vector(&player_id, &vec))
    }

    pub fn submit_achievement_claim(
        &self,
        player_id: &str,
        claim: AchievementClaimInput,
    ) -> io::Result<AchievementClaim> {
        let player_id = player_id.to_string();
        self.exec(move |svc| svc.submit_achievement_claim(&player_id, claim))
    }

    pub fn acknowledge_canonical_claim(
        &self,
        player_id: &str,
        envelope: EabClaimEnvelope,
    ) -> io::Result<EabClaimAcknowledgement> {
        let player_id = player_id.to_string();
        let achievement_registry_path = self.achievement_registry_path.clone();
        self.exec(move |svc| {
            let registry = AchievementRegistry::load(&achievement_registry_path)?;
            let record = &envelope.record;
            let definition = registry
                .get(
                    &record.developer,
                    &record.game,
                    &record.achievement_id,
                    record.version,
                )
                .cloned();
            svc.acknowledge_canonical_claim(&player_id, envelope, definition.as_ref())
        })
    }

    pub fn get_claim_acknowledgement(
        &self,
        player_id: &str,
        claim_id: &str,
    ) -> io::Result<Option<EabClaimAcknowledgement>> {
        let player_id = player_id.to_string();
        let claim_id = claim_id.to_string();
        self.exec(move |svc| {
            Ok(svc
                .get_claim_acknowledgement(&player_id, &claim_id)
                .cloned())
        })
    }

    pub fn review_achievement_claim(
        &self,
        player_id: &str,
        claim_id: &str,
        reviewer: &str,
        action: AchievementClaimReviewAction,
        review_note: Option<String>,
        achievement: Option<AchievementDefinition>,
    ) -> io::Result<(AchievementClaim, Option<AwardRecord>)> {
        let player_id = player_id.to_string();
        let claim_id = claim_id.to_string();
        let reviewer = reviewer.to_string();
        self.exec(move |svc| {
            svc.review_achievement_claim(
                &player_id,
                &claim_id,
                &reviewer,
                action,
                review_note,
                achievement.as_ref(),
            )
        })
    }

    pub fn award_achievement(
        &self,
        player_id: &str,
        achievement: AchievementDefinition,
    ) -> io::Result<AwardRecord> {
        let player_id = player_id.to_string();
        self.exec(move |svc| svc.award_achievement(&player_id, &achievement))
    }

    pub fn award_entitlement(
        &self,
        player_id: &str,
        entitlement: EntitlementDefinition,
        quantity: u32,
        expiration_date: Option<String>,
    ) -> io::Result<AwardRecord> {
        let player_id = player_id.to_string();
        self.exec(move |svc| {
            svc.award_entitlement(&player_id, &entitlement, quantity, expiration_date)
        })
    }
}

impl Drop for EabRuntime {
    fn drop(&mut self) {
        let _ = self.handle.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn exec_on_runtime<R, F>(
    handle: &ProactorHandle<ChannelPort>,
    service: Arc<Mutex<PlayerProfileService>>,
    job: F,
) -> io::Result<R>
where
    R: Send + 'static,
    F: FnOnce(&mut PlayerProfileService) -> io::Result<R> + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel(1);
    handle
        .enqueue(CompletionKind::Job, 0, move |_| {
            let result = service
                .lock()
                .map_err(|_| io_other("EAB runtime service lock poisoned"))
                .and_then(|mut svc| job(&mut svc));
            let _ = tx.send(result);
        })
        .map_err(|err| io_other(format!("failed to enqueue EAB runtime job: {err}")))?;
    rx.recv()
        .map_err(|err| io_other(format!("failed to receive EAB runtime job result: {err}")))?
}

fn handle_secure_claim_request(
    handle: &ProactorHandle<ChannelPort>,
    service: &Arc<Mutex<PlayerProfileService>>,
    registry_path: &Path,
    request: SecureMessage,
) -> SecureMessage {
    match request {
        SecureMessage::SubmitClaimRequest(request) => {
            let request_id = request.request_id;
            let Some(player_id) = player_id_from_session(&request.session_token) else {
                return protocol_error(
                    request_id,
                    ProtocolErrorCode::AuthenticationFailed,
                    false,
                    "authentication failed",
                );
            };
            let envelope = request.envelope;
            let registry_path = registry_path.to_path_buf();
            let result = exec_on_runtime(handle, Arc::clone(service), move |svc| {
                let registry = AchievementRegistry::load(&registry_path)?;
                let record = &envelope.record;
                let definition = registry
                    .get(
                        &record.developer,
                        &record.game,
                        &record.achievement_id,
                        record.version,
                    )
                    .cloned();
                svc.acknowledge_canonical_claim(&player_id, envelope, definition.as_ref())
            });
            match result {
                Ok(acknowledgement) => SecureMessage::SubmitClaimResponse(SubmitClaimResponse {
                    request_id,
                    acknowledgement,
                }),
                Err(error) if error.kind() == io::ErrorKind::NotFound => protocol_error(
                    request_id,
                    ProtocolErrorCode::InvalidRequest,
                    false,
                    "player profile not found",
                ),
                Err(_) => protocol_error(
                    request_id,
                    ProtocolErrorCode::Internal,
                    true,
                    "claim service unavailable",
                ),
            }
        }
        SecureMessage::ClaimStatusRequest(request) => {
            let request_id = request.request_id;
            let Some(player_id) = player_id_from_session(&request.session_token) else {
                return protocol_error(
                    request_id,
                    ProtocolErrorCode::AuthenticationFailed,
                    false,
                    "authentication failed",
                );
            };
            let claim_id = request.claim_id;
            let response_claim_id = claim_id.clone();
            let result = exec_on_runtime(handle, Arc::clone(service), move |svc| {
                Ok(svc
                    .get_claim_acknowledgement(&player_id, &claim_id)
                    .cloned())
            });
            match result {
                Ok(acknowledgement) => SecureMessage::ClaimStatusResponse(ClaimStatusResponse {
                    request_id,
                    claim_id: response_claim_id,
                    acknowledgement,
                }),
                Err(_) => protocol_error(
                    request_id,
                    ProtocolErrorCode::Internal,
                    true,
                    "claim service unavailable",
                ),
            }
        }
        unexpected => protocol_error(
            unexpected.request_id(),
            ProtocolErrorCode::UnsupportedMessage,
            false,
            "request message required",
        ),
    }
}

fn protocol_error(
    request_id: [u8; 16],
    code: ProtocolErrorCode,
    retryable: bool,
    detail: &str,
) -> SecureMessage {
    SecureMessage::ProtocolErrorResponse(ProtocolErrorResponse {
        request_id,
        code,
        retryable,
        detail: detail.to_string(),
    })
}

fn io_other(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::issue_test_session;
    use crate::quic_transport::secure_message_round_trip;
    use eab_core::{
        record_offline_achievement, AchievementAccomplishment, AchievementIssuanceMode,
        AchievementRepeatability, AchievementVisibility, EabClaimDecisionCode, EabClaimDisposition,
        MemoryOfflineAchievementStorage, OfflineAchievementContext, OfflineAchievementEvent,
        OfflineAwardOutcome,
    };
    use eab_game_sdk::{EabClaimTransport, QuicEabClaimTransport};
    use eab_wire::{ClaimStatusRequest, ProtocolErrorCode, SecureMessage};
    use uuid::Uuid;

    fn definition() -> AchievementDefinition {
        AchievementDefinition::new(
            "pudding",
            "secure-flight",
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

    fn envelope() -> EabClaimEnvelope {
        let mut storage = MemoryOfflineAchievementStorage::new();
        let context = OfflineAchievementContext {
            local_player_id: "untrusted-local-slot".into(),
            save_id: "save-1".into(),
            installation_id: "install-1".into(),
            session_id: "offline-session".into(),
            client_sequence: 1,
            game_build: "1.0.0".into(),
        };
        let event = OfflineAchievementEvent {
            event_key: "flight_completed".into(),
            value: 1,
            occurred_at: "2026-08-06T12:00:00Z".into(),
            evidence: Some("local-run-receipt".into()),
        };
        let OfflineAwardOutcome::Awarded(record) =
            record_offline_achievement(&mut storage, &definition(), &event, &context).unwrap()
        else {
            panic!("expected offline award");
        };
        EabClaimEnvelope::try_from(&record).unwrap()
    }

    #[test]
    fn pinned_quic_claim_submission_uses_authenticated_session_binding_and_reconciles() {
        let root = std::env::temp_dir().join(format!("eab-quic-claim-{}", Uuid::new_v4()));
        let ledger_path = root.join("ledger");
        let registry_path = root.join("achievement_registry.json");
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = AchievementRegistry::default();
        registry.insert(definition());
        registry.save(&registry_path).unwrap();

        let runtime = EabRuntime::new_with_achievement_registry_path(
            Box::new(FileTopicLedgerStorage::new(&ledger_path)),
            Arc::new(StaticStatusProvider::new("test")),
            None,
            &registry_path,
        )
        .unwrap();
        let player_id = Uuid::new_v4().to_string();
        runtime.create_profile(&player_id, "Secure Pilot").unwrap();
        let session_token = issue_test_session(&player_id);

        let identity = QuicServerIdentity::generate_for_spike().unwrap();
        let pin = identity.certificate_fingerprint();
        let server = runtime
            .start_quic_claim_service("127.0.0.1:0".parse().unwrap(), identity)
            .unwrap();
        let envelope = envelope();
        let claim_id = envelope.claim_id().to_string();
        let transport =
            QuicEabClaimTransport::new(server.local_addr(), pin, session_token).unwrap();
        let acknowledgement = transport.submit_claim(&envelope.record).unwrap();

        assert_eq!(acknowledgement.claim_id, claim_id);
        assert_eq!(
            acknowledgement.disposition,
            EabClaimDisposition::Acknowledged
        );
        assert_eq!(acknowledgement.code, EabClaimDecisionCode::Acknowledged);
        assert_eq!(
            runtime
                .get_reward_state(&player_id)
                .unwrap()
                .unwrap()
                .achievements
                .len(),
            1
        );

        assert_eq!(
            transport.claim_status(&claim_id).unwrap().unwrap(),
            acknowledgement
        );

        let unauthorized = secure_message_round_trip(
            server.local_addr(),
            pin,
            SecureMessage::ClaimStatusRequest(ClaimStatusRequest {
                request_id: [0x33; 16],
                session_token: "invalid-session".into(),
                claim_id,
            }),
        )
        .unwrap();
        let SecureMessage::ProtocolErrorResponse(error) = unauthorized else {
            panic!("expected authentication error");
        };
        assert_eq!(error.code, ProtocolErrorCode::AuthenticationFailed);
        assert!(!error.retryable);

        drop(server);
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
}
