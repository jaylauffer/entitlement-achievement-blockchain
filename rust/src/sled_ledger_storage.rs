use std::path::PathBuf;
use uuid::Uuid;

use crate::blockchain::Block;
use crate::ledger_storage::LedgerStorage;

/// Ledger storage backed by a sled key-value database.
/// Blocks for each player are stored in a separate tree
/// named after the player UUID.
pub struct SledLedgerStorage {
    db: sled::Db,
}

impl SledLedgerStorage {
    /// Open or create a sled database at the given path.
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        let db = sled::open(path.into()).expect("failed to open sled database");
        Self { db }
    }
}

impl LedgerStorage for SledLedgerStorage {
    fn append_block(&self, player_id: Uuid, block: &Block) -> std::io::Result<()> {
        let tree = self.db.open_tree(player_id.to_string()).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        let id = self.db.generate_id().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        let json = serde_json::to_vec(block)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        tree.insert(id.to_be_bytes(), json).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        tree.flush().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(())
    }

    fn load_blocks(&self, player_id: Uuid) -> std::io::Result<Vec<Block>> {
        let tree = self.db.open_tree(player_id.to_string()).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        let mut blocks = Vec::new();
        for item in tree.iter() {
            let (_key, val) = item.map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e)
            })?;
            let block: Block = serde_json::from_slice(&val).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e)
            })?;
            blocks.push(block);
        }
        blocks.sort_by_key(|b| b.timestamp.clone());
        Ok(blocks)
    }

    fn list_player_ids(&self) -> std::io::Result<Vec<Uuid>> {
        let mut ids = Vec::new();
        for name in self.db.tree_names() {
            if let Ok(s) = std::str::from_utf8(&name) {
                if let Ok(id) = Uuid::parse_str(s) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sled_storage_round_trip() {
        let dir = "test_sled_db";
        let storage = SledLedgerStorage::new(dir);
        let player = Uuid::new_v4();
        let block = Block {
            block_hash: "h".into(),
            previous_block_hash: "p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };
        storage.append_block(player, &block).expect("append block");
        let loaded = storage.load_blocks(player).expect("load blocks");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].block_hash, "h");
        std::fs::remove_dir_all(dir).ok();
    }
}
