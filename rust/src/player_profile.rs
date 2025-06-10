pub mod profile_service {
    use std::collections::HashMap;
    use serde::{Serialize, Deserialize};
    use sha2::{Sha256, Digest};
    use chrono::prelude::*;
    use uuid::Uuid;
    use crate::hd::BitVec;
    use crate::blockchain::{Blockchain, Transaction, TransactionData, ProfileChange};

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
    }

    impl PlayerProfileService {
        pub fn new() -> Self {
            PlayerProfileService {
                profiles: HashMap::new(),
                ledger: Blockchain::new(),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_service::*;
    use crate::hd::{BitVec, hamming_distance};

    #[test]
    fn test_profile_creation_and_update() {
        let mut service = PlayerProfileService::new();
        service.create_profile("player1", "Alice");
        let vec = BitVec::seed("TEST", DEFAULT_DIM);
        service.set_vector("player1", vec.clone());
        let profile = service.get_profile("player1").unwrap();
        assert_eq!(hamming_distance(&profile.profile_vec, &vec), 0);
        assert_eq!(service.ledger.chain.len(), 3);
        assert!(service.ledger.is_valid_chain());
        assert_eq!(service.ledger.chain[1].app_version, env!("CARGO_PKG_VERSION"));
    }
}
