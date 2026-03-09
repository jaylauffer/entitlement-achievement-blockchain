use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use uuid::Uuid;

use crate::blockchain::Block;

/// Trait describing persistence for blockchain blocks.
pub trait LedgerStorage {
    /// Append a block to the log for the given player id.
    fn append_block(&self, player_id: Uuid, block: &Block) -> std::io::Result<()>;
    /// Load all blocks for the given player id.
    fn load_blocks(&self, player_id: Uuid) -> std::io::Result<Vec<Block>>;
    /// List all player ids that currently have a log file.
    fn list_player_ids(&self) -> std::io::Result<Vec<Uuid>>;
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
        // On Windows, exclusive file locks can fail with ERROR_ACCESS_DENIED if the
        // handle is opened append-only. Open the log with read/write access as well
        // so the same storage code works across desktop targets.
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .append(true)
            .open(path)?;
        file.lock_exclusive()?;
        let json = serde_json::to_string(block)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let mut writer = BufWriter::new(&file);
        writer.write_all(json.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        file.sync_data()?;
        FileExt::unlock(&file)?;
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
        for (index, line) in reader.lines().enumerate() {
            let line = line?;
            let block: Block = serde_json::from_str(&line).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("failed to parse block at line {}: {}", index + 1, err),
                )
            })?;
            blocks.push(block);
        }
        Ok(blocks)
    }

    fn list_player_ids(&self) -> std::io::Result<Vec<Uuid>> {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&self.base_path)? {
            let entry = entry?;
            if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                if let Ok(id) = Uuid::parse_str(stem) {
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
    use crate::blockchain::Block;
    use std::collections::HashSet;
    use std::io::Read;
    use std::thread;

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
        storage.append_block(player, &block).expect("append block");
        let loaded = storage.load_blocks(player).expect("load blocks");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].block_hash, "h");
        let _ = std::fs::remove_file(storage.topic_path(&player));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_load_blocks_rejects_corrupt_lines() {
        let dir = "test_logs_corrupt";
        let storage = FileTopicLedgerStorage::new(dir);
        let player = Uuid::new_v4();
        let path = storage.topic_path(&player);
        let mut file = File::create(&path).expect("create log");
        let block = Block {
            block_hash: "h".into(),
            previous_block_hash: "p".into(),
            timestamp: "t".into(),
            app_version: "v".into(),
            nonce: 0,
            transactions: vec![],
        };
        let json = serde_json::to_string(&block).expect("serialize block");
        writeln!(file, "{}", json).expect("write valid block");
        writeln!(file, "{{not-json").expect("write corrupt block");

        let result = storage.load_blocks(player);
        assert!(result.is_err(), "expected corrupt log to error");

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_concurrent_appends_do_not_interleave() {
        let dir = format!("test_logs_concurrent_{}", Uuid::new_v4());
        let storage = FileTopicLedgerStorage::new(&dir);
        let player = Uuid::new_v4();
        let mut handles = Vec::new();
        for index in 0..16 {
            let dir = dir.clone();
            let player = player;
            handles.push(thread::spawn(move || {
                let storage = FileTopicLedgerStorage::new(&dir);
                let block = Block {
                    block_hash: format!("h{}", index),
                    previous_block_hash: "p".into(),
                    timestamp: "t".into(),
                    app_version: "v".into(),
                    nonce: index,
                    transactions: vec![],
                };
                storage.append_block(player, &block).expect("append block");
            }));
        }
        for handle in handles {
            handle.join().expect("thread join");
        }
        let loaded = storage.load_blocks(player).expect("load blocks");
        assert_eq!(loaded.len(), 16);
        let hashes: HashSet<String> = loaded.into_iter().map(|block| block.block_hash).collect();
        assert_eq!(hashes.len(), 16);
        let _ = std::fs::remove_file(storage.topic_path(&player));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_append_block_writes_complete_lines() {
        let dir = format!("test_logs_consistency_{}", Uuid::new_v4());
        let storage = FileTopicLedgerStorage::new(&dir);
        let player = Uuid::new_v4();
        for index in 0..4 {
            let block = Block {
                block_hash: format!("h{}", index),
                previous_block_hash: "p".into(),
                timestamp: "t".into(),
                app_version: "v".into(),
                nonce: index,
                transactions: vec![],
            };
            storage.append_block(player, &block).expect("append block");
        }
        let mut file = File::open(storage.topic_path(&player)).expect("open log");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("read log");
        assert!(contents.ends_with('\n'), "expected newline terminated log");
        let parsed: Vec<Block> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse block"))
            .collect();
        assert_eq!(parsed.len(), 4);
        let _ = std::fs::remove_file(storage.topic_path(&player));
        let _ = std::fs::remove_dir_all(dir);
    }
}
