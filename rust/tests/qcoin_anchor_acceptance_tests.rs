use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use loadngo_eab::blockchain::Block;
use loadngo_eab::ledger_storage::LedgerStorage;
use loadngo_eab::qcoin_ledger_storage::QCoinLedgerStorage;
use qcoin_types::Block as QCoinBlock;
use serde::Deserialize;
use uuid::Uuid;

fn sample_block(label: &str) -> Block {
    Block {
        block_hash: format!("block-{label}"),
        previous_block_hash: format!("prev-{label}"),
        timestamp: "2026-04-17T00:00:00Z".to_string(),
        app_version: "test".to_string(),
        nonce: 0,
        transactions: vec![],
    }
}

fn unique_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("eab_qcoin_anchor_{label}_{}", Uuid::new_v4()))
}

fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    condition()
}

#[derive(Debug, Deserialize)]
struct TipResponse {
    height: u64,
}

fn qcoin_http_base(target: SocketAddr) -> String {
    match target {
        SocketAddr::V4(addr) => format!("http://{}:{}", addr.ip(), addr.port()),
        SocketAddr::V6(addr) => format!("http://[{}]:{}", addr.ip(), addr.port()),
    }
}

fn qcoin_transaction_is_included(target: SocketAddr, tx_id: [u8; 32]) -> bool {
    let url = format!("{}/tip", qcoin_http_base(target));
    let tip = match ureq::get(&url).call() {
        Ok(response) => match response.into_json::<TipResponse>() {
            Ok(tip) => tip,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    for height in (1..=tip.height).rev() {
        let url = format!("{}/blocks/{height}", qcoin_http_base(target));
        let response = match ureq::get(&url).call() {
            Ok(response) => response,
            Err(_) => continue,
        };
        let mut reader = response.into_reader();
        let mut bytes = Vec::new();
        if reader.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let Ok(block) = bincode::deserialize::<QCoinBlock>(&bytes) else {
            continue;
        };
        if block.transactions.iter().any(|tx| tx.tx_id() == tx_id) {
            return true;
        }
    }

    false
}

#[test]
fn qcoin_anchor_outbox_survives_storage_restart() {
    let root = unique_root("restart");
    let topics = root.join("player_logs");
    let outbox = root.join("qcoin_anchor_outbox.json");
    let player = Uuid::new_v4();
    let block = sample_block("restart");

    {
        let storage = QCoinLedgerStorage::new_with_target(&topics, &outbox, None);
        storage
            .append_block(player, &block)
            .expect("append block into qcoin-backed storage");

        let status = storage.status_provider().snapshot();
        assert_eq!(status.ledger_backend, "qcoin");
        assert_eq!(status.qcoin_node_target, None);
        assert_eq!(status.anchor_outbox_pending, 1);
        assert_eq!(status.anchor_outbox_pending_submission, 1);
        assert_eq!(status.anchor_outbox_accepted_not_included, 0);
        assert_eq!(status.last_anchor_accepted_unix_seconds, None);
        assert_eq!(status.last_anchor_included_unix_seconds, None);
        assert_eq!(status.last_anchor_success_unix_seconds, None);
    }

    let storage = QCoinLedgerStorage::new_with_target(&topics, &outbox, None);
    let loaded = storage.load_blocks(player).expect("reload player blocks");
    let status = storage.status_provider().snapshot();

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].block_hash, block.block_hash);
    assert_eq!(status.anchor_outbox_pending, 1);
    assert_eq!(status.anchor_outbox_pending_submission, 1);
    assert_eq!(status.anchor_outbox_accepted_not_included, 0);
    assert_eq!(status.last_anchor_accepted_unix_seconds, None);
    assert_eq!(status.last_anchor_included_unix_seconds, None);
    assert_eq!(status.last_anchor_success_unix_seconds, None);

    std::fs::remove_dir_all(root).ok();
}

#[test]
#[ignore = "requires a live qcoin node target via EAB_QCOIN_TEST_TARGET"]
fn qcoin_anchor_outbox_drains_against_live_qcoin_node() {
    let target = std::env::var("EAB_QCOIN_TEST_TARGET")
        .expect("set EAB_QCOIN_TEST_TARGET=host:port to run this test");
    let target: SocketAddr = target
        .to_socket_addrs()
        .expect("resolve qcoin test target")
        .next()
        .expect("at least one qcoin test target address");

    let root = unique_root("live");
    let topics = root.join("player_logs");
    let outbox = root.join("qcoin_anchor_outbox.json");
    let player = Uuid::new_v4();
    let block = sample_block("live");
    let tx_id = QCoinLedgerStorage::anchor_transaction_id(player, &block)
        .expect("derive qcoin anchor transaction id");
    let storage = QCoinLedgerStorage::new_with_target(&topics, &outbox, Some(target));

    storage
        .append_block(player, &block)
        .expect("append block into qcoin-backed storage");

    let included = wait_for(Duration::from_secs(20), || {
        let status = storage.status_provider().snapshot();
        status.anchor_outbox_pending == 0
            && status.anchor_outbox_pending_submission == 0
            && status.anchor_outbox_accepted_not_included == 0
            && status.last_anchor_included_unix_seconds.is_some()
            && qcoin_transaction_is_included(target, tx_id)
    });

    let status = storage.status_provider().snapshot();
    let expected_target = target.to_string();
    assert!(
        included,
        "expected qcoin anchor to reach inclusion, got status: pending={}, pending_submission={}, accepted_not_included={}, last_accepted={:?}, last_included={:?}, last_success={:?}, last_error={:?}",
        status.anchor_outbox_pending,
        status.anchor_outbox_pending_submission,
        status.anchor_outbox_accepted_not_included,
        status.last_anchor_accepted_unix_seconds,
        status.last_anchor_included_unix_seconds,
        status.last_anchor_success_unix_seconds,
        status.last_anchor_error
    );
    assert_eq!(
        status.qcoin_node_target.as_deref(),
        Some(expected_target.as_str())
    );
    assert_eq!(status.anchor_outbox_pending, 0);
    assert_eq!(status.anchor_outbox_pending_submission, 0);
    assert_eq!(status.anchor_outbox_accepted_not_included, 0);
    assert!(status.last_anchor_accepted_unix_seconds.is_some());
    assert!(status.last_anchor_included_unix_seconds.is_some());
    assert!(status.last_anchor_success_unix_seconds.is_some());
    assert_eq!(status.last_anchor_error, None);
    assert!(qcoin_transaction_is_included(target, tx_id));

    std::fs::remove_dir_all(root).ok();
}
