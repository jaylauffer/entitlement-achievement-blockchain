use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use eab_core::{
    AchievementAccomplishment, AchievementAwardMetadata, AchievementAwardPolicy,
    AchievementDefinition, AchievementIdentity, AchievementIssuanceMode, AchievementPresentation,
    AchievementRepeatability, AchievementVisibility,
};

#[derive(Serialize, Deserialize, Default)]
pub struct AchievementRegistry {
    achievements: HashMap<String, AchievementDefinition>,
}

impl AchievementRegistry {
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        match File::open(path) {
            Ok(mut f) => {
                let mut data = String::new();
                f.read_to_string(&mut data)?;
                let reg: AchievementRegistry = serde_json::from_str(&data).unwrap_or_default();
                Ok(reg)
            }
            Err(_) => Ok(AchievementRegistry::default()),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn insert(&mut self, def: AchievementDefinition) {
        let key = Self::key(
            def.developer(),
            def.game(),
            def.achievement_id(),
            def.version(),
        );
        self.achievements.insert(key, def);
    }

    pub fn get(
        &self,
        developer: &str,
        game: &str,
        id: &str,
        version: u32,
    ) -> Option<&AchievementDefinition> {
        let key = Self::key(developer, game, id, version);
        self.achievements.get(&key)
    }

    fn key(dev: &str, game: &str, id: &str, version: u32) -> String {
        format!("{}:{}:{}:v{}", dev, game, id, version)
    }
}
