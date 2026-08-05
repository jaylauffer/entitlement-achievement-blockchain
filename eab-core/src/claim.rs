use serde::{Deserialize, Serialize};

use crate::{verify_offline_record_integrity, OfflineAchievementRecord, OfflineClaimReadiness};

pub const EAB_CLAIM_ENVELOPE_SCHEMA_VERSION: u32 = 1;
pub const EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION: u32 = 1;

/// Canonical transport-neutral payload for presenting one embedded EAB occurrence.
///
/// Authenticated account binding is intentionally not part of this client-controlled
/// envelope. A transport authenticates a player session and passes the bound player id to
/// the authoritative service separately.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EabClaimEnvelope {
    pub schema_version: u32,
    pub record: OfflineAchievementRecord,
}

impl EabClaimEnvelope {
    pub fn from_offline_record(
        record: &OfflineAchievementRecord,
    ) -> Result<Self, EabClaimEnvelopeError> {
        let envelope = Self {
            schema_version: EAB_CLAIM_ENVELOPE_SCHEMA_VERSION,
            record: record.clone(),
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<(), EabClaimEnvelopeError> {
        if self.schema_version != EAB_CLAIM_ENVELOPE_SCHEMA_VERSION {
            return Err(EabClaimEnvelopeError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        let integrity_ok = verify_offline_record_integrity(&self.record)
            .map_err(|err| EabClaimEnvelopeError::InvalidRecord(err.to_string()))?;
        if !integrity_ok {
            return Err(EabClaimEnvelopeError::InvalidRecord(
                "integrity verification failed".to_string(),
            ));
        }
        if self.record.claim_readiness != OfflineClaimReadiness::Ready {
            return Err(EabClaimEnvelopeError::NotReady(
                self.record.claim_readiness.clone(),
            ));
        }
        Ok(())
    }

    pub fn claim_id(&self) -> &str {
        &self.record.claim_id
    }
}

impl TryFrom<&OfflineAchievementRecord> for EabClaimEnvelope {
    type Error = EabClaimEnvelopeError;

    fn try_from(record: &OfflineAchievementRecord) -> Result<Self, Self::Error> {
        Self::from_offline_record(record)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EabClaimEnvelopeError {
    #[error("unsupported claim envelope schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("invalid offline EAB record: {0}")]
    InvalidRecord(String),
    #[error("offline EAB record is not ready for claim submission: {0:?}")]
    NotReady(OfflineClaimReadiness),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EabClaimDisposition {
    Pending,
    Acknowledged,
    Rejected,
    Conflict,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EabClaimDecisionCode {
    PendingReview,
    Acknowledged,
    AlreadyAcknowledged,
    InvalidEnvelope,
    ClaimNotReady,
    ClaimIdPayloadMismatch,
    DefinitionNotFound,
    DefinitionIdentityMismatch,
    DefinitionDigestMismatch,
    IssuanceModeDisallowsClaim,
    EvidenceRequired,
    EventMismatch,
    ThresholdNotMet,
    RepeatableNotSupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EabAwardReference {
    pub transaction_id: String,
    pub block_hash: String,
}

/// Transport-neutral result of authoritative EAB claim processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EabClaimAcknowledgement {
    pub schema_version: u32,
    pub claim_id: String,
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub disposition: EabClaimDisposition,
    pub code: EabClaimDecisionCode,
    pub first_observed_at: String,
    pub decided_at: Option<String>,
    pub award: Option<EabAwardReference>,
}
