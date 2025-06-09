pub mod profile_service {
    use std::collections::HashMap;
    use serde::{Serialize, Deserialize};
    use crate::hd::{BitVec, hamming_distance};

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
    }

    impl PlayerProfileService {
        pub fn new() -> Self {
            PlayerProfileService {
                profiles: HashMap::new(),
            }
        }

        pub fn create_profile(&mut self, player_id: &str, name: &str) -> &PlayerProfile {
            let profile = PlayerProfile::new(player_id.to_string(), name.to_string());
            self.profiles.insert(player_id.to_string(), profile);
            self.profiles.get(player_id).unwrap()
        }

        pub fn get_profile(&self, player_id: &str) -> Option<&PlayerProfile> {
            self.profiles.get(player_id)
        }

        pub fn set_vector(&mut self, player_id: &str, vec: BitVec) {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                profile.set_vector(vec);
            }
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
    }
}
