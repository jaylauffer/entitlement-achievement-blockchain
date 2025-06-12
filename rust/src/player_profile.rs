pub mod profile_service {
    use std::collections::HashMap;
    use serde::{Serialize, Deserialize};
    use sha2::{Sha256, Digest};
    use chrono::prelude::*;
    use uuid::Uuid;
    use crate::hd::BitVec;
    use crate::blockchain::{Blockchain, Transaction, TransactionData, ProfileChange};
    use crate::ledger_storage::LedgerStorage;

    pub const DEFAULT_DIM: usize = 16384;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PlayerProfile {
        pub player_id: String,
        pub name: String,
        pub profile_vec: BitVec,
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
        pub ledger: Blockchain,
        storage: Box<dyn LedgerStorage + Send + Sync>,
    }

    impl PlayerProfileService {
        pub fn new(storage: Box<dyn LedgerStorage + Send + Sync>) -> Self {
            PlayerProfileService {
                profiles: HashMap::new(),
                ledger: Blockchain::new(),
                storage,
            }
        }

        pub fn create_profile(&mut self, player_id: &str, name: &str) -> &PlayerProfile {
            let profile = PlayerProfile::new(player_id.to_string(), name.to_string());
            self.profiles.insert(player_id.to_string(), profile);
            let cloned = self.profiles.get(player_id).unwrap().clone();
            self.log_change(&cloned);
            self.profiles.get(player_id).unwrap()
        }

        pub fn get_profile(&self, player_id: &str) -> Option<&PlayerProfile> {
            self.profiles.get(player_id)
        }

        pub fn set_vector(&mut self, player_id: &str, vec: BitVec) {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                profile.set_vector(vec);
                let cloned = profile.clone();
                self.log_change(&cloned);
            }
        }

        pub fn merge_vector(&mut self, player_id: &str, vec: &BitVec) {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                let new_vec = profile.profile_vec.xor(vec);
                profile.set_vector(new_vec);
                let cloned = profile.clone();
                self.log_change(&cloned);
            }
        }

        pub fn award_achievement(&mut self, player_id: &str, achievement: &crate::achievement_registry::AchievementDefinition) {
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
                let json = serde_json::to_string(&details).unwrap();
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
                    let block = self.ledger.get_latest_block().clone();
                    let _ = self.storage.append_block(id, &block);
                }
            }
        }

        fn log_change(&mut self, profile: &PlayerProfile) {
            let json = serde_json::to_string(profile).unwrap();
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
                details: TransactionData::ProfileChange(ProfileChange { profile_hash: hash }),
            };
            self.ledger.add_block(vec![txn]);
            if let Ok(id) = Uuid::parse_str(&profile.player_id) {
                let block = self.ledger.get_latest_block().clone();
                let _ = self.storage.append_block(id, &block);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_service::*;
    use crate::hd::{BitVec, hamming_distance};
    use crate::ledger_storage::FileTopicLedgerStorage;
    use uuid::Uuid;

    #[test]
    fn test_profile_creation_and_update() {
        let dir = "test_player_logs";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Alice");
        let vec = BitVec::seed("TEST", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone());
        let profile = service.get_profile(&pid).unwrap();
        assert_eq!(hamming_distance(&profile.profile_vec, &vec), 0);
        assert_eq!(service.ledger.chain.len(), 3);
        assert!(service.ledger.is_valid_chain());
        assert_eq!(service.ledger.chain[1].app_version, env!("CARGO_PKG_VERSION"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_award_achievement() {
        let dir = "test_player_logs2";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Bob");

        let ach = crate::achievement_registry::AchievementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            achievement_id: "ach1".into(),
            version: 1,
            name: "First".into(),
            description: "Earned".into(),
        };

        service.award_achievement(&pid, &ach);
        assert_eq!(service.ledger.chain.len(), 3);
        if let crate::blockchain::TransactionData::Achievement(a) = &service.ledger.chain[2].transactions[0].details {
            assert_eq!(a.achievement_id, "ach1");
            assert_eq!(a.version, 1);
        } else {
            panic!("expected achievement transaction");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
