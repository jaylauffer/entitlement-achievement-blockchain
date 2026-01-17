pub mod profile_service {
    use std::collections::HashMap;
    use serde::{Serialize, Deserialize};
    use sha2::{Sha256, Digest};
    use chrono::prelude::*;
    use uuid::Uuid;
    use crate::hd::BitVec;
    use crate::blockchain::{Block, Blockchain, Transaction, TransactionData, ProfileChange};
    use crate::ledger_storage::LedgerStorage;

    pub const DEFAULT_DIM: usize = 16384;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PlayerProfile {
        pub player_id: String,
        pub name: String,
        pub profile_vec: BitVec,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct PlayerRewardState {
        pub entitlements: Vec<crate::blockchain::Entitlement>,
        pub achievements: Vec<crate::blockchain::Achievement>,
    }

    impl PlayerProfile {
        pub fn new(player_id: String, name: String) -> Self {
            PlayerProfile {
                player_id,
                name,
                profile_vec: BitVec::new(DEFAULT_DIM),
            }
        }


        pub fn set_vector(&mut self, vec: BitVec) {
            self.profile_vec = vec;
        }
    }

    pub struct PlayerProfileService {
        profiles: HashMap<String, PlayerProfile>,
        rewards: HashMap<String, PlayerRewardState>,
        pub ledger: Blockchain,
        storage: Box<dyn LedgerStorage + Send + Sync>,
    }

    impl PlayerProfileService {
        pub fn new(storage: Box<dyn LedgerStorage + Send + Sync>) -> Self {
            let mut service = PlayerProfileService {
                profiles: HashMap::new(),
                rewards: HashMap::new(),
                ledger: Blockchain::new(),
                storage,
            };
            if let Ok(ids) = service.storage.list_player_ids() {
                let mut verified_blocks = Vec::new();
                let mut _quarantined_blocks: Vec<Block> = Vec::new();
                for id in ids {
                    if let Ok(b) = service.storage.load_blocks(id) {
                        let (valid, quarantined) = Self::verify_player_blocks(b);
                        for block in &valid {
                            for txn in &block.transactions {
                                match &txn.details {
                                    TransactionData::ProfileChange(change) => {
                                        service.profiles.insert(change.profile.player_id.clone(), change.profile.clone());
                                    }
                                    TransactionData::Entitlement(entitlement) => {
                                        service
                                            .rewards
                                            .entry(txn.player_id.clone())
                                            .or_default()
                                            .entitlements
                                            .push(entitlement.clone());
                                    }
                                    TransactionData::Achievement(achievement) => {
                                        service
                                            .rewards
                                            .entry(txn.player_id.clone())
                                            .or_default()
                                            .achievements
                                            .push(achievement.clone());
                                    }
                                }
                            }
                        }
                        verified_blocks.extend(valid);
                        _quarantined_blocks.extend(quarantined);
                    }
                }
                verified_blocks.sort_by_key(|b| b.timestamp.clone());
                let mut seen = std::collections::HashSet::new();
                for block in verified_blocks {
                    if seen.insert(block.block_hash.clone()) {
                        service.ledger.chain.push(block);
                    }
                }
            }
            service
        }

        fn verify_player_blocks(blocks: Vec<Block>) -> (Vec<Block>, Vec<Block>) {
            let mut verified = Vec::new();
            let mut quarantined = Vec::new();
            let mut chain = Blockchain::new();
            for block in blocks {
                chain.chain.push(block.clone());
                if chain.is_valid_chain() {
                    verified.push(block);
                } else {
                    chain.chain.pop();
                    quarantined.push(block);
                }
            }
            (verified, quarantined)
        }

        pub fn create_profile(&mut self, player_id: &str, name: &str) -> std::io::Result<&PlayerProfile> {
            let profile = PlayerProfile::new(player_id.to_string(), name.to_string());
            self.profiles.insert(player_id.to_string(), profile.clone());
            self.rewards.entry(player_id.to_string()).or_default();
            self.log_change(&profile)?;
            self.profiles
                .get(player_id)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "profile insertion failed"))
        }

        pub fn get_profile(&self, player_id: &str) -> Option<&PlayerProfile> {
            self.profiles.get(player_id)
        }

        pub fn get_reward_state(&self, player_id: &str) -> Option<&PlayerRewardState> {
            self.rewards.get(player_id)
        }

        pub fn set_vector(&mut self, player_id: &str, vec: BitVec) -> std::io::Result<()> {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                profile.set_vector(vec);
                let cloned = profile.clone();
                self.log_change(&cloned)
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "profile not found"))
            }
        }

        pub fn merge_vector(&mut self, player_id: &str, vec: &BitVec) -> std::io::Result<()> {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                let new_vec = profile.profile_vec.xor(vec);
                profile.set_vector(new_vec);
                let cloned = profile.clone();
                self.log_change(&cloned)
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "profile not found"))
            }
        }

        pub fn award_entitlement(&mut self, player_id: &str, entitlement: &crate::entitlement_registry::EntitlementDefinition, quantity: u32, expiration_date: Option<String>) -> std::io::Result<()> {
            if self.profiles.get(player_id).is_some() {
                let details = crate::blockchain::Entitlement {
                    developer: entitlement.developer.clone(),
                    game: entitlement.game.clone(),
                    entitlement_id: entitlement.entitlement_id.clone(),
                    version: entitlement.version,
                    item_type: entitlement.item_type.clone(),
                    item_id: entitlement.item_id.clone(),
                    quantity,
                    metadata: entitlement.description.clone(),
                    expiration_date,
                };
                self.rewards
                    .entry(player_id.to_string())
                    .or_default()
                    .entitlements
                    .push(details.clone());
                let json = serde_json::to_string(&details)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                let mut hasher = Sha256::new();
                hasher.update(json);
                let hash = hex::encode(hasher.finalize());
                let txn = Transaction {
                    transaction_id: Uuid::new_v4().to_string(),
                    player_id: player_id.to_string(),
                    transaction_type: "entitlement".to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    data_hash: hash.clone(),
                    signature: String::new(),
                    details: TransactionData::Entitlement(details),
                };
                self.ledger.add_block(vec![txn]);
                if let Ok(id) = Uuid::parse_str(player_id) {
                    if let Some(b) = self.ledger.get_latest_block() {
                        let block = b.clone();
                        self.storage.append_block(id, &block)?;
                    }
                }
                Ok(())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "profile not found"))
            }
        }

        pub fn award_achievement(&mut self, player_id: &str, achievement: &crate::achievement_registry::AchievementDefinition) -> std::io::Result<()> {
            if self.profiles.get(player_id).is_some() {
                let details = crate::blockchain::Achievement {
                    developer: achievement.developer.clone(),
                    game: achievement.game.clone(),
                    achievement_id: achievement.achievement_id.clone(),
                    version: achievement.version,
                    achievement_name: achievement.name.clone(),
                    criteria: achievement.description.clone(),
                    timestamp_earned: Utc::now().to_rfc3339(),
                    metadata: String::new(),
                };
                self.rewards
                    .entry(player_id.to_string())
                    .or_default()
                    .achievements
                    .push(details.clone());
                let json = serde_json::to_string(&details)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                let mut hasher = Sha256::new();
                hasher.update(json);
                let hash = hex::encode(hasher.finalize());
                let txn = Transaction {
                    transaction_id: Uuid::new_v4().to_string(),
                    player_id: player_id.to_string(),
                    transaction_type: "achievement".to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    data_hash: hash.clone(),
                    signature: String::new(),
                    details: TransactionData::Achievement(details),
                };
                self.ledger.add_block(vec![txn]);
                if let Ok(id) = Uuid::parse_str(player_id) {
                    if let Some(b) = self.ledger.get_latest_block() {
                        let block = b.clone();
                        self.storage.append_block(id, &block)?;
                    }
                }
                Ok(())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "profile not found"))
            }
        }

        fn log_change(&mut self, profile: &PlayerProfile) -> std::io::Result<()> {
            let json = serde_json::to_string(profile)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let mut hasher = Sha256::new();
            hasher.update(json);
            let hash = hex::encode(hasher.finalize());
            let txn = Transaction {
                transaction_id: Uuid::new_v4().to_string(),
                player_id: profile.player_id.clone(),
                transaction_type: "profile_change".to_string(),
                timestamp: Utc::now().to_rfc3339(),
                data_hash: hash.clone(),
                signature: String::new(),
                details: TransactionData::ProfileChange(ProfileChange { profile_hash: hash, profile: profile.clone() }),
            };
            self.ledger.add_block(vec![txn]);
            if let Ok(id) = Uuid::parse_str(&profile.player_id) {
                if let Some(b) = self.ledger.get_latest_block() {
                    let block = b.clone();
                    self.storage.append_block(id, &block)?;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_service::*;
    use crate::hd::{BitVec, hamming_distance};
    use crate::ledger_storage::FileTopicLedgerStorage;
    use crate::blockchain::Block;
    use uuid::Uuid;

    #[test]
    fn test_profile_creation_and_update() {
        let dir = "test_player_logs";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Alice").expect("create profile");
        let vec = BitVec::seed("TEST", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone()).expect("set vector");
        let profile = service.get_profile(&pid).expect("missing profile");
        assert_eq!(hamming_distance(&profile.profile_vec, &vec), 0);
        assert_eq!(service.ledger.chain.len(), 3);
        assert!(service.ledger.is_valid_chain());
        assert_eq!(service.ledger.chain[1].app_version, env!("CARGO_PKG_VERSION"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_award_achievement() {
        let dir = "test_player_logs2";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Bob").expect("create profile");

        let ach = crate::achievement_registry::AchievementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            achievement_id: "ach1".into(),
            version: 1,
            name: "First".into(),
            description: "Earned".into(),
        };

        service.award_achievement(&pid, &ach).expect("award achievement");
        assert_eq!(service.ledger.chain.len(), 3);
        if let crate::blockchain::TransactionData::Achievement(a) = &service.ledger.chain[2].transactions[0].details {
            assert_eq!(a.achievement_id, "ach1");
            assert_eq!(a.version, 1);
        } else {
            panic!("expected achievement transaction");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_award_entitlement() {
        let dir = "test_player_logs3";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Eve").expect("create profile");

        let ent = crate::entitlement_registry::EntitlementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            entitlement_id: "ent1".into(),
            version: 1,
            item_type: "item".into(),
            item_id: "i1".into(),
            description: "desc".into(),
        };

        service.award_entitlement(&pid, &ent, 1, None).expect("award entitlement");
        assert_eq!(service.ledger.chain.len(), 3);
        if let crate::blockchain::TransactionData::Entitlement(e) = &service.ledger.chain[2].transactions[0].details {
            assert_eq!(e.entitlement_id, "ent1");
            assert_eq!(e.version, 1);
            assert!(e.expiration_date.is_none());
        } else {
            panic!("expected entitlement transaction");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_restart_with_persisted_data() {
        let dir = "test_player_logs_restart";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Restart").expect("create profile");
        let vec = BitVec::seed("TEST", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone()).expect("set vector");
        let chain_len = service.ledger.chain.len();
        drop(service);

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), chain_len);
        let profile = service.get_profile(&pid).expect("missing profile");
        assert_eq!(profile.name, "Restart");
        assert_eq!(hamming_distance(&profile.profile_vec, &vec), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_restart_persists_rewards_state() {
        let dir = "test_player_logs_restart_rewards";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Rewards").expect("create profile");

        let ach = crate::achievement_registry::AchievementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            achievement_id: "ach2".into(),
            version: 2,
            name: "Second".into(),
            description: "Earned".into(),
        };

        let ent = crate::entitlement_registry::EntitlementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            entitlement_id: "ent2".into(),
            version: 2,
            item_type: "item".into(),
            item_id: "i2".into(),
            description: "desc".into(),
        };

        service.award_entitlement(&pid, &ent, 2, None).expect("award entitlement");
        service.award_achievement(&pid, &ach).expect("award achievement");
        let chain_len = service.ledger.chain.len();
        drop(service);

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), chain_len);
        let rewards = service.get_reward_state(&pid).expect("missing rewards");
        assert_eq!(rewards.entitlements.len(), 1);
        assert_eq!(rewards.achievements.len(), 1);
        assert_eq!(rewards.entitlements[0].entitlement_id, "ent2");
        assert_eq!(rewards.achievements[0].achievement_id, "ach2");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_rejects_tampered_blocks_on_restart() {
        let dir = "test_player_logs_tampered";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Mallory").expect("create profile");
        let vec = BitVec::seed("TAMPERED", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone()).expect("set vector");
        drop(service);

        let path = format!("{}/{}.log", dir, pid);
        let contents = std::fs::read_to_string(&path).expect("read log");
        let mut blocks: Vec<Block> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse block"))
            .collect();
        assert!(blocks.len() >= 2, "expected at least two blocks");
        blocks[1].previous_block_hash = "tampered".to_string();
        let tampered_contents = blocks
            .into_iter()
            .map(|block| serde_json::to_string(&block).expect("serialize block"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{}\n", tampered_contents)).expect("write log");

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), 2);
        let profile = service.get_profile(&pid).expect("missing profile");
        let default_vec = BitVec::new(DEFAULT_DIM);
        assert_eq!(hamming_distance(&profile.profile_vec, &default_vec), 0);
        assert_ne!(hamming_distance(&profile.profile_vec, &vec), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_rejects_out_of_order_blocks() {
        let dir = "test_player_logs_out_of_order";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "OutOfOrder").expect("create profile");
        let vec1 = BitVec::seed("ORDER1", DEFAULT_DIM);
        let vec2 = BitVec::seed("ORDER2", DEFAULT_DIM);
        service.set_vector(&pid, vec1.clone()).expect("set vector 1");
        service.set_vector(&pid, vec2.clone()).expect("set vector 2");
        drop(service);

        let path = format!("{}/{}.log", dir, pid);
        let contents = std::fs::read_to_string(&path).expect("read log");
        let mut blocks: Vec<Block> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse block"))
            .collect();
        assert!(blocks.len() >= 3, "expected at least three blocks");
        blocks.swap(1, 2);
        let reordered_contents = blocks
            .into_iter()
            .map(|block| serde_json::to_string(&block).expect("serialize block"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{}\n", reordered_contents)).expect("write log");

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), 3);
        let profile = service.get_profile(&pid).expect("missing profile");
        assert_eq!(hamming_distance(&profile.profile_vec, &vec1), 0);
        assert_ne!(hamming_distance(&profile.profile_vec, &vec2), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
