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

const META_TREE: &str = "ledger_meta";

impl SledLedgerStorage {
    /// Open or create a sled database at the given path.
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        let db = sled::open(path.into()).expect("failed to open sled database");
        Self { db }
    }

    fn update_head(&self, player_id: Uuid, block: &Block) -> std::io::Result<()> {
        let meta_tree = self.db.open_tree(META_TREE).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        let height_key = format!("{player_id}:head_height");
        let hash_key = format!("{player_id}:head_hash");
        let current_height = meta_tree
            .get(height_key.as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
            .map(|bytes| {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&bytes);
                u64::from_be_bytes(buf)
            })
            .unwrap_or(0);
        let next_height = current_height + 1;
        meta_tree
            .insert(height_key.as_bytes(), next_height.to_be_bytes().to_vec())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        meta_tree
            .insert(hash_key.as_bytes(), block.block_hash.as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        meta_tree
            .flush()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(())
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
        self.update_head(player_id, block)?;
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
        let dir = std::env::temp_dir()
            .join(format!("test_sled_db_{}", Uuid::new_v4()));
        let storage = SledLedgerStorage::new(&dir);
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

    #[test]
    fn test_sled_storage_ordering_with_same_timestamp() {
        let dir = std::env::temp_dir()
            .join(format!("test_sled_db_order_{}", Uuid::new_v4()));
        let storage = SledLedgerStorage::new(&dir);
        let player = Uuid::new_v4();
        let block_a = Block {
            block_hash: "hash-a".into(),
            previous_block_hash: "p".into(),
            timestamp: "same-time".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };
        let block_b = Block {
            block_hash: "hash-b".into(),
            previous_block_hash: "hash-a".into(),
            timestamp: "same-time".into(),
            app_version: "v".into(),
            nonce: 1,
            transactions: vec![],
        };
        storage.append_block(player, &block_a).expect("append block a");
        storage.append_block(player, &block_b).expect("append block b");
        let loaded = storage.load_blocks(player).expect("load blocks");
        let hashes: Vec<_> = loaded.iter().map(|b| b.block_hash.as_str()).collect();
        assert_eq!(hashes, vec!["hash-a", "hash-b"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
