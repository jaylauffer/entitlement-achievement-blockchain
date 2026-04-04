use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, io};

use qcoin_consensus::{ConsensusEngine, DummyConsensusEngine};
use qcoin_ledger::{ChainState, LedgerState};
use qcoin_script::DeterministicScriptEngine;
use qcoin_types::{
    Hash256, Output, Transaction as QCoinTransaction, TransactionCore, TransactionKind,
    TransactionWitness,
};
use uuid::Uuid;

use crate::blockchain::Block;
use crate::ledger_storage::{FileTopicLedgerStorage, LedgerStorage};
use crate::player_profile::profile_service::AchievementClaim;

struct QCoinRuntime {
    chain: ChainState,
}

/// Storage backend that keeps the existing per-player block logs while mirroring
/// each appended block into a local QCoin chain-state anchor.
pub struct QCoinLedgerStorage {
    topic_storage: FileTopicLedgerStorage,
    state_path: PathBuf,
    node_url: Option<String>,
    script_engine: DeterministicScriptEngine,
    runtime: Mutex<QCoinRuntime>,
}

impl QCoinLedgerStorage {
    /// `topic_base_path` keeps the per-player block logs used by profile rehydration.
    /// `qcoin_state_path` stores mirrored QCoin chain state in binary format.
    pub fn new<P1: Into<PathBuf>, P2: Into<PathBuf>>(
        topic_base_path: P1,
        qcoin_state_path: P2,
    ) -> Self {
        let state_path = qcoin_state_path.into();
        if let Some(parent) = state_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let chain = Self::load_chain_state(&state_path).unwrap_or_else(Self::initial_chain_state);

        Self {
            topic_storage: FileTopicLedgerStorage::new(topic_base_path),
            state_path,
            node_url: env::var("QCOIN_NODE_URL")
                .ok()
                .map(|url| url.trim_end_matches('/').to_string())
                .filter(|url| !url.is_empty()),
            script_engine: DeterministicScriptEngine::default(),
            runtime: Mutex::new(QCoinRuntime { chain }),
        }
    }

