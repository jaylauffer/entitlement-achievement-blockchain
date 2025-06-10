use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::blockchain::Block;

/// Trait describing persistence for blockchain blocks.
pub trait LedgerStorage {
    /// Append a block to the log for the given player id.
    fn append_block(&self, player_id: Uuid, block: &Block) -> std::io::Result<()>;
    /// Load all blocks for the given player id.
    fn load_blocks(&self, player_id: Uuid) -> std::io::Result<Vec<Block>>;
}

/// A simple file-based log storage. Each player's log is written to
/// `<base_path>/<player_id>.log` where the player id is a UUID string.
pub struct FileTopicLedgerStorage {
    base_path: PathBuf,
}

impl FileTopicLedgerStorage {
    /// Create a new storage rooted at the provided path.
    pub fn new<P: Into<PathBuf>>(base_path: P) -> Self {
        let base = base_path.into();
        std::fs::create_dir_all(&base).ok();
        Self { base_path: base }
    }

    fn topic_path(&self, player_id: &Uuid) -> PathBuf {
        self.base_path.join(format!("{}.log", player_id))
    }
}

impl LedgerStorage for FileTopicLedgerStorage {
    fn append_block(&self, player_id: Uuid, block: &Block) -> std::io::Result<()> {
        let path = self.topic_path(&player_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let json = serde_json::to_string(block).unwrap();
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        Ok(())
    }

    fn load_blocks(&self, player_id: Uuid) -> std::io::Result<Vec<Block>> {
        let path = self.topic_path(&player_id);
        if !Path::new(&path).exists() {
            return Ok(Vec::new());
        }
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut blocks = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let block: Block = serde_json::from_str(&line).unwrap_or_else(|_| Block {
                block_hash: String::new(),
                previous_block_hash: String::new(),
                timestamp: String::new(),
                app_version: String::new(),
                nonce: 0,
                transactions: vec![],
            });
            blocks.push(block);
        }
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain::Block;

    #[test]
    fn test_file_topic_storage_round_trip() {
        let dir = "test_logs";
        let storage = FileTopicLedgerStorage::new(dir);
        let player = Uuid::new_v4();
        let block = Block {
            block_hash: "h".into(),
            previous_block_hash: "p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };
        storage.append_block(player, &block).unwrap();
        let loaded = storage.load_blocks(player).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].block_hash, "h");
        std::fs::remove_file(storage.topic_path(&player)).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

