pub mod profile_service {
    use std::collections::HashMap;
    use serde::{Serialize, Deserialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PlayerProfile {
        pub player_id: String,
        pub name: String,
        pub hyper_dimensions: Vec<f32>,
    }

    impl PlayerProfile {
        pub fn new(player_id: String, name: String) -> Self {
            PlayerProfile {
                player_id,
                name,
                hyper_dimensions: Vec::new(),
            }
        }


        pub fn set_dimensions(&mut self, dims: Vec<f32>) {
            self.hyper_dimensions = dims;
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

        pub fn set_dimensions(&mut self, player_id: &str, dims: Vec<f32>) {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                profile.set_dimensions(dims);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_service::*;

    #[test]
    fn test_profile_creation_and_update() {
        let mut service = PlayerProfileService::new();
        service.create_profile("player1", "Alice");
        service.set_dimensions("player1", vec![0.1, 0.2, 0.3]);
        let profile = service.get_profile("player1").unwrap();
        assert_eq!(profile.hyper_dimensions, vec![0.1, 0.2, 0.3]);
    }
}