    fn io_other(message: impl Into<String>) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, message.into())
    }

    fn initial_chain_state() -> ChainState {
        let ledger = LedgerState {
            utxos: Default::default(),
            assets: Default::default(),
        };
        let state_root = ledger.state_root();

        ChainState {
            ledger,
            height: 0,
            tip_hash: [0u8; 32],
            state_root,
            last_timestamp: 0,
            chain_id: 0,
        }
    }

    fn load_chain_state(path: &Path) -> Option<ChainState> {
        let mut file = File::open(path).ok()?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).ok()?;
        bincode::deserialize::<ChainState>(&contents).ok()
    }

    fn save_chain_state(path: &Path, chain: &ChainState) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let state = bincode::serialize(chain)
            .map_err(|err| Self::io_other(format!("failed to serialize qcoin state: {err}")))?;
        let mut file = File::create(path)?;
        file.write_all(&state)?;
        Ok(())
    }

    fn player_owner_hash(player_id: Uuid) -> Hash256 {
        *blake3::hash(player_id.as_bytes()).as_bytes()
    }

    fn block_metadata_hash(block: &Block) -> std::io::Result<Hash256> {
        let json = serde_json::to_vec(block)
            .map_err(|err| Self::io_other(format!("failed to serialize block payload: {err}")))?;
        Ok(*blake3::hash(&json).as_bytes())
    }

    fn make_anchor_tx(player_id: Uuid, block: &Block) -> std::io::Result<QCoinTransaction> {
        let metadata_hash = Self::block_metadata_hash(block)?;

        Ok(QCoinTransaction {
            core: TransactionCore {
                kind: TransactionKind::Transfer,
                inputs: vec![],
                outputs: vec![Output {
                    owner_script_hash: Self::player_owner_hash(player_id),
                    // Anchor-only output: no assets moved, only metadata hash anchored.
                    assets: vec![],
                    metadata_hash: Some(metadata_hash),
                }],
            },
            witness: TransactionWitness::default(),
        })
    }

    fn current_unix_timestamp() -> std::io::Result<u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| Self::io_other(format!("failed to read system time: {err}")))?;
        Ok(now.as_secs())
    }

    fn mirror_to_qcoin(&self, player_id: Uuid, block: &Block) -> std::io::Result<()> {
        let tx = Self::make_anchor_tx(player_id, block)?;

        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| Self::io_other("qcoin runtime lock poisoned"))?;

        while Self::current_unix_timestamp()? <= runtime.chain.last_timestamp {
            thread::sleep(Duration::from_millis(200));
        }

        // Create a fresh dummy consensus engine for each proposal to avoid
        // storing non-Send components inside the storage type.
        let consensus = DummyConsensusEngine::default();

        let qcoin_block = consensus
            .propose_block(&runtime.chain, vec![tx])
            .map_err(|err| Self::io_other(format!("failed to propose qcoin block: {err}")))?;

        consensus
            .validate_block(&runtime.chain, &qcoin_block)
            .map_err(|err| Self::io_other(format!("failed to validate qcoin block: {err}")))?;

        runtime
            .chain
            .apply_block(&qcoin_block, &self.script_engine)
            .map_err(|err| Self::io_other(format!("failed to apply qcoin block: {err}")))?;

        Self::save_chain_state(&self.state_path, &runtime.chain)?;

        if let Some(node_url) = &self.node_url {
            self.submit_block_to_node(node_url, &qcoin_block)?;
        }

        Ok(())
    }

    fn submit_block_to_node(&self, node_url: &str, block: &qcoin_types::Block) -> io::Result<()> {
        let endpoint = format!("{}/blocks", node_url.trim_end_matches('/'));
        let payload = bincode::serialize(block)
            .map_err(|err| Self::io_other(format!("failed to serialize qcoin block: {err}")))?;

        match ureq::post(&endpoint)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&payload)
        {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(Self::io_other(format!(
                    "qcoin node rejected block ({code}) at {endpoint}: {body}"
                )))
            }
            Err(err) => Err(Self::io_other(format!(
                "failed to submit block to qcoin node {endpoint}: {err}"
            ))),
        }
    }
}

impl LedgerStorage for QCoinLedgerStorage {
    fn append_block(&self, player_id: Uuid, block: &Block) -> std::io::Result<()> {
        // Keep canonical per-player logs for service recovery semantics.
        self.topic_storage.append_block(player_id, block)?;
        // Mirror each block into QCoin for cross-chain anchoring.
        self.mirror_to_qcoin(player_id, block)
    }

    fn load_blocks(&self, player_id: Uuid) -> std::io::Result<Vec<Block>> {
        self.topic_storage.load_blocks(player_id)
    }

    fn list_player_ids(&self) -> std::io::Result<Vec<Uuid>> {
        self.topic_storage.list_player_ids()
    }

    fn load_achievement_claims(&self, player_id: Uuid) -> std::io::Result<Vec<AchievementClaim>> {
        self.topic_storage.load_achievement_claims(player_id)
    }

    fn save_achievement_claims(
        &self,
        player_id: Uuid,
        claims: &[AchievementClaim],
    ) -> std::io::Result<()> {
        self.topic_storage
            .save_achievement_claims(player_id, claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qcoin_storage_appends_and_mirrors() {
        let root = std::env::temp_dir().join(format!("test_qcoin_storage_{}", Uuid::new_v4()));
        let topics = root.join("player_logs");
        let state = root.join("qcoin_chain_state.json");
        let storage = QCoinLedgerStorage::new(&topics, &state);

        let player = Uuid::new_v4();
        let block = Block {
            block_hash: "h".into(),
            previous_block_hash: "p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };

        storage.append_block(player, &block).expect("append+mirror");

        let loaded = storage.load_blocks(player).expect("load");
        assert_eq!(loaded.len(), 1);
        assert!(state.exists(), "qcoin state should be written");

        std::fs::remove_dir_all(root).ok();
    }
}
