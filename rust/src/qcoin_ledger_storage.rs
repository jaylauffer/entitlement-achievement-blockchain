use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::{env, io};

use loadngo_proactor::{ChannelPort, CompletionKind, Proactor, ProactorHandle};
use qcoin_types::{
    Block as QCoinBlock, Hash256, Output, Transaction as QCoinTransaction, TransactionCore,
    TransactionKind, TransactionWitness,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::blockchain::Block;
use crate::eab_node::{EabNodeStatusProvider, NodeStatusSnapshot};
use crate::ledger_storage::{FileTopicLedgerStorage, LedgerStorage};
use crate::player_profile::profile_service::AchievementClaim;

const QCOIN_WIRE_MAGIC: [u8; 4] = *b"QCN1";
const DEFAULT_QCOIN_NODE_PORT: u16 = 9700;
const OUTBOX_RETRY_DELAY: Duration = Duration::from_secs(5);
const ACCEPTED_RETRY_DELAY: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NodeInfo {
    wire_version: u16,
    min_compatible_wire_version: u16,
    software_version: String,
    chain_id: u32,
    node_public_key_hex: String,
    capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TipResponse {
    height: u64,
    tip_hash_hex: String,
    state_root_hex: String,
    last_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SubmitBlockResponse {
    accepted: bool,
    height: u64,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SubmitTransactionResponse {
    accepted: bool,
    tx_id_hex: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum WireMessage {
    PresenceAnnounce,
    NodeInfo(NodeInfo),
    TipRequest,
    TipResponse(TipResponse),
    BlockRequest {
        height: u64,
    },
    BlockResponse {
        height: u64,
        block: Option<QCoinBlock>,
    },
    SubmitBlock {
        block: QCoinBlock,
    },
    SubmitBlockResponse(SubmitBlockResponse),
    TransactionAnnounce {
        tx_id: Hash256,
    },
    TransactionRequest {
        tx_id: Hash256,
    },
    TransactionResponse {
        tx_id: Hash256,
        transaction: Option<QCoinTransaction>,
    },
    SubmitTransaction {
        transaction: QCoinTransaction,
    },
    SubmitTransactionResponse(SubmitTransactionResponse),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
enum AnchorProgress {
    #[default]
    PendingSubmission,
    AcceptedNotIncluded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnchorOutboxEntry {
    player_id: Uuid,
    block: Block,
    transaction: QCoinTransaction,
    #[serde(default)]
    progress: AnchorProgress,
    attempts: u32,
    #[serde(default)]
    last_submitted_unix_seconds: Option<u64>,
    #[serde(default)]
    last_accepted_unix_seconds: Option<u64>,
    last_error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AnchorOutboxState {
    pending: Vec<AnchorOutboxEntry>,
}

#[derive(Debug, Clone, Default)]
struct AnchorRuntimeStatus {
    pending_entries: usize,
    pending_submission_entries: usize,
    accepted_not_included_entries: usize,
    last_anchor_accepted_unix_seconds: Option<u64>,
    last_anchor_included_unix_seconds: Option<u64>,
    last_anchor_success_unix_seconds: Option<u64>,
    last_anchor_error: Option<String>,
    last_anchor_error_unix_seconds: Option<u64>,
}

struct AnchorOutboxShared {
    path: PathBuf,
    file_lock: Mutex<()>,
    node_target: Option<SocketAddr>,
    status: Mutex<AnchorRuntimeStatus>,
}

/// Storage backend that keeps the canonical per-player logs while enqueueing
/// qcoin anchor transactions for asynchronous submission to a live qcoin node.
pub struct QCoinLedgerStorage {
    topic_storage: FileTopicLedgerStorage,
    outbox: Arc<AnchorOutboxShared>,
    worker: Option<ProactorHandle<ChannelPort>>,
}

struct QCoinStatusProvider {
    outbox: Arc<AnchorOutboxShared>,
}

impl QCoinLedgerStorage {
    /// `topic_base_path` keeps the per-player block logs used by profile rehydration.
    /// `qcoin_outbox_path` stores pending qcoin anchor submissions.
    pub fn new<P1: Into<PathBuf>, P2: Into<PathBuf>>(
        topic_base_path: P1,
        qcoin_outbox_path: P2,
    ) -> Self {
        Self::new_with_target(
            topic_base_path,
            qcoin_outbox_path,
            resolve_qcoin_node_target(),
        )
    }

    /// Explicit constructor for tests and controlled runtime wiring.
    pub fn new_with_target<P1: Into<PathBuf>, P2: Into<PathBuf>>(
        topic_base_path: P1,
        qcoin_outbox_path: P2,
        node_target: Option<SocketAddr>,
    ) -> Self {
        let outbox_path = qcoin_outbox_path.into();
        if let Some(parent) = outbox_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let initial_state = load_outbox_state_unlocked(&outbox_path).unwrap_or_default();
        let (pending_entries, pending_submission_entries, accepted_not_included_entries) =
            anchor_progress_counts(&initial_state);
        let outbox = Arc::new(AnchorOutboxShared {
            path: outbox_path,
            file_lock: Mutex::new(()),
            node_target,
            status: Mutex::new(AnchorRuntimeStatus {
                pending_entries,
                pending_submission_entries,
                accepted_not_included_entries,
                ..AnchorRuntimeStatus::default()
            }),
        });

        let worker = node_target.map(|_| spawn_anchor_worker(Arc::clone(&outbox)));

        Self {
            topic_storage: FileTopicLedgerStorage::new(topic_base_path),
            outbox,
            worker,
        }
    }

    pub fn status_provider(&self) -> Arc<dyn EabNodeStatusProvider> {
        Arc::new(QCoinStatusProvider {
            outbox: Arc::clone(&self.outbox),
        })
    }

    pub fn anchor_transaction_id(player_id: Uuid, block: &Block) -> io::Result<Hash256> {
        Ok(Self::make_anchor_tx(player_id, block)?.tx_id())
    }

    fn io_other(message: impl Into<String>) -> io::Error {
        io::Error::new(io::ErrorKind::Other, message.into())
    }

    fn player_owner_hash(player_id: Uuid) -> Hash256 {
        *blake3::hash(player_id.as_bytes()).as_bytes()
    }

    fn block_metadata_hash(block: &Block) -> io::Result<Hash256> {
        let json = serde_json::to_vec(block)
            .map_err(|err| Self::io_other(format!("failed to serialize block payload: {err}")))?;
        Ok(*blake3::hash(&json).as_bytes())
    }

    fn make_anchor_tx(player_id: Uuid, block: &Block) -> io::Result<QCoinTransaction> {
        let metadata_hash = Self::block_metadata_hash(block)?;

        Ok(QCoinTransaction {
            core: TransactionCore {
                kind: TransactionKind::Transfer,
                inputs: vec![],
                outputs: vec![Output {
                    owner_script_hash: Self::player_owner_hash(player_id),
                    assets: vec![],
                    metadata_hash: Some(metadata_hash),
                }],
            },
            witness: TransactionWitness::default(),
        })
    }

    fn enqueue_anchor(&self, player_id: Uuid, block: &Block) -> io::Result<()> {
        let tx = Self::make_anchor_tx(player_id, block)?;
        let mut state = load_outbox_state(&self.outbox)?;
        state.pending.push(AnchorOutboxEntry {
            player_id,
            block: block.clone(),
            transaction: tx,
            progress: AnchorProgress::PendingSubmission,
            attempts: 0,
            last_submitted_unix_seconds: None,
            last_accepted_unix_seconds: None,
            last_error: None,
        });
        save_outbox_state(&self.outbox, &state)?;
        update_status_from_outbox_state(&self.outbox, &state);
        Ok(())
    }
}

impl LedgerStorage for QCoinLedgerStorage {
    fn append_block(&self, player_id: Uuid, block: &Block) -> io::Result<()> {
        self.topic_storage.append_block(player_id, block)?;
        self.enqueue_anchor(player_id, block)?;
        if let Some(worker) = &self.worker {
            schedule_outbox_drain(worker, Arc::clone(&self.outbox), Duration::ZERO);
        }
        Ok(())
    }

    fn load_blocks(&self, player_id: Uuid) -> io::Result<Vec<Block>> {
        self.topic_storage.load_blocks(player_id)
    }

    fn list_player_ids(&self) -> io::Result<Vec<Uuid>> {
        self.topic_storage.list_player_ids()
    }

    fn load_achievement_claims(&self, player_id: Uuid) -> io::Result<Vec<AchievementClaim>> {
        self.topic_storage.load_achievement_claims(player_id)
    }

    fn save_achievement_claims(
        &self,
        player_id: Uuid,
        claims: &[AchievementClaim],
    ) -> io::Result<()> {
        self.topic_storage
            .save_achievement_claims(player_id, claims)
    }
}

impl EabNodeStatusProvider for QCoinStatusProvider {
    fn snapshot(&self) -> NodeStatusSnapshot {
        let status = self
            .outbox
            .status
            .lock()
            .expect("qcoin anchor status lock poisoned")
            .clone();
        NodeStatusSnapshot {
            ledger_backend: "qcoin".to_string(),
            qcoin_node_target: self.outbox.node_target.map(|target| target.to_string()),
            anchor_outbox_pending: status.pending_entries,
            anchor_outbox_pending_submission: status.pending_submission_entries,
            anchor_outbox_accepted_not_included: status.accepted_not_included_entries,
            last_anchor_accepted_unix_seconds: status.last_anchor_accepted_unix_seconds,
            last_anchor_included_unix_seconds: status.last_anchor_included_unix_seconds,
            last_anchor_success_unix_seconds: status.last_anchor_success_unix_seconds,
            last_anchor_error: status.last_anchor_error,
            last_anchor_error_unix_seconds: status.last_anchor_error_unix_seconds,
        }
    }
}

fn spawn_anchor_worker(outbox: Arc<AnchorOutboxShared>) -> ProactorHandle<ChannelPort> {
    let proactor = Proactor::new(ChannelPort::new());
    let handle = proactor.handle();
    let thread_proactor = proactor;
    thread::spawn(move || {
        if let Err(err) = thread_proactor.run_until_stopped() {
            eprintln!("EAB qcoin anchor worker stopped with error: {err}");
        }
    });
    schedule_outbox_drain(&handle, outbox, Duration::ZERO);
    handle
}

fn schedule_outbox_drain(
    handle: &ProactorHandle<ChannelPort>,
    outbox: Arc<AnchorOutboxShared>,
    delay: Duration,
) {
    let next_handle = handle.clone();
    let task_outbox = Arc::clone(&outbox);
    let task = move |_completion| match process_outbox(&task_outbox) {
        Ok(true) => {
            schedule_outbox_drain(&next_handle, Arc::clone(&task_outbox), OUTBOX_RETRY_DELAY)
        }
        Ok(false) => {}
        Err(err) => {
            eprintln!("EAB qcoin anchor outbox processing failed: {err}");
            schedule_outbox_drain(&next_handle, Arc::clone(&task_outbox), OUTBOX_RETRY_DELAY);
        }
    };

    let result = if delay.is_zero() {
        handle.enqueue(CompletionKind::Job, 0, task)
    } else {
        handle.defer_for(delay, CompletionKind::Timer, 0, task)
    };

    if let Err(err) = result {
        eprintln!("Failed to schedule EAB qcoin anchor outbox work: {err}");
    }
}

fn process_outbox(outbox: &AnchorOutboxShared) -> io::Result<bool> {
    let mut state = load_outbox_state(outbox)?;
    if state.pending.is_empty() {
        update_status_from_outbox_state(outbox, &state);
        return Ok(false);
    }

    let target = outbox.node_target.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "qcoin node target is not configured",
        )
    })?;

    let mut remaining = Vec::new();
    for mut entry in state.pending.drain(..) {
        let tx_id = entry.transaction.tx_id();

        if qcoin_transaction_is_included(target, tx_id)? {
            record_anchor_included(outbox);
            continue;
        }

        if !should_submit_anchor(&entry) {
            remaining.push(entry);
            continue;
        }

        entry.last_submitted_unix_seconds = Some(current_unix_timestamp());

        match submit_transaction_to_qcoin(target, &entry.transaction) {
            Ok(response) if response.accepted => {
                entry.progress = AnchorProgress::AcceptedNotIncluded;
                entry.last_accepted_unix_seconds = Some(current_unix_timestamp());
                entry.last_error = None;
                record_anchor_accepted(outbox);
                remaining.push(entry);
            }
            Ok(response) if response.message.contains("already pending") => {
                entry.progress = AnchorProgress::AcceptedNotIncluded;
                entry
                    .last_accepted_unix_seconds
                    .get_or_insert_with(current_unix_timestamp);
                entry.last_error = None;
                record_anchor_accepted(outbox);
                remaining.push(entry);
            }
            Ok(response) if response.message.contains("already committed") => {
                if qcoin_transaction_is_included(target, tx_id)? {
                    record_anchor_included(outbox);
                } else {
                    entry.progress = AnchorProgress::AcceptedNotIncluded;
                    entry
                        .last_accepted_unix_seconds
                        .get_or_insert_with(current_unix_timestamp);
                    entry.last_error = None;
                    record_anchor_accepted(outbox);
                    remaining.push(entry);
                }
            }
            Ok(response) => {
                entry.attempts += 1;
                entry.last_error = Some(response.message);
                record_anchor_error(outbox, entry.last_error.clone());
                remaining.push(entry);
            }
            Err(err) => {
                entry.attempts += 1;
                entry.last_error = Some(err.to_string());
                record_anchor_error(outbox, entry.last_error.clone());
                remaining.push(entry);
            }
        }
    }

    state.pending = remaining;
    save_outbox_state(outbox, &state)?;
    update_status_from_outbox_state(outbox, &state);
    Ok(!state.pending.is_empty())
}

fn qcoin_transaction_is_included(target: SocketAddr, tx_id: Hash256) -> io::Result<bool> {
    let tip = fetch_qcoin_tip_http(target)?;
    for height in (1..=tip.height).rev() {
        let Some(block) = fetch_qcoin_block_http(target, height)? else {
            continue;
        };
        if block.transactions.iter().any(|tx| tx.tx_id() == tx_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn fetch_qcoin_tip_http(target: SocketAddr) -> io::Result<TipResponse> {
    let url = format!("{}/tip", qcoin_http_base(target));
    ureq::get(&url)
        .call()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("tip request failed: {err}")))?
        .into_json::<TipResponse>()
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tip parse failed: {err}"),
            )
        })
}

fn fetch_qcoin_block_http(target: SocketAddr, height: u64) -> io::Result<Option<QCoinBlock>> {
    let url = format!("{}/blocks/{height}", qcoin_http_base(target));
    let response = match ureq::get(&url).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(err) => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("block request failed: {err}"),
            ))
        }
    };
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("block read failed: {err}")))?;
    bincode::deserialize::<QCoinBlock>(&bytes)
        .map(Some)
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("block decode failed: {err}"),
            )
        })
}

