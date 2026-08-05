use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::AchievementDefinition;

const OFFLINE_RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum OfflineAchievementError {
    #[error("invalid achievement definition: {0}")]
    InvalidDefinition(String),
    #[error("invalid offline achievement context: {0}")]
    InvalidContext(String),
    #[error("invalid offline achievement event: {0}")]
    InvalidEvent(String),
    #[error("repeatable offline achievements are not supported by this first evaluator")]
    RepeatableNotSupported,
    #[error("offline achievement record integrity check failed")]
    Integrity,
    #[error("duplicate offline achievement identity: {0}")]
    DuplicateIdentity(String),
    #[error("offline achievement serialization failed: {0}")]
    Serialization(String),
    #[error(transparent)]
    Storage(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfflineAchievementEvent {
    pub event_key: String,
    pub value: u64,
    pub occurred_at: String,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfflineAchievementContext {
    pub local_player_id: String,
    pub save_id: String,
    pub installation_id: String,
    pub session_id: String,
    pub client_sequence: u64,
    pub game_build: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfflineClaimReadiness {
    Ready,
    NotAllowedByIssuancePolicy,
    MissingRequiredEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfflineAchievementRecord {
    pub schema_version: u32,
    pub local_award_id: String,
    pub claim_id: String,
    pub developer: String,
    pub game: String,
    pub achievement_id: String,
    pub version: u32,
    pub definition_digest: String,
    pub local_player_id: String,
    pub save_id: String,
    pub installation_id: String,
    pub session_id: String,
    pub client_sequence: u64,
    pub earned_at_local: String,
    pub recorded_at_local: String,
    pub game_build: String,
    pub event_key: String,
    pub event_value: u64,
    pub evidence: Option<String>,
    pub claim_readiness: OfflineClaimReadiness,
    pub local_record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfflineAwardOutcome {
    NoMatchingEvent,
    ThresholdNotMet { observed: u64, required: u64 },
    Awarded(OfflineAchievementRecord),
    AlreadyAwarded(OfflineAchievementRecord),
}

pub trait OfflineAchievementStorage {
    fn records(&self) -> &[OfflineAchievementRecord];

    fn append(&mut self, record: &OfflineAchievementRecord) -> Result<(), OfflineAchievementError>;
}

#[derive(Debug, Default)]
pub struct MemoryOfflineAchievementStorage {
    records: Vec<OfflineAchievementRecord>,
}

impl MemoryOfflineAchievementStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl OfflineAchievementStorage for MemoryOfflineAchievementStorage {
    fn records(&self) -> &[OfflineAchievementRecord] {
        &self.records
    }

    fn append(&mut self, record: &OfflineAchievementRecord) -> Result<(), OfflineAchievementError> {
        validate_new_identity(&self.records, record)?;
        if !verify_offline_record_integrity(record)? {
            return Err(OfflineAchievementError::Integrity);
        }
        self.records.push(record.clone());
        Ok(())
    }
}

/// Single-writer JSON-lines reference storage for immutable offline EAB records.
pub struct FileOfflineAchievementStorage {
    path: PathBuf,
    records: Vec<OfflineAchievementRecord>,
}

impl FileOfflineAchievementStorage {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, OfflineAchievementError> {
        let path = path.into();
        let mut records = Vec::new();
        if path.exists() {
            let reader = BufReader::new(File::open(&path)?);
            for (index, line) in reader.lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let record: OfflineAchievementRecord =
                    serde_json::from_str(&line).map_err(|err| {
                        OfflineAchievementError::Serialization(format!(
                            "record {} could not be decoded: {err}",
                            index + 1
                        ))
                    })?;
                if !verify_offline_record_integrity(&record)? {
                    return Err(OfflineAchievementError::Integrity);
                }
                validate_new_identity(&records, &record)?;
                records.push(record);
            }
        }
        Ok(Self { path, records })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl OfflineAchievementStorage for FileOfflineAchievementStorage {
    fn records(&self) -> &[OfflineAchievementRecord] {
        &self.records
    }

    fn append(&mut self, record: &OfflineAchievementRecord) -> Result<(), OfflineAchievementError> {
        validate_new_identity(&self.records, record)?;
        if !verify_offline_record_integrity(record)? {
            return Err(OfflineAchievementError::Integrity);
        }
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, record)
            .map_err(|err| OfflineAchievementError::Serialization(err.to_string()))?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_data()?;
        self.records.push(record.clone());
        Ok(())
    }
}

/// Evaluates one game event and, when earned, persists one immutable EAB offline record.
///
/// The returned `Awarded` record is immediately usable as a game-scoped receipt and later
/// maps to an online achievement claim without changing its `claim_id`.
pub fn record_offline_achievement<S: OfflineAchievementStorage>(
    storage: &mut S,
    definition: &AchievementDefinition,
    event: &OfflineAchievementEvent,
    context: &OfflineAchievementContext,
) -> Result<OfflineAwardOutcome, OfflineAchievementError> {
    validate_definition(definition)?;
    validate_context(context)?;
    validate_event(event)?;

    let expected_event = definition
        .accomplishment
        .event_key
        .as_deref()
        .ok_or_else(|| {
            OfflineAchievementError::InvalidDefinition(
                "offline evaluation requires accomplishment.event_key".to_string(),
            )
        })?;
    if event.event_key != expected_event {
        return Ok(OfflineAwardOutcome::NoMatchingEvent);
    }

    if definition.is_repeatable() {
        return Err(OfflineAchievementError::RepeatableNotSupported);
    }
    let required = definition.accomplishment.threshold.unwrap_or(1);
    if required == 0 {
        return Err(OfflineAchievementError::InvalidDefinition(
            "accomplishment.threshold must be greater than zero".to_string(),
        ));
    }
    if event.value < required {
        return Ok(OfflineAwardOutcome::ThresholdNotMet {
            observed: event.value,
            required,
        });
    }

    if let Some(existing) = storage.records().iter().find(|record| {
        record.local_player_id == context.local_player_id
            && record.developer == definition.developer()
            && record.game == definition.game()
            && record.achievement_id == definition.achievement_id()
    }) {
        return Ok(OfflineAwardOutcome::AlreadyAwarded(existing.clone()));
    }

    let claim_readiness = if !definition.allows_claim_review() {
        OfflineClaimReadiness::NotAllowedByIssuancePolicy
    } else if definition.accomplishment.requires_evidence && event.evidence.is_none() {
        OfflineClaimReadiness::MissingRequiredEvidence
    } else {
        OfflineClaimReadiness::Ready
    };
    let now = Utc::now().to_rfc3339();
    let mut record = OfflineAchievementRecord {
        schema_version: OFFLINE_RECORD_SCHEMA_VERSION,
        local_award_id: Uuid::new_v4().to_string(),
        claim_id: Uuid::new_v4().to_string(),
        developer: definition.developer().to_string(),
        game: definition.game().to_string(),
        achievement_id: definition.achievement_id().to_string(),
        version: definition.version(),
        definition_digest: definition_digest(definition)?,
        local_player_id: context.local_player_id.clone(),
        save_id: context.save_id.clone(),
        installation_id: context.installation_id.clone(),
        session_id: context.session_id.clone(),
        client_sequence: context.client_sequence,
        earned_at_local: event.occurred_at.clone(),
        recorded_at_local: now,
        game_build: context.game_build.clone(),
        event_key: event.event_key.clone(),
        event_value: event.value,
        evidence: event.evidence.clone(),
        claim_readiness,
        local_record_hash: String::new(),
    };
    record.local_record_hash = offline_record_hash(&record)?;
    storage.append(&record)?;
    Ok(OfflineAwardOutcome::Awarded(record))
}

pub fn definition_digest(
    definition: &AchievementDefinition,
) -> Result<String, OfflineAchievementError> {
    sha256_json(definition)
}

pub fn verify_offline_record_integrity(
    record: &OfflineAchievementRecord,
) -> Result<bool, OfflineAchievementError> {
    Ok(record.schema_version == OFFLINE_RECORD_SCHEMA_VERSION
        && !record.local_record_hash.is_empty()
        && offline_record_hash(record)? == record.local_record_hash)
}

fn offline_record_hash(
    record: &OfflineAchievementRecord,
) -> Result<String, OfflineAchievementError> {
    let mut material = record.clone();
    material.local_record_hash.clear();
    sha256_json(&material)
}

fn sha256_json<T: Serialize>(value: &T) -> Result<String, OfflineAchievementError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|err| OfflineAchievementError::Serialization(err.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn validate_definition(definition: &AchievementDefinition) -> Result<(), OfflineAchievementError> {
    for (name, value) in [
        ("developer", definition.developer()),
        ("game", definition.game()),
        ("achievement_id", definition.achievement_id()),
    ] {
        if value.trim().is_empty() {
            return Err(OfflineAchievementError::InvalidDefinition(format!(
                "{name} must not be empty"
            )));
        }
    }
    if definition.version() == 0 {
        return Err(OfflineAchievementError::InvalidDefinition(
            "version must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn validate_context(context: &OfflineAchievementContext) -> Result<(), OfflineAchievementError> {
    for (name, value) in [
        ("local_player_id", context.local_player_id.as_str()),
        ("save_id", context.save_id.as_str()),
        ("installation_id", context.installation_id.as_str()),
        ("session_id", context.session_id.as_str()),
        ("game_build", context.game_build.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(OfflineAchievementError::InvalidContext(format!(
                "{name} must not be empty"
            )));
        }
    }
    Ok(())
}

fn validate_event(event: &OfflineAchievementEvent) -> Result<(), OfflineAchievementError> {
    if event.event_key.trim().is_empty() {
        return Err(OfflineAchievementError::InvalidEvent(
            "event_key must not be empty".to_string(),
        ));
    }
    if event.occurred_at.trim().is_empty() {
        return Err(OfflineAchievementError::InvalidEvent(
            "occurred_at must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_new_identity(
    records: &[OfflineAchievementRecord],
    record: &OfflineAchievementRecord,
) -> Result<(), OfflineAchievementError> {
    if records.iter().any(|existing| {
        existing.local_award_id == record.local_award_id || existing.claim_id == record.claim_id
    }) {
        return Err(OfflineAchievementError::DuplicateIdentity(
            record.claim_id.clone(),
        ));
    }
    Ok(())
}
