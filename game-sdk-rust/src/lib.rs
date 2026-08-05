mod claim_transport;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use claim_transport::{
    EabClaimTransport, HttpEabClaimTransport, QuicClaimTransportError, QuicEabClaimTransport,
};

pub use eab_core::{
    definition_digest, record_offline_achievement, verify_offline_record_integrity,
    AchievementAccomplishment, AchievementAwardMetadata, AchievementAwardPolicy,
    AchievementDefinition, AchievementIdentity, AchievementIssuanceMode, AchievementPresentation,
    AchievementRepeatability, AchievementVisibility, EabAwardReference, EabClaimAcknowledgement,
    EabClaimDecisionCode, EabClaimDisposition, EabClaimEnvelope, EabClaimEnvelopeError,
    FileOfflineAchievementStorage, MemoryOfflineAchievementStorage, OfflineAchievementContext,
    OfflineAchievementError, OfflineAchievementEvent, OfflineAchievementRecord,
    OfflineAchievementStorage, OfflineAwardOutcome, OfflineClaimReadiness,
    EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION, EAB_CLAIM_ENVELOPE_SCHEMA_VERSION,
};

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("http error: {0}")]
    Http(String),
    #[error("server returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("offline achievement is not ready for claim submission: {0}")]
    OfflineClaimNotReady(String),
    #[error("offline achievement record is invalid: {0}")]
    OfflineRecord(String),
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

    pub fn get_profile(
        &self,
        player_id: &str,
        player_token: &str,
    ) -> Result<PlayerProfile, SdkError> {
        self.get_json(&format!("/profiles/{player_id}"), Some(player_token))
    }

    pub fn get_rewards(
        &self,
        player_id: &str,
        player_token: &str,
    ) -> Result<PlayerRewardState, SdkError> {
        self.get_json(
            &format!("/profiles/{player_id}/rewards"),
            Some(player_token),
        )
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
        developer_token: &str,
        request: &AwardAchievementRequest,
    ) -> Result<AwardReceipt, SdkError> {
        self.post_json(
            &format!("/profiles/{player_id}/achievements"),
            Some(developer_token),
            request,
        )
    }

    pub fn submit_entitlement_award(
        &self,
        player_id: &str,
        developer_token: &str,
        request: &AwardEntitlementRequest,
    ) -> Result<AwardReceipt, SdkError> {
        self.post_json(
            &format!("/profiles/{player_id}/entitlements"),
            Some(developer_token),
            request,
        )
    }

    pub fn submit_achievement_claim(
        &self,
        player_id: &str,
        player_token: &str,
        request: &SubmitAchievementClaimRequest,
    ) -> Result<AchievementClaim, SdkError> {
        self.post_json(
            &format!("/profiles/{player_id}/achievement-claims"),
            Some(player_token),
            request,
        )
    }

    pub fn list_achievement_claims(
        &self,
        player_id: &str,
        player_token: &str,
    ) -> Result<Vec<AchievementClaim>, SdkError> {
        self.get_json(
            &format!("/profiles/{player_id}/achievement-claims"),
            Some(player_token),
        )
    }

    pub fn submit_canonical_achievement_claim(
        &self,
        player_id: &str,
        player_token: &str,
        envelope: &EabClaimEnvelope,
    ) -> Result<EabClaimAcknowledgement, SdkError> {
        self.post_json(
            &format!("/profiles/{player_id}/achievement-claim-envelopes"),
            Some(player_token),
            envelope,
        )
    }

    pub fn get_claim_acknowledgement(
        &self,
        player_id: &str,
        player_token: &str,
        claim_id: &str,
    ) -> Result<Option<EabClaimAcknowledgement>, SdkError> {
        self.get_optional_json(
            &format!("/profiles/{player_id}/achievement-claims/{claim_id}/acknowledgement"),
            Some(player_token),
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

        match req.send_json(
            serde_json::to_value(body).map_err(|err| SdkError::Serialization(err.to_string()))?,
        ) {
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

        match req.send_json(
            serde_json::to_value(body).map_err(|err| SdkError::Serialization(err.to_string()))?,
        ) {
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

    fn get_optional_json<TResp: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        bearer_token: Option<&str>,
    ) -> Result<Option<TResp>, SdkError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::get(&url);
        if let Some(token) = bearer_token {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        match req.call() {
            Ok(response) => response
                .into_json::<TResp>()
                .map(Some)
                .map_err(|err| SdkError::Serialization(err.to_string())),
            Err(ureq::Error::Status(404, _)) => Ok(None),
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
pub struct SubmitAchievementClaimRequest {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub claim_id: String,
    pub session_id: String,
    pub client_sequence: u64,
    pub claimed_at: String,
    pub evidence: Option<String>,
}

impl TryFrom<&OfflineAchievementRecord> for SubmitAchievementClaimRequest {
    type Error = SdkError;

    fn try_from(record: &OfflineAchievementRecord) -> Result<Self, Self::Error> {
        let integrity_ok = verify_offline_record_integrity(record)
            .map_err(|err| SdkError::OfflineRecord(err.to_string()))?;
        if !integrity_ok {
            return Err(SdkError::OfflineRecord(
                "integrity verification failed".to_string(),
            ));
        }
        if record.claim_readiness != OfflineClaimReadiness::Ready {
            return Err(SdkError::OfflineClaimNotReady(format!(
                "{:?}",
                record.claim_readiness
            )));
        }
        Ok(Self {
            developer: record.developer.clone(),
            game: record.game.clone(),
            achievement_id: record.achievement_id.clone(),
            version: record.version,
            claim_id: record.claim_id.clone(),
            session_id: record.session_id.clone(),
            client_sequence: record.client_sequence,
            claimed_at: record.earned_at_local.clone(),
            evidence: record.evidence.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AchievementClaimStatus {
    Pending,
    Promoted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AchievementClaim {
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub claim_id: String,
    pub session_id: String,
    pub client_sequence: u64,
    pub claimed_at: String,
    pub evidence: Option<String>,
    pub submitted_at: String,
    pub status: AchievementClaimStatus,
    pub reviewed_at: Option<String>,
    pub reviewer: Option<String>,
    pub review_note: Option<String>,
    pub awarded_transaction_id: Option<String>,
    pub awarded_block_hash: Option<String>,
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