fn qcoin_http_base(target: SocketAddr) -> String {
    match target {
        SocketAddr::V4(addr) => format!("http://{}:{}", addr.ip(), addr.port()),
        SocketAddr::V6(addr) => format!("http://[{}]:{}", addr.ip(), addr.port()),
    }
}

fn load_outbox_state(outbox: &AnchorOutboxShared) -> io::Result<AnchorOutboxState> {
    let _guard = outbox
        .file_lock
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "qcoin outbox lock poisoned"))?;
    load_outbox_state_unlocked(&outbox.path)
}

fn save_outbox_state(outbox: &AnchorOutboxShared, state: &AnchorOutboxState) -> io::Result<()> {
    let _guard = outbox
        .file_lock
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "qcoin outbox lock poisoned"))?;
    save_outbox_state_unlocked(&outbox.path, state)
}

fn load_outbox_state_unlocked(path: &Path) -> io::Result<AnchorOutboxState> {
    if !path.exists() {
        return Ok(AnchorOutboxState::default());
    }
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    serde_json::from_str(&contents).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "failed to parse qcoin anchor outbox {}: {err}",
                path.display()
            ),
        )
    })
}

fn save_outbox_state_unlocked(path: &Path, state: &AnchorOutboxState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("tmp");
    let payload = serde_json::to_vec_pretty(state).map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "failed to serialize qcoin anchor outbox {}: {err}",
                path.display()
            ),
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp_path)?;
    file.write_all(&payload)?;
    file.flush()?;
    file.sync_all()?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn should_submit_anchor(entry: &AnchorOutboxEntry) -> bool {
    match entry.progress {
        AnchorProgress::PendingSubmission => true,
        AnchorProgress::AcceptedNotIncluded => entry
            .last_accepted_unix_seconds
            .map(|accepted| {
                current_unix_timestamp().saturating_sub(accepted) >= ACCEPTED_RETRY_DELAY.as_secs()
            })
            .unwrap_or(true),
    }
}

