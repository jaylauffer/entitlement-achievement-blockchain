use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("http error: {0}")]
    Http(String),
    #[error("server returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("serialization error: {0}")]
    Serialization(String),
}

#[derive(Debug, Clone)]
pub struct EabClient {
    base_url: String,
}

impl EabClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub fn exchange_identity(
        &self,
        provider: &str,
        token: &str,
    ) -> Result<IdentityExchangeResponse, SdkError> {
        self.post_json::<_, IdentityExchangeResponse>(
            "/identity/exchange",
            None,
            &IdentityExchangeRequest {
                provider: provider.to_string(),
                token: token.to_string(),
            },
        )
    }

    pub fn create_profile(
        &self,
        player_token: &str,
        name: &str,
    ) -> Result<PlayerProfile, SdkError> {
        self.post_json::<_, PlayerProfile>(
            "/profiles",
            Some(player_token),
            &CreateProfileRequest {
                name: name.to_string(),
            },
        )
    }

    pub fn get_profile(&self, player_id: &str, player_token: &str) -> Result<PlayerProfile, SdkError> {
        self.get_json(&format!("/profiles/{player_id}"), Some(player_token))
    }

    pub fn get_rewards(
        &self,
        player_id: &str,
        player_token: &str,
    ) -> Result<PlayerRewardState, SdkError> {
        self.get_json(&format!("/profiles/{player_id}/rewards"), Some(player_token))
    }

    pub fn register_achievement(
        &self,
        developer_token: &str,
        request: &RegisterAchievementRequest,
    ) -> Result<(), SdkError> {
        self.post_json_no_content("/achievements", Some(developer_token), request)
    }

    pub fn register_entitlement(
        &self,
        developer_token: &str,
        request: &RegisterEntitlementRequest,
    ) -> Result<(), SdkError> {
        self.post_json_no_content("/entitlements", Some(developer_token), request)
    }

    pub fn submit_achievement_award(
        &self,
        player_id: &str,
        player_token: &str,
        request: &AwardAchievementRequest,
    ) -> Result<AwardReceipt, SdkError> {
        self.post_json(
            &format!("/profiles/{player_id}/achievements"),
            Some(player_token),
            request,
        )
    }

    pub fn submit_entitlement_award(
        &self,
        player_id: &str,
        player_token: &str,
        request: &AwardEntitlementRequest,
    ) -> Result<AwardReceipt, SdkError> {
        self.post_json(
            &format!("/profiles/{player_id}/entitlements"),
            Some(player_token),
            request,
        )
    }

    pub fn verify_receipt_integrity(receipt: &AwardReceipt) -> Result<bool, SdkError> {
        let payload = serde_json::to_string(&receipt.details)
            .map_err(|err| SdkError::Serialization(err.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        let expected = hex::encode(hasher.finalize());
        Ok(expected == receipt.data_hash && !receipt.block_hash.is_empty())
    }

    fn post_json_no_content<T: Serialize>(
        &self,
        path: &str,
        bearer_token: Option<&str>,
        body: &T,
    ) -> Result<(), SdkError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::post(&url);
        if let Some(token) = bearer_token {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        match req.send_json(serde_json::to_value(body).map_err(|err| SdkError::Serialization(err.to_string()))?) {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(status, response)) => Err(SdkError::Status {
                status,
                body: response.into_string().unwrap_or_default(),
            }),
            Err(err) => Err(SdkError::Http(err.to_string())),
        }
    }

    fn post_json<TReq: Serialize, TResp: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        bearer_token: Option<&str>,
        body: &TReq,
    ) -> Result<TResp, SdkError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::post(&url);
        if let Some(token) = bearer_token {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        match req.send_json(serde_json::to_value(body).map_err(|err| SdkError::Serialization(err.to_string()))?) {
            Ok(response) => response
                .into_json::<TResp>()
                .map_err(|err| SdkError::Serialization(err.to_string())),
            Err(ureq::Error::Status(status, response)) => Err(SdkError::Status {
                status,
                body: response.into_string().unwrap_or_default(),
            }),
            Err(err) => Err(SdkError::Http(err.to_string())),
        }
    }

    fn get_json<TResp: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        bearer_token: Option<&str>,
    ) -> Result<TResp, SdkError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::get(&url);
        if let Some(token) = bearer_token {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        match req.call() {
            Ok(response) => response
                .into_json::<TResp>()
                .map_err(|err| SdkError::Serialization(err.to_string())),
            Err(ureq::Error::Status(status, response)) => Err(SdkError::Status {
                status,
                body: response.into_string().unwrap_or_default(),
            }),
            Err(err) => Err(SdkError::Http(err.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityExchangeRequest {
    pub provider: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityExchangeResponse {
    pub access_token: String,
    pub player_id: String,
    pub is_new_player: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProfileRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub player_id: String,
    pub name: String,
    pub profile_vec: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerRewardState {
    pub entitlements: Vec<EntitlementAward>,
    pub achievements: Vec<AchievementAward>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitlementAward {
    pub developer: String,
    pub game: String,
    pub entitlement_id: String,
    pub version: u32,
    pub item_type: String,
    pub item_id: String,
    pub quantity: u32,
    pub metadata: String,
    pub expiration_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AchievementAward {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub achievement_name: String,
    pub criteria: String,
    pub timestamp_earned: String,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAchievementRequest {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterEntitlementRequest {
    pub developer: String,
    pub game: String,
    pub entitlement_id: String,
    pub version: u32,
    pub item_type: String,
    pub item_id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardAchievementRequest {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardEntitlementRequest {
    pub developer: String,
    pub game: String,
    pub entitlement_id: String,
    pub version: u32,
    pub quantity: u32,
    pub expiration_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardReceipt {
    pub player_id: String,
    pub transaction_id: String,
    pub transaction_type: String,
    pub timestamp: String,
    pub data_hash: String,
    pub block_hash: String,
    pub details: ReceiptDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReceiptDetails {
    Entitlement(EntitlementAward),
    Achievement(AchievementAward),
    ProfileChange(serde_json::Value),
}
