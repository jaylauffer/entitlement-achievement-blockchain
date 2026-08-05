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
pub struct AchievementIdentity {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementPresentation {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementAccomplishment {
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
    pub visibility: AchievementVisibility,
    #[serde(default)]
    pub repeatability: AchievementRepeatability,
    #[serde(default)]
    pub issuance_mode: AchievementIssuanceMode,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementAwardMetadata {
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub visibility: AchievementVisibility,
    #[serde(default)]
    pub repeatability: AchievementRepeatability,
    #[serde(default)]
    pub issuance_mode: AchievementIssuanceMode,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementDefinition {
    #[serde(flatten)]
    pub identity: AchievementIdentity,
    #[serde(flatten)]
    pub presentation: AchievementPresentation,
    #[serde(flatten)]
    pub policy: AchievementAwardPolicy,
    #[serde(default)]
    pub accomplishment: AchievementAccomplishment,
}

impl AchievementDefinition {
    pub fn new(
        developer: impl Into<String>,
        game: impl Into<String>,
        achievement_id: impl Into<String>,
        version: u32,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            identity: AchievementIdentity {
                developer: developer.into(),
                game: game.into(),
                achievement_id: achievement_id.into(),
                version,
            },
            presentation: AchievementPresentation {
                name: name.into(),
                description: description.into(),
                category: String::new(),
            },
            policy: AchievementAwardPolicy::default(),
            accomplishment: AchievementAccomplishment::default(),
        }
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.presentation.category = category.into();
        self
    }

    pub fn with_policy(
        mut self,
        visibility: AchievementVisibility,
        repeatability: AchievementRepeatability,
        issuance_mode: AchievementIssuanceMode,
    ) -> Self {
        self.policy.visibility = visibility;
        self.policy.repeatability = repeatability;
        self.policy.issuance_mode = issuance_mode;
        self
    }

    pub fn with_accomplishment(mut self, accomplishment: AchievementAccomplishment) -> Self {
        self.accomplishment = accomplishment;
        self
    }

    pub fn developer(&self) -> &str {
        &self.identity.developer
    }

    pub fn game(&self) -> &str {
        &self.identity.game
    }

    pub fn achievement_id(&self) -> &str {
        &self.identity.achievement_id
    }

    pub fn version(&self) -> u32 {
        self.identity.version
    }

    pub fn name(&self) -> &str {
        &self.presentation.name
    }

    pub fn description(&self) -> &str {
        &self.presentation.description
    }

    pub fn category(&self) -> &str {
        &self.presentation.category
    }

    pub fn accomplishment_summary(&self) -> &str {
        if self.accomplishment.summary.is_empty() {
            self.description()
        } else {
            &self.accomplishment.summary
        }
    }

    pub fn award_metadata(&self) -> AchievementAwardMetadata {
        AchievementAwardMetadata {
            category: self.category().to_string(),
            visibility: self.policy.visibility.clone(),
            repeatability: self.policy.repeatability.clone(),
            issuance_mode: self.policy.issuance_mode.clone(),
        }
    }

    pub fn allows_direct_award(&self) -> bool {
        matches!(
            self.policy.issuance_mode,
            AchievementIssuanceMode::DirectAwardOnly
                | AchievementIssuanceMode::DirectAwardOrClaimReview
        )
    }

    pub fn allows_claim_review(&self) -> bool {
        matches!(
            self.policy.issuance_mode,
            AchievementIssuanceMode::ClaimReviewOnly
                | AchievementIssuanceMode::DirectAwardOrClaimReview
        )
    }

    pub fn should_be_public_proof(&self) -> bool {
        self.policy.visibility == AchievementVisibility::PublicProof
    }

    pub fn is_repeatable(&self) -> bool {
        self.policy.repeatability == AchievementRepeatability::Repeatable
    }
}