fn anchor_progress_counts(state: &AnchorOutboxState) -> (usize, usize, usize) {
    let pending_entries = state.pending.len();
    let accepted_not_included_entries = state
        .pending
        .iter()
        .filter(|entry| entry.progress == AnchorProgress::AcceptedNotIncluded)
        .count();
    let pending_submission_entries = pending_entries.saturating_sub(accepted_not_included_entries);
    (
        pending_entries,
        pending_submission_entries,
        accepted_not_included_entries,
    )
}

fn update_status_from_outbox_state(outbox: &AnchorOutboxShared, state: &AnchorOutboxState) {
    let (pending_entries, pending_submission_entries, accepted_not_included_entries) =
        anchor_progress_counts(state);
    if let Ok(mut status) = outbox.status.lock() {
        status.pending_entries = pending_entries;
        status.pending_submission_entries = pending_submission_entries;
        status.accepted_not_included_entries = accepted_not_included_entries;
    }
}

fn record_anchor_accepted(outbox: &AnchorOutboxShared) {
    if let Ok(mut status) = outbox.status.lock() {
        status.last_anchor_accepted_unix_seconds = Some(current_unix_timestamp());
        status.last_anchor_error = None;
        status.last_anchor_error_unix_seconds = None;
    }
}

fn record_anchor_included(outbox: &AnchorOutboxShared) {
    if let Ok(mut status) = outbox.status.lock() {
        let now = current_unix_timestamp();
        status.last_anchor_included_unix_seconds = Some(now);
        status.last_anchor_success_unix_seconds = Some(now);
        status.last_anchor_error = None;
        status.last_anchor_error_unix_seconds = None;
    }
}

