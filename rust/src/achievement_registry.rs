use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AchievementVisibility {
    #[default]
    Private,
    PublicProof,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AchievementRepeatability {
    #[default]
    OncePerPlayer,
    Repeatable,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AchievementIssuanceMode {
    DirectAwardOnly,
    ClaimReviewOnly,
    #[default]
    DirectAwardOrClaimReview,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementSuccessCriteria {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub event_key: Option<String>,
    #[serde(default)]
    pub threshold: Option<u64>,
    #[serde(default)]
    pub requires_evidence: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementAwardPolicy {
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub visibility: AchievementVisibility,
    #[serde(default)]
    pub repeatability: AchievementRepeatability,
    #[serde(default)]
    pub issuance_mode: AchievementIssuanceMode,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AchievementDefinition {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub visibility: AchievementVisibility,
    #[serde(default)]
    pub repeatability: AchievementRepeatability,
    #[serde(default)]
    pub issuance_mode: AchievementIssuanceMode,
    #[serde(default)]
    pub success_criteria: AchievementSuccessCriteria,
}

impl AchievementDefinition {
    pub fn criteria_summary(&self) -> &str {
        if self.success_criteria.summary.is_empty() {
            &self.description
        } else {
            &self.success_criteria.summary
        }
    }

    pub fn award_policy(&self) -> AchievementAwardPolicy {
        AchievementAwardPolicy {
            category: self.category.clone(),
            visibility: self.visibility.clone(),
            repeatability: self.repeatability.clone(),
            issuance_mode: self.issuance_mode.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct AchievementRegistry {
    achievements: HashMap<String, AchievementDefinition>,
}

impl AchievementRegistry {
    pub fn load(path: &str) -> std::io::Result<Self> {
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

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn insert(&mut self, def: AchievementDefinition) {
        let key = Self::key(&def.developer, &def.game, &def.achievement_id, def.version);
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
