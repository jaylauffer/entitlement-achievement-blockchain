#![doc = include_str!("../README.md")]

mod achievement;
mod claim;
mod offline;

pub use achievement::{
    AchievementAccomplishment, AchievementAwardMetadata, AchievementAwardPolicy,
    AchievementDefinition, AchievementIdentity, AchievementIssuanceMode, AchievementPresentation,
    AchievementRepeatability, AchievementVisibility,
};
pub use claim::{
    EabAwardReference, EabClaimAcknowledgement, EabClaimDecisionCode, EabClaimDisposition,
    EabClaimEnvelope, EabClaimEnvelopeError, EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION,
    EAB_CLAIM_ENVELOPE_SCHEMA_VERSION,
};
pub use offline::{
    definition_digest, record_offline_achievement, verify_offline_record_integrity,
    FileOfflineAchievementStorage, MemoryOfflineAchievementStorage, OfflineAchievementContext,
    OfflineAchievementError, OfflineAchievementEvent, OfflineAchievementRecord,
    OfflineAchievementStorage, OfflineAwardOutcome, OfflineClaimReadiness,
};
