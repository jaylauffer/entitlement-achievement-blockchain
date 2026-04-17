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
use crate::ledger_storage::{FileTopicLedgerStorage, LedgerStorage};
use crate::player_profile::profile_service::AchievementClaim;

const QCOIN_WIRE_MAGIC: [u8; 4] = *b"QCN1";
const DEFAULT_QCOIN_NODE_PORT: u16 = 9700;
const OUTBOX_RETRY_DELAY: Duration = Duration::from_secs(5);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnchorOutboxEntry {
    player_id: Uuid,
    block: Block,
    transaction: QCoinTransaction,
    attempts: u32,
    last_error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AnchorOutboxState {
    pending: Vec<AnchorOutboxEntry>,
}

struct AnchorOutboxShared {
    path: PathBuf,
    file_lock: Mutex<()>,
    node_target: SocketAddr,
}

/// Storage backend that keeps the canonical per-player logs while enqueueing
/// qcoin anchor transactions for asynchronous submission to a live qcoin node.
pub struct QCoinLedgerStorage {
    topic_storage: FileTopicLedgerStorage,
    outbox: Arc<AnchorOutboxShared>,
    worker: Option<ProactorHandle<ChannelPort>>,
}

impl QCoinLedgerStorage {
    /// `topic_base_path` keeps the per-player block logs used by profile rehydration.
    /// `qcoin_outbox_path` stores pending qcoin anchor submissions.
    pub fn new<P1: Into<PathBuf>, P2: Into<PathBuf>>(
        topic_base_path: P1,
        qcoin_outbox_path: P2,
    ) -> Self {
        let outbox_path = qcoin_outbox_path.into();
        if let Some(parent) = outbox_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let node_target = resolve_qcoin_node_target();
        let outbox = Arc::new(AnchorOutboxShared {
            path: outbox_path,
            file_lock: Mutex::new(()),
            node_target: node_target.unwrap_or_else(|| {
                "127.0.0.1:9700"
                    .parse()
                    .expect("default qcoin node target should parse")
            }),
        });

        let worker = node_target.map(|_| spawn_anchor_worker(Arc::clone(&outbox)));

        Self {
            topic_storage: FileTopicLedgerStorage::new(topic_base_path),
            outbox,
            worker,
        }
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
            attempts: 0,
            last_error: None,
        });
        save_outbox_state(&self.outbox, &state)?;
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
        return Ok(false);
    }

    let mut remaining = Vec::new();
    for mut entry in state.pending.drain(..) {
        match submit_transaction_to_qcoin(outbox.node_target, &entry.transaction) {
            Ok(response) if response.accepted => {}
            Ok(response) => {
                entry.attempts += 1;
                entry.last_error = Some(response.message);
                remaining.push(entry);
            }
            Err(err) => {
                entry.attempts += 1;
                entry.last_error = Some(err.to_string());
                remaining.push(entry);
            }
        }
    }

    state.pending = remaining;
    save_outbox_state(outbox, &state)?;
    Ok(!state.pending.is_empty())
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

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn derives_udp_target_from_http_url() {
        let target = derive_target_from_url("http://127.0.0.1:9700").expect("derive target");
        assert_eq!(target, "127.0.0.1:9700".parse().unwrap());
    }
}