fn record_anchor_error(outbox: &AnchorOutboxShared, error: Option<String>) {
    if let Ok(mut status) = outbox.status.lock() {
        status.last_anchor_error = error;
        status.last_anchor_error_unix_seconds = Some(current_unix_timestamp());
    }
}

fn current_unix_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn resolve_qcoin_node_target() -> Option<SocketAddr> {
    if let Ok(target) = env::var("QCOIN_NODE_TARGET") {
        let trimmed = target.trim();
        if !trimmed.is_empty() {
            return resolve_target_spec(trimmed).ok();
        }
    }

    if let Ok(url) = env::var("QCOIN_NODE_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return derive_target_from_url(trimmed).ok();
        }
    }

    None
}

fn resolve_target_spec(spec: &str) -> io::Result<SocketAddr> {
    spec.to_socket_addrs()?.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no socket addresses resolved for {spec}"),
        )
    })
}

fn derive_target_from_url(url: &str) -> io::Result<SocketAddr> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .trim();
    if authority.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing authority in qcoin URL {url}"),
        ));
    }

    let target = if authority.contains(':') || authority.starts_with('[') {
        authority.to_string()
    } else {
        format!("{authority}:{DEFAULT_QCOIN_NODE_PORT}")
    };
    resolve_target_spec(&target)
}

