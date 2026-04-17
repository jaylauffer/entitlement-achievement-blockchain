use std::env;
use std::io;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use loadngo_proactor::{ChannelPort, CompletionKind, Proactor, ProactorHandle};

use crate::achievement_registry::AchievementDefinition;
use crate::eab_node::{
    EabNodeCommandHandler, EabNodeService, EabNodeStatusProvider, StaticStatusProvider,
};
use crate::entitlement_registry::EntitlementDefinition;
use crate::hd::BitVec;
use crate::ledger_storage::{FileTopicLedgerStorage, LedgerStorage};
use crate::player_profile::profile_service::{
    AchievementClaim, AchievementClaimInput, AchievementClaimReviewAction, AwardRecord,
    PlayerProfile, PlayerProfileService, PlayerRewardState,
};
use crate::qcoin_ledger_storage::QCoinLedgerStorage;
use crate::sled_ledger_storage::SledLedgerStorage;

pub struct EabRuntime {
    service: Arc<Mutex<PlayerProfileService>>,
    handle: ProactorHandle<ChannelPort>,
    thread: Option<thread::JoinHandle<()>>,
    node_service: Option<EabNodeService>,
    _status_provider: Arc<dyn EabNodeStatusProvider>,
}

struct RuntimeCommandHandler {
    service: Arc<Mutex<PlayerProfileService>>,
}

impl EabNodeCommandHandler for RuntimeCommandHandler {
    fn award_achievement(
        &self,
        player_id: &str,
        achievement: &AchievementDefinition,
    ) -> io::Result<AwardRecord> {
        self.service
            .lock()
            .map_err(|_| io_other("EAB runtime service lock poisoned"))?
            .award_achievement(player_id, achievement)
    }
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
                Arc::clone(&status_provider),
                Some(Arc::new(RuntimeCommandHandler {
                    service: Arc::clone(&service),
                })),
                handle.clone(),
            )
            .map_err(io_other)?,
            None => None,
        };

        Ok(Self {
            service,
            handle,
            thread: Some(thread),
            node_service,
            _status_provider: status_provider,
        })
    }

    fn exec<R, F>(&self, job: F) -> io::Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut PlayerProfileService) -> io::Result<R> + Send + 'static,
    {
        let service = Arc::clone(&self.service);
        let (tx, rx) = mpsc::sync_channel(1);
        self.handle
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

    pub fn award_achievement_via_node(
        &self,
        target: std::net::SocketAddr,
        player_id: &str,
        achievement: AchievementDefinition,
    ) -> io::Result<AwardRecord> {
        let node_service = self
            .node_service
            .as_ref()
            .ok_or_else(|| io_other("EAB node service is not enabled"))?;
        node_service
            .award_achievement_remote(target, player_id, achievement)
            .map_err(io_other)
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

fn io_other(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message.into())
}