fn submit_transaction_to_qcoin(
    target: SocketAddr,
    transaction: &QCoinTransaction,
) -> io::Result<SubmitTransactionResponse> {
    let bind_addr: SocketAddr = match target {
        SocketAddr::V4(_) => "0.0.0.0:0".parse().expect("valid IPv4 wildcard bind"),
        SocketAddr::V6(_) => "[::]:0".parse().expect("valid IPv6 wildcard bind"),
    };
    let socket = UdpSocket::bind(bind_addr)?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;

    let frame = encode_wire_message(&WireMessage::SubmitTransaction {
        transaction: transaction.clone(),
    })?;
    socket.send_to(&frame, target)?;

    let mut buf = [0u8; 64 * 1024];
    loop {
        let (len, source) = socket.recv_from(&mut buf)?;
        if source != target {
            continue;
        }

        match decode_wire_message(&buf[..len])? {
            WireMessage::SubmitTransactionResponse(response) => return Ok(response),
            WireMessage::PresenceAnnounce | WireMessage::NodeInfo(_) => continue,
            _ => continue,
        }
    }
}

fn encode_wire_message(message: &WireMessage) -> io::Result<Vec<u8>> {
    let mut frame = QCOIN_WIRE_MAGIC.to_vec();
    let payload = bincode::serialize(message)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_wire_message(frame: &[u8]) -> io::Result<WireMessage> {
    if frame.len() < QCOIN_WIRE_MAGIC.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame shorter than qcoin wire header",
        ));
    }
    if frame[..QCOIN_WIRE_MAGIC.len()] != QCOIN_WIRE_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame does not match qcoin wire magic",
        ));
    }
    bincode::deserialize(&frame[QCOIN_WIRE_MAGIC.len()..])
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use qcoin_crypto::{Dilithium2Scheme, PqSignatureScheme};
    use qcoin_types::BlockHeader;

    #[derive(Default)]
    struct FakeQcoinState {
        accepted: Vec<QCoinTransaction>,
        included: Vec<QCoinTransaction>,
    }

    struct FakeQcoinNode {
        addr: SocketAddr,
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<FakeQcoinState>>,
        udp_thread: Option<thread::JoinHandle<()>>,
        tcp_thread: Option<thread::JoinHandle<()>>,
    }

    impl FakeQcoinNode {
        fn start() -> Self {
            let udp = UdpSocket::bind("127.0.0.1:0").expect("bind fake qcoin udp");
            udp.set_read_timeout(Some(Duration::from_millis(100)))
                .expect("set fake qcoin udp timeout");
            let addr = udp.local_addr().expect("fake qcoin udp addr");

            let tcp = TcpListener::bind(addr).expect("bind fake qcoin tcp");
            tcp.set_nonblocking(true)
                .expect("set fake qcoin tcp nonblocking");

            let stop = Arc::new(AtomicBool::new(false));
            let state = Arc::new(Mutex::new(FakeQcoinState::default()));

            let udp_stop = Arc::clone(&stop);
            let udp_state = Arc::clone(&state);
            let udp_thread = thread::spawn(move || {
                let mut buf = [0u8; 64 * 1024];
                while !udp_stop.load(Ordering::SeqCst) {
                    match udp.recv_from(&mut buf) {
                        Ok((len, source)) => {
                            let response = match decode_wire_message(&buf[..len]) {
                                Ok(WireMessage::SubmitTransaction { transaction }) => {
                                    let mut state =
                                        udp_state.lock().expect("fake qcoin udp state lock");
                                    let tx_id = transaction.tx_id();
                                    if state.included.iter().any(|tx| tx.tx_id() == tx_id) {
                                        SubmitTransactionResponse {
                                            accepted: false,
                                            tx_id_hex: hex::encode(tx_id),
                                            message: "already committed".to_string(),
                                        }
                                    } else if state.accepted.iter().any(|tx| tx.tx_id() == tx_id) {
                                        SubmitTransactionResponse {
                                            accepted: false,
                                            tx_id_hex: hex::encode(tx_id),
                                            message: "already pending".to_string(),
                                        }
                                    } else {
                                        state.accepted.push(transaction);
                                        SubmitTransactionResponse {
                                            accepted: true,
                                            tx_id_hex: hex::encode(tx_id),
                                            message: "transaction accepted into mempool"
                                                .to_string(),
                                        }
                                    }
                                }
                                Ok(_) => continue,
                                Err(_) => continue,
                            };

                            let frame = encode_wire_message(
                                &WireMessage::SubmitTransactionResponse(response),
                            )
                            .expect("encode fake qcoin submit response");
                            let _ = udp.send_to(&frame, source);
                        }
                        Err(err)
                            if matches!(
                                err.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                }
            });

            let tcp_stop = Arc::clone(&stop);
            let tcp_state = Arc::clone(&state);
            let tcp_thread = thread::spawn(move || {
                let scheme = Dilithium2Scheme;
                let (pk, sk) = scheme.keygen().expect("fake qcoin keypair");
                let sig = scheme
                    .sign(&sk, b"fake-qcoin-block")
                    .expect("fake qcoin signature");
                while !tcp_stop.load(Ordering::SeqCst) {
                    match tcp.accept() {
                        Ok((mut stream, _)) => {
                            let _ = handle_fake_qcoin_http(&mut stream, &tcp_state, &pk, &sig);
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(50));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                addr,
                stop,
                state,
                udp_thread: Some(udp_thread),
                tcp_thread: Some(tcp_thread),
            }
        }

        fn include_next_pending(&self) {
            let mut state = self.state.lock().expect("fake qcoin state lock");
            if !state.accepted.is_empty() {
                let tx = state.accepted.remove(0);
                state.included.push(tx);
            }
        }
    }

    impl Drop for FakeQcoinNode {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ =
                TcpStream::connect(self.addr).and_then(|stream| stream.shutdown(Shutdown::Both));
            let _ = UdpSocket::bind("127.0.0.1:0")
                .and_then(|socket| socket.send_to(b"stop", self.addr).map(|_| ()));
            if let Some(thread) = self.udp_thread.take() {
                let _ = thread.join();
            }
            if let Some(thread) = self.tcp_thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn handle_fake_qcoin_http(
        stream: &mut TcpStream,
        state: &Arc<Mutex<FakeQcoinState>>,
        proposer_public_key: &qcoin_crypto::PublicKey,
        signature: &qcoin_crypto::Signature,
    ) -> io::Result<()> {
        let mut buf = [0u8; 4096];
        let len = stream.read(&mut buf)?;
        let request = String::from_utf8_lossy(&buf[..len]);
        let request_line = request.lines().next().unwrap_or_default();
        let path = request_line.split_whitespace().nth(1).unwrap_or("/");

        if path == "/tip" {
            let state = state.lock().expect("fake qcoin http state lock");
            let tip = TipResponse {
                height: state.included.len() as u64,
                tip_hash_hex: hex::encode([0u8; 32]),
                state_root_hex: hex::encode([0u8; 32]),
                last_timestamp: 0,
            };
            let payload = serde_json::to_vec(&tip).expect("serialize fake qcoin tip");
            write_http_response(stream, "200 OK", "application/json", &payload)
        } else if let Some(height) = path.strip_prefix("/blocks/") {
            let height: usize = height.parse().map_err(|err: std::num::ParseIntError| {
                io::Error::new(io::ErrorKind::InvalidInput, err.to_string())
            })?;
            let state = state.lock().expect("fake qcoin http state lock");
            if height == 0 || height > state.included.len() {
                write_http_response(stream, "404 Not Found", "text/plain", b"missing block")
            } else {
                let tx = state.included[height - 1].clone();
                let block = QCoinBlock {
                    header: BlockHeader {
                        parent_hash: [0u8; 32],
                        state_root: [0u8; 32],
                        tx_root: [0u8; 32],
                        height: height as u64,
                        timestamp: 0,
                    },
                    transactions: vec![tx],
                    proposer_public_key: proposer_public_key.clone(),
                    signature: signature.clone(),
                };
                let payload = bincode::serialize(&block)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
                write_http_response(stream, "200 OK", "application/octet-stream", &payload)
            }
        } else {
            write_http_response(stream, "404 Not Found", "text/plain", b"unknown route")
        }
    }

    fn write_http_response(
        stream: &mut TcpStream,
        status: &str,
        content_type: &str,
        body: &[u8],
    ) -> io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        stream.flush()
    }

    #[test]
    fn qcoin_storage_appends_and_enqueues_anchor() {
        let root = std::env::temp_dir().join(format!("test_qcoin_storage_{}", Uuid::new_v4()));
        let topics = root.join("player_logs");
        let outbox = root.join("qcoin_anchor_outbox.json");
        let storage = QCoinLedgerStorage::new(&topics, &outbox);

        let player = Uuid::new_v4();
        let block = Block {
            block_hash: "h".into(),
            previous_block_hash: "p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };

        storage
            .append_block(player, &block)
            .expect("append+enqueue");

        let loaded = storage.load_blocks(player).expect("load");
        assert_eq!(loaded.len(), 1);

        let state = load_outbox_state_unlocked(&outbox).expect("load outbox");
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.pending[0].player_id, player);
        assert_eq!(state.pending[0].progress, AnchorProgress::PendingSubmission);

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn qcoin_status_provider_reports_pending_outbox_entries() {
        let root = std::env::temp_dir().join(format!("test_qcoin_status_{}", Uuid::new_v4()));
        let topics = root.join("player_logs");
        let outbox = root.join("qcoin_anchor_outbox.json");
        let storage = QCoinLedgerStorage::new(&topics, &outbox);

        let player = Uuid::new_v4();
        let block = Block {
            block_hash: "h".into(),
            previous_block_hash: "p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };

        storage
            .append_block(player, &block)
            .expect("append+enqueue");

        let status = storage.status_provider().snapshot();
        assert_eq!(status.ledger_backend, "qcoin");
        assert_eq!(status.anchor_outbox_pending, 1);
        assert_eq!(status.anchor_outbox_pending_submission, 1);
        assert_eq!(status.anchor_outbox_accepted_not_included, 0);
        assert_eq!(status.last_anchor_accepted_unix_seconds, None);
        assert_eq!(status.last_anchor_included_unix_seconds, None);

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn derives_udp_target_from_http_url() {
        let target = derive_target_from_url("http://127.0.0.1:9700").expect("derive target");
        assert_eq!(target, "127.0.0.1:9700".parse().unwrap());
    }

    #[test]
    fn process_outbox_tracks_accepted_then_included_lifecycle() {
        let fake = FakeQcoinNode::start();
        let root = std::env::temp_dir().join(format!("test_qcoin_lifecycle_{}", Uuid::new_v4()));
        let topics = root.join("player_logs");
        let outbox = root.join("qcoin_anchor_outbox.json");
        let storage = QCoinLedgerStorage::new_with_target(&topics, &outbox, Some(fake.addr));

        let player = Uuid::new_v4();
        let block = Block {
            block_hash: "lifecycle-h".into(),
            previous_block_hash: "lifecycle-p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };

        storage
            .append_block(player, &block)
            .expect("append block into lifecycle storage");

        process_outbox(&storage.outbox).expect("first outbox pass");
        let state = load_outbox_state(&storage.outbox).expect("load accepted outbox state");
        let status = storage.status_provider().snapshot();
        assert_eq!(state.pending.len(), 1);
        assert_eq!(
            state.pending[0].progress,
            AnchorProgress::AcceptedNotIncluded
        );
        assert_eq!(status.anchor_outbox_pending, 1);
        assert_eq!(status.anchor_outbox_pending_submission, 0);
        assert_eq!(status.anchor_outbox_accepted_not_included, 1);
        assert!(status.last_anchor_accepted_unix_seconds.is_some());
        assert_eq!(status.last_anchor_included_unix_seconds, None);

        fake.include_next_pending();

        process_outbox(&storage.outbox).expect("second outbox pass");
        let state = load_outbox_state(&storage.outbox).expect("load included outbox state");
        let status = storage.status_provider().snapshot();
        assert!(state.pending.is_empty());
        assert_eq!(status.anchor_outbox_pending, 0);
        assert_eq!(status.anchor_outbox_pending_submission, 0);
        assert_eq!(status.anchor_outbox_accepted_not_included, 0);
        assert!(status.last_anchor_included_unix_seconds.is_some());
        assert!(status.last_anchor_success_unix_seconds.is_some());

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn one_included_anchor_does_not_clear_later_anchor() {
        let fake = FakeQcoinNode::start();
        let root = std::env::temp_dir().join(format!("test_qcoin_multi_{}", Uuid::new_v4()));
        let topics = root.join("player_logs");
        let outbox = root.join("qcoin_anchor_outbox.json");
        let storage = QCoinLedgerStorage::new_with_target(&topics, &outbox, Some(fake.addr));

        let player = Uuid::new_v4();
        let first = Block {
            block_hash: "multi-h-1".into(),
            previous_block_hash: "multi-p-1".into(),
            timestamp: "t1".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };
        let second = Block {
            block_hash: "multi-h-2".into(),
            previous_block_hash: "multi-p-2".into(),
            timestamp: "t2".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };

        storage
            .append_block(player, &first)
            .expect("append first block");
        storage
            .append_block(player, &second)
            .expect("append second block");

        process_outbox(&storage.outbox).expect("accept both anchors");
        let state = load_outbox_state(&storage.outbox).expect("load accepted anchors");
        assert_eq!(state.pending.len(), 2);
        assert!(state
            .pending
            .iter()
            .all(|entry| entry.progress == AnchorProgress::AcceptedNotIncluded));

        fake.include_next_pending();

        process_outbox(&storage.outbox).expect("include first anchor only");
        let state = load_outbox_state(&storage.outbox).expect("load partially included anchors");
        let status = storage.status_provider().snapshot();
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.pending[0].block.block_hash, second.block_hash);
        assert_eq!(
            state.pending[0].progress,
            AnchorProgress::AcceptedNotIncluded
        );
        assert_eq!(status.anchor_outbox_pending, 1);
        assert_eq!(status.anchor_outbox_pending_submission, 0);
        assert_eq!(status.anchor_outbox_accepted_not_included, 1);

        fake.include_next_pending();

        process_outbox(&storage.outbox).expect("include second anchor");
        let state = load_outbox_state(&storage.outbox).expect("load fully included anchors");
        let status = storage.status_provider().snapshot();
        assert!(state.pending.is_empty());
        assert_eq!(status.anchor_outbox_pending, 0);
        assert_eq!(status.anchor_outbox_accepted_not_included, 0);
        assert!(status.last_anchor_included_unix_seconds.is_some());

        std::fs::remove_dir_all(root).ok();
    }
}
