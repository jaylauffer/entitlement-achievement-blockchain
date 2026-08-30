pub mod profile_service {
    use crate::blockchain::{Block, Blockchain, ProfileChange, Transaction, TransactionData};
    use crate::hd::BitVec;
    use crate::ledger_storage::LedgerStorage;
    use chrono::prelude::*;
    use eab_core::{
        definition_digest, EabAwardReference, EabClaimAcknowledgement, EabClaimDecisionCode,
        EabClaimDisposition, EabClaimEnvelope, EabClaimEnvelopeError,
        EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION,
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use uuid::Uuid;

    pub const DEFAULT_DIM: usize = 16384;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PlayerProfile {
        pub player_id: String,
        pub name: String,
        pub profile_vec: BitVec,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct PlayerRewardState {
        pub entitlements: Vec<crate::blockchain::Entitlement>,
        pub achievements: Vec<crate::blockchain::Achievement>,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    pub enum AchievementClaimStatus {
        Pending,
        Promoted,
        Rejected,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub canonical_envelope: Option<EabClaimEnvelope>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub acknowledgement: Option<EabClaimAcknowledgement>,
    }

    #[derive(Debug, Clone)]
    pub struct AchievementClaimInput {
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

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AwardRecord {
        pub player_id: String,
        pub transaction_id: String,
        pub transaction_type: String,
        pub timestamp: String,
        pub data_hash: String,
        pub block_hash: String,
        pub details: TransactionData,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AchievementClaimReviewAction {
        Promote,
        Reject,
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
        rewards: HashMap<String, PlayerRewardState>,
        achievement_claims: HashMap<String, Vec<AchievementClaim>>,
        pub ledger: Blockchain,
        storage: Box<dyn LedgerStorage + Send + Sync>,
    }

    impl PlayerProfileService {
        pub fn new(storage: Box<dyn LedgerStorage + Send + Sync>) -> Self {
            let mut service = PlayerProfileService {
                profiles: HashMap::new(),
                rewards: HashMap::new(),
                achievement_claims: HashMap::new(),
                ledger: Blockchain::new(),
                storage,
            };
            if let Ok(ids) = service.storage.list_player_ids() {
                let mut verified_blocks = Vec::new();
                let mut _quarantined_blocks: Vec<Block> = Vec::new();
                for id in ids {
                    if let Ok(b) = service.storage.load_blocks(id) {
                        let (valid, quarantined) = Self::verify_player_blocks(b);
                        for block in &valid {
                            for txn in &block.transactions {
                                match &txn.details {
                                    TransactionData::ProfileChange(change) => {
                                        service.profiles.insert(
                                            change.profile.player_id.clone(),
                                            change.profile.clone(),
                                        );
                                    }
                                    TransactionData::Entitlement(entitlement) => {
                                        service
                                            .rewards
                                            .entry(txn.player_id.clone())
                                            .or_default()
                                            .entitlements
                                            .push(entitlement.clone());
                                    }
                                    TransactionData::Achievement(achievement) => {
                                        service
                                            .rewards
                                            .entry(txn.player_id.clone())
                                            .or_default()
                                            .achievements
                                            .push(achievement.clone());
                                    }
                                }
                            }
                        }
                        verified_blocks.extend(valid);
                        _quarantined_blocks.extend(quarantined);
                    }
                    if let Ok(claims) = service.storage.load_achievement_claims(id) {
                        if !claims.is_empty() {
                            service.achievement_claims.insert(id.to_string(), claims);
                        }
                    }
                }
                verified_blocks.sort_by_key(|b| b.timestamp.clone());
                let mut seen = std::collections::HashSet::new();
                for block in verified_blocks {
                    if seen.insert(block.block_hash.clone()) {
                        service.ledger.chain.push(block);
                    }
                }
            }
            service
        }

        fn verify_player_blocks(blocks: Vec<Block>) -> (Vec<Block>, Vec<Block>) {
            let mut verified = Vec::new();
            let mut quarantined = Vec::new();
            let mut chain = Blockchain::new();
            for block in blocks {
                chain.chain.push(block.clone());
                if chain.is_valid_chain() {
                    verified.push(block);
                } else {
                    chain.chain.pop();
                    quarantined.push(block);
                }
            }
            (verified, quarantined)
        }

        pub fn create_profile(
            &mut self,
            player_id: &str,
            name: &str,
        ) -> std::io::Result<&PlayerProfile> {
            let profile = PlayerProfile::new(player_id.to_string(), name.to_string());
            self.profiles.insert(player_id.to_string(), profile.clone());
            self.rewards.entry(player_id.to_string()).or_default();
            self.log_change(&profile)?;
            self.profiles
                .get(player_id)
                .ok_or_else(|| std::io::Error::other("profile insertion failed"))
        }

        pub fn get_profile(&self, player_id: &str) -> Option<&PlayerProfile> {
            self.profiles.get(player_id)
        }

        pub fn get_reward_state(&self, player_id: &str) -> Option<&PlayerRewardState> {
            self.rewards.get(player_id)
        }

        pub fn get_achievement_claims(&self, player_id: &str) -> Option<&Vec<AchievementClaim>> {
            self.achievement_claims.get(player_id)
        }

        pub fn get_achievement_claim(
            &self,
            player_id: &str,
            claim_id: &str,
        ) -> Option<&AchievementClaim> {
            self.achievement_claims
                .get(player_id)?
                .iter()
                .find(|claim| claim.claim_id == claim_id)
        }

        pub fn set_vector(&mut self, player_id: &str, vec: BitVec) -> std::io::Result<()> {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                profile.set_vector(vec);
                let cloned = profile.clone();
                self.log_change(&cloned)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ))
            }
        }

        pub fn merge_vector(&mut self, player_id: &str, vec: &BitVec) -> std::io::Result<()> {
            if let Some(profile) = self.profiles.get_mut(player_id) {
                let new_vec = profile.profile_vec.xor(vec);
                profile.set_vector(new_vec);
                let cloned = profile.clone();
                self.log_change(&cloned)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ))
            }
        }

        pub fn award_entitlement(
            &mut self,
            player_id: &str,
            entitlement: &crate::entitlement_registry::EntitlementDefinition,
            quantity: u32,
            expiration_date: Option<String>,
        ) -> std::io::Result<AwardRecord> {
            if self.profiles.contains_key(player_id) {
                let details = crate::blockchain::Entitlement {
                    developer: entitlement.developer.clone(),
                    game: entitlement.game.clone(),
                    entitlement_id: entitlement.entitlement_id.clone(),
                    version: entitlement.version,
                    item_type: entitlement.item_type.clone(),
                    item_id: entitlement.item_id.clone(),
                    quantity,
                    metadata: entitlement.description.clone(),
                    expiration_date,
                };
                self.rewards
                    .entry(player_id.to_string())
                    .or_default()
                    .entitlements
                    .push(details.clone());
                let json = serde_json::to_string(&details).map_err(std::io::Error::other)?;
                let mut hasher = Sha256::new();
                hasher.update(json);
                let hash = hex::encode(hasher.finalize());
                let txn = Transaction {
                    transaction_id: Uuid::new_v4().to_string(),
                    player_id: player_id.to_string(),
                    transaction_type: "entitlement".to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    data_hash: hash.clone(),
                    signature: String::new(),
                    details: TransactionData::Entitlement(details),
                };
                self.ledger.add_block(vec![txn]);
                if let Ok(id) = Uuid::parse_str(player_id) {
                    if let Some(b) = self.ledger.get_latest_block() {
                        let block = b.clone();
                        self.storage.append_block(id, &block)?;
                        let tx = block.transactions.last().ok_or_else(|| {
                            std::io::Error::other("missing entitlement transaction in latest block")
                        })?;
                        return Ok(AwardRecord {
                            player_id: tx.player_id.clone(),
                            transaction_id: tx.transaction_id.clone(),
                            transaction_type: tx.transaction_type.clone(),
                            timestamp: tx.timestamp.clone(),
                            data_hash: tx.data_hash.clone(),
                            block_hash: block.block_hash.clone(),
                            details: tx.details.clone(),
                        });
                    }
                }
                Err(std::io::Error::other("failed to persist entitlement award"))
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ))
            }
        }

        pub fn award_achievement(
            &mut self,
            player_id: &str,
            achievement: &crate::achievement_registry::AchievementDefinition,
        ) -> std::io::Result<AwardRecord> {
            if self.profiles.contains_key(player_id) {
                let details = crate::blockchain::Achievement {
                    developer: achievement.developer().to_string(),
                    game: achievement.game().to_string(),
                    achievement_id: achievement.achievement_id().to_string(),
                    version: achievement.version(),
                    achievement_name: achievement.name().to_string(),
                    criteria: achievement.accomplishment_summary().to_string(),
                    timestamp_earned: Utc::now().to_rfc3339(),
                    metadata: serde_json::to_string(&achievement.award_metadata())
                        .map_err(std::io::Error::other)?,
                };
                self.rewards
                    .entry(player_id.to_string())
                    .or_default()
                    .achievements
                    .push(details.clone());
                let json = serde_json::to_string(&details).map_err(std::io::Error::other)?;
                let mut hasher = Sha256::new();
                hasher.update(json);
                let hash = hex::encode(hasher.finalize());
                let txn = Transaction {
                    transaction_id: Uuid::new_v4().to_string(),
                    player_id: player_id.to_string(),
                    transaction_type: "achievement".to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    data_hash: hash.clone(),
                    signature: String::new(),
                    details: TransactionData::Achievement(details),
                };
                self.ledger.add_block(vec![txn]);
                if let Ok(id) = Uuid::parse_str(player_id) {
                    if let Some(b) = self.ledger.get_latest_block() {
                        let block = b.clone();
                        self.storage.append_block(id, &block)?;
                        let tx = block.transactions.last().ok_or_else(|| {
                            std::io::Error::other("missing achievement transaction in latest block")
                        })?;
                        return Ok(AwardRecord {
                            player_id: tx.player_id.clone(),
                            transaction_id: tx.transaction_id.clone(),
                            transaction_type: tx.transaction_type.clone(),
                            timestamp: tx.timestamp.clone(),
                            data_hash: tx.data_hash.clone(),
                            block_hash: block.block_hash.clone(),
                            details: tx.details.clone(),
                        });
                    }
                }
                Err(std::io::Error::other("failed to persist achievement award"))
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ))
            }
        }

        pub fn submit_achievement_claim(
            &mut self,
            player_id: &str,
            claim: AchievementClaimInput,
        ) -> std::io::Result<AchievementClaim> {
            if !self.profiles.contains_key(player_id) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ));
            }

            let claims = self
                .achievement_claims
                .entry(player_id.to_string())
                .or_default();
            if let Some(existing) = claims
                .iter()
                .find(|existing| existing.claim_id == claim.claim_id)
            {
                return Ok(existing.clone());
            }

            let stored = AchievementClaim {
                developer: claim.developer,
                game: claim.game,
                achievement_id: claim.achievement_id,
                version: claim.version,
                claim_id: claim.claim_id,
                session_id: claim.session_id,
                client_sequence: claim.client_sequence,
                claimed_at: claim.claimed_at,
                evidence: claim.evidence,
                submitted_at: Utc::now().to_rfc3339(),
                status: AchievementClaimStatus::Pending,
                reviewed_at: None,
                reviewer: None,
                review_note: None,
                awarded_transaction_id: None,
                awarded_block_hash: None,
                canonical_envelope: None,
                acknowledgement: None,
            };
            claims.push(stored.clone());
            self.persist_claims(player_id)?;
            Ok(stored)
        }

        /// Applies authoritative definition policy to a canonical embedded-EAB claim.
        ///
        /// The authenticated account binding is supplied separately as `player_id`; the
        /// client-controlled envelope cannot select its own destination account. This method
        /// contains no HTTP or node-transport behavior.
        pub fn acknowledge_canonical_claim(
            &mut self,
            player_id: &str,
            envelope: EabClaimEnvelope,
            definition: Option<&crate::achievement_registry::AchievementDefinition>,
        ) -> std::io::Result<EabClaimAcknowledgement> {
            if !self.profiles.contains_key(player_id) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ));
            }

            if let Some(existing) = self
                .get_achievement_claim(player_id, envelope.claim_id())
                .cloned()
            {
                if existing.canonical_envelope.as_ref() == Some(&envelope) {
                    if let Some(acknowledgement) = existing.acknowledgement {
                        return Ok(acknowledgement);
                    }
                }
                return Ok(Self::claim_acknowledgement(
                    &envelope,
                    EabClaimDisposition::Conflict,
                    EabClaimDecisionCode::ClaimIdPayloadMismatch,
                    Utc::now().to_rfc3339(),
                    None,
                ));
            }

            let first_observed_at = Utc::now().to_rfc3339();
            if let Some((disposition, code)) =
                Self::canonical_claim_policy_decision(&envelope, definition)?
            {
                let acknowledgement = Self::claim_acknowledgement(
                    &envelope,
                    disposition,
                    code,
                    first_observed_at,
                    None,
                );
                self.store_canonical_claim(player_id, envelope, acknowledgement.clone())?;
                return Ok(acknowledgement);
            }

            let definition = definition.expect("validated canonical definition");
            let (code, award) = if let Some(existing) =
                self.existing_achievement_award_reference(player_id, definition)
            {
                (EabClaimDecisionCode::AlreadyAcknowledged, Some(existing))
            } else {
                let created = self.award_achievement(player_id, definition)?;
                (
                    EabClaimDecisionCode::Acknowledged,
                    Some(EabAwardReference {
                        transaction_id: created.transaction_id,
                        block_hash: created.block_hash,
                    }),
                )
            };
            let acknowledgement = Self::claim_acknowledgement(
                &envelope,
                EabClaimDisposition::Acknowledged,
                code,
                first_observed_at,
                award,
            );
            self.store_canonical_claim(player_id, envelope, acknowledgement.clone())?;
            Ok(acknowledgement)
        }

        pub fn get_claim_acknowledgement(
            &self,
            player_id: &str,
            claim_id: &str,
        ) -> Option<&EabClaimAcknowledgement> {
            self.get_achievement_claim(player_id, claim_id)?
                .acknowledgement
                .as_ref()
        }

        fn canonical_claim_policy_decision(
            envelope: &EabClaimEnvelope,
            definition: Option<&crate::achievement_registry::AchievementDefinition>,
        ) -> std::io::Result<Option<(EabClaimDisposition, EabClaimDecisionCode)>> {
            if let Err(error) = envelope.validate() {
                let code = match error {
                    EabClaimEnvelopeError::NotReady(_) => EabClaimDecisionCode::ClaimNotReady,
                    EabClaimEnvelopeError::UnsupportedSchemaVersion(_)
                    | EabClaimEnvelopeError::InvalidRecord(_) => {
                        EabClaimDecisionCode::InvalidEnvelope
                    }
                };
                return Ok(Some((EabClaimDisposition::Rejected, code)));
            }

            let Some(definition) = definition else {
                return Ok(Some((
                    EabClaimDisposition::Conflict,
                    EabClaimDecisionCode::DefinitionNotFound,
                )));
            };
            let record = &envelope.record;
            if record.developer != definition.developer()
                || record.game != definition.game()
                || record.achievement_id != definition.achievement_id()
                || record.version != definition.version()
            {
                return Ok(Some((
                    EabClaimDisposition::Conflict,
                    EabClaimDecisionCode::DefinitionIdentityMismatch,
                )));
            }
            let expected_digest = definition_digest(definition)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
            if record.definition_digest != expected_digest {
                return Ok(Some((
                    EabClaimDisposition::Conflict,
                    EabClaimDecisionCode::DefinitionDigestMismatch,
                )));
            }
            if !definition.allows_claim_review() {
                return Ok(Some((
                    EabClaimDisposition::Rejected,
                    EabClaimDecisionCode::IssuanceModeDisallowsClaim,
                )));
            }
            if definition.is_repeatable() {
                return Ok(Some((
                    EabClaimDisposition::Rejected,
                    EabClaimDecisionCode::RepeatableNotSupported,
                )));
            }
            if definition.accomplishment.requires_evidence && record.evidence.is_none() {
                return Ok(Some((
                    EabClaimDisposition::Rejected,
                    EabClaimDecisionCode::EvidenceRequired,
                )));
            }
            if definition.accomplishment.event_key.as_deref() != Some(record.event_key.as_str()) {
                return Ok(Some((
                    EabClaimDisposition::Rejected,
                    EabClaimDecisionCode::EventMismatch,
                )));
            }
            let threshold = definition.accomplishment.threshold.unwrap_or(1);
            if threshold == 0 || record.event_value < threshold {
                return Ok(Some((
                    EabClaimDisposition::Rejected,
                    EabClaimDecisionCode::ThresholdNotMet,
                )));
            }
            Ok(None)
        }

        fn claim_acknowledgement(
            envelope: &EabClaimEnvelope,
            disposition: EabClaimDisposition,
            code: EabClaimDecisionCode,
            first_observed_at: String,
            award: Option<EabAwardReference>,
        ) -> EabClaimAcknowledgement {
            let record = &envelope.record;
            EabClaimAcknowledgement {
                schema_version: EAB_CLAIM_ACKNOWLEDGEMENT_SCHEMA_VERSION,
                claim_id: record.claim_id.clone(),
                developer: record.developer.clone(),
                game: record.game.clone(),
                achievement_id: record.achievement_id.clone(),
                version: record.version,
                disposition,
                code,
                first_observed_at,
                decided_at: Some(Utc::now().to_rfc3339()),
                award,
            }
        }

        fn store_canonical_claim(
            &mut self,
            player_id: &str,
            envelope: EabClaimEnvelope,
            acknowledgement: EabClaimAcknowledgement,
        ) -> std::io::Result<()> {
            let record = &envelope.record;
            let promoted = acknowledgement.disposition == EabClaimDisposition::Acknowledged;
            let stored = AchievementClaim {
                developer: record.developer.clone(),
                game: record.game.clone(),
                achievement_id: record.achievement_id.clone(),
                version: record.version,
                claim_id: record.claim_id.clone(),
                session_id: record.session_id.clone(),
                client_sequence: record.client_sequence,
                claimed_at: record.earned_at_local.clone(),
                evidence: record.evidence.clone(),
                submitted_at: acknowledgement.first_observed_at.clone(),
                status: if promoted {
                    AchievementClaimStatus::Promoted
                } else {
                    AchievementClaimStatus::Rejected
                },
                reviewed_at: acknowledgement.decided_at.clone(),
                reviewer: Some("eab-authority".to_string()),
                review_note: Some(format!("{:?}", acknowledgement.code)),
                awarded_transaction_id: acknowledgement
                    .award
                    .as_ref()
                    .map(|award| award.transaction_id.clone()),
                awarded_block_hash: acknowledgement
                    .award
                    .as_ref()
                    .map(|award| award.block_hash.clone()),
                canonical_envelope: Some(envelope),
                acknowledgement: Some(acknowledgement),
            };
            self.achievement_claims
                .entry(player_id.to_string())
                .or_default()
                .push(stored);
            self.persist_claims(player_id)
        }

        fn existing_achievement_award_reference(
            &self,
            player_id: &str,
            definition: &crate::achievement_registry::AchievementDefinition,
        ) -> Option<EabAwardReference> {
            self.ledger.chain.iter().rev().find_map(|block| {
                block.transactions.iter().rev().find_map(|transaction| {
                    let TransactionData::Achievement(achievement) = &transaction.details else {
                        return None;
                    };
                    (transaction.player_id == player_id
                        && achievement.developer == definition.developer()
                        && achievement.game == definition.game()
                        && achievement.achievement_id == definition.achievement_id())
                    .then(|| EabAwardReference {
                        transaction_id: transaction.transaction_id.clone(),
                        block_hash: block.block_hash.clone(),
                    })
                })
            })
        }

        pub fn review_achievement_claim(
            &mut self,
            player_id: &str,
            claim_id: &str,
            reviewer: &str,
            action: AchievementClaimReviewAction,
            review_note: Option<String>,
            achievement: Option<&crate::achievement_registry::AchievementDefinition>,
        ) -> std::io::Result<(AchievementClaim, Option<AwardRecord>)> {
            if !self.profiles.contains_key(player_id) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "profile not found",
                ));
            }

            let claim_index = self
                .achievement_claims
                .get(player_id)
                .and_then(|claims| claims.iter().position(|claim| claim.claim_id == claim_id))
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                })?;

            match action {
                AchievementClaimReviewAction::Promote => {
                    let stored_claim = self
                        .achievement_claims
                        .get(player_id)
                        .and_then(|claims| claims.get(claim_index))
                        .cloned()
                        .ok_or_else(|| {
                            std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                        })?;
                    if stored_claim.status == AchievementClaimStatus::Promoted {
                        return Ok((stored_claim, None));
                    }
                    if stored_claim.status == AchievementClaimStatus::Rejected {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "rejected claim cannot be promoted",
                        ));
                    }
                    let achievement = achievement.ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "achievement definition not found",
                        )
                    })?;
                    let reviewed_at = Utc::now().to_rfc3339();
                    {
                        let claims =
                            self.achievement_claims.get_mut(player_id).ok_or_else(|| {
                                std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                            })?;
                        let claim = claims.get_mut(claim_index).ok_or_else(|| {
                            std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                        })?;
                        claim.status = AchievementClaimStatus::Promoted;
                        claim.reviewed_at = Some(reviewed_at);
                        claim.reviewer = Some(reviewer.to_string());
                        claim.review_note = review_note;
                    }
                    self.persist_claims(player_id)?;

                    let award = self.award_achievement(player_id, achievement)?;
                    let claims = self.achievement_claims.get_mut(player_id).ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                    })?;
                    let claim = claims.get_mut(claim_index).ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                    })?;
                    claim.awarded_transaction_id = Some(award.transaction_id.clone());
                    claim.awarded_block_hash = Some(award.block_hash.clone());
                    let updated = claim.clone();
                    self.persist_claims(player_id)?;
                    Ok((updated, Some(award)))
                }
                AchievementClaimReviewAction::Reject => {
                    let claims = self.achievement_claims.get_mut(player_id).ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                    })?;
                    let claim = claims.get_mut(claim_index).ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "claim not found")
                    })?;
                    if claim.status == AchievementClaimStatus::Promoted {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "promoted claim cannot be rejected",
                        ));
                    }
                    claim.status = AchievementClaimStatus::Rejected;
                    claim.reviewed_at = Some(Utc::now().to_rfc3339());
                    claim.reviewer = Some(reviewer.to_string());
                    claim.review_note = review_note;
                    let updated = claim.clone();
                    self.persist_claims(player_id)?;
                    Ok((updated, None))
                }
            }
        }

        fn persist_claims(&self, player_id: &str) -> std::io::Result<()> {
            let id = Uuid::parse_str(player_id).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid player id")
            })?;
            let claims = self
                .achievement_claims
                .get(player_id)
                .cloned()
                .unwrap_or_default();
            self.storage.save_achievement_claims(id, &claims)
        }

        fn log_change(&mut self, profile: &PlayerProfile) -> std::io::Result<()> {
            let json = serde_json::to_string(profile).map_err(std::io::Error::other)?;
            let mut hasher = Sha256::new();
            hasher.update(json);
            let hash = hex::encode(hasher.finalize());
            let txn = Transaction {
                transaction_id: Uuid::new_v4().to_string(),
                player_id: profile.player_id.clone(),
                transaction_type: "profile_change".to_string(),
                timestamp: Utc::now().to_rfc3339(),
                data_hash: hash.clone(),
                signature: String::new(),
                details: TransactionData::ProfileChange(ProfileChange {
                    profile_hash: hash,
                    profile: profile.clone(),
                }),
            };
            self.ledger.add_block(vec![txn]);
            if let Ok(id) = Uuid::parse_str(&profile.player_id) {
                if let Some(b) = self.ledger.get_latest_block() {
                    let block = b.clone();
                    self.storage.append_block(id, &block)?;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_service::*;
    use crate::blockchain::Block;
    use crate::hd::{hamming_distance, BitVec};
    use crate::ledger_storage::{FileTopicLedgerStorage, LedgerStorage};
    use uuid::Uuid;

    #[test]
    fn test_profile_creation_and_update() {
        let dir = "test_player_logs";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Alice")
            .expect("create profile");
        let vec = BitVec::seed("TEST", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone()).expect("set vector");
        let profile = service.get_profile(&pid).expect("missing profile");
        assert_eq!(hamming_distance(&profile.profile_vec, &vec), 0);
        assert_eq!(service.ledger.chain.len(), 3);
        assert!(service.ledger.is_valid_chain());
        assert_eq!(
            service.ledger.chain[1].app_version,
            env!("CARGO_PKG_VERSION")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_award_achievement() {
        let dir = "test_player_logs2";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Bob").expect("create profile");

        let ach = crate::achievement_registry::AchievementDefinition::new(
            "dev", "game", "ach1", 1, "First", "Earned",
        );

        let receipt = service
            .award_achievement(&pid, &ach)
            .expect("award achievement");
        assert_eq!(receipt.player_id, pid);
        assert_eq!(receipt.transaction_type, "achievement");
        assert_eq!(service.ledger.chain.len(), 3);
        if let crate::blockchain::TransactionData::Achievement(a) =
            &service.ledger.chain[2].transactions[0].details
        {
            assert_eq!(a.achievement_id, "ach1");
            assert_eq!(a.version, 1);
            assert_eq!(a.criteria, "Earned");
        } else {
            panic!("expected achievement transaction");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_award_achievement_preserves_modeled_criteria_and_policy() {
        let dir = "test_player_logs2_modeled";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Bob").expect("create profile");

        let ach = crate::achievement_registry::AchievementDefinition::new(
            "dev",
            "game",
            "first-flight",
            1,
            "First Flight",
            "Complete your first successful run",
        )
        .with_category("progression")
        .with_policy(
            crate::achievement_registry::AchievementVisibility::PublicProof,
            crate::achievement_registry::AchievementRepeatability::OncePerPlayer,
            crate::achievement_registry::AchievementIssuanceMode::DirectAwardOrClaimReview,
        )
        .with_accomplishment(crate::achievement_registry::AchievementAccomplishment {
            summary: "Complete one successful run".into(),
            event_key: Some("run_completed".into()),
            threshold: Some(1),
            requires_evidence: false,
        });

        service
            .award_achievement(&pid, &ach)
            .expect("award achievement");
        if let crate::blockchain::TransactionData::Achievement(a) =
            &service.ledger.chain[2].transactions[0].details
        {
            assert_eq!(a.criteria, "Complete one successful run");
            let metadata: crate::achievement_registry::AchievementAwardMetadata =
                serde_json::from_str(&a.metadata).expect("parse metadata");
            assert_eq!(metadata.category, "progression");
            assert_eq!(
                metadata.visibility,
                crate::achievement_registry::AchievementVisibility::PublicProof
            );
            assert_eq!(
                metadata.repeatability,
                crate::achievement_registry::AchievementRepeatability::OncePerPlayer
            );
            assert_eq!(
                metadata.issuance_mode,
                crate::achievement_registry::AchievementIssuanceMode::DirectAwardOrClaimReview
            );
        } else {
            panic!("expected achievement transaction");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_award_entitlement() {
        let dir = "test_player_logs3";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service.create_profile(&pid, "Eve").expect("create profile");

        let ent = crate::entitlement_registry::EntitlementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            entitlement_id: "ent1".into(),
            version: 1,
            item_type: "item".into(),
            item_id: "i1".into(),
            description: "desc".into(),
        };

        let receipt = service
            .award_entitlement(&pid, &ent, 1, None)
            .expect("award entitlement");
        assert_eq!(receipt.player_id, pid);
        assert_eq!(receipt.transaction_type, "entitlement");
        assert_eq!(service.ledger.chain.len(), 3);
        if let crate::blockchain::TransactionData::Entitlement(e) =
            &service.ledger.chain[2].transactions[0].details
        {
            assert_eq!(e.entitlement_id, "ent1");
            assert_eq!(e.version, 1);
            assert!(e.expiration_date.is_none());
        } else {
            panic!("expected entitlement transaction");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_restart_with_persisted_data() {
        let dir = "test_player_logs_restart";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Restart")
            .expect("create profile");
        let vec = BitVec::seed("TEST", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone()).expect("set vector");
        let chain_len = service.ledger.chain.len();
        drop(service);

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), chain_len);
        let profile = service.get_profile(&pid).expect("missing profile");
        assert_eq!(profile.name, "Restart");
        assert_eq!(hamming_distance(&profile.profile_vec, &vec), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_restart_persists_rewards_state() {
        let dir = "test_player_logs_restart_rewards";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Rewards")
            .expect("create profile");

        let ach = crate::achievement_registry::AchievementDefinition::new(
            "dev", "game", "ach2", 2, "Second", "Earned",
        );

        let ent = crate::entitlement_registry::EntitlementDefinition {
            developer: "dev".into(),
            game: "game".into(),
            entitlement_id: "ent2".into(),
            version: 2,
            item_type: "item".into(),
            item_id: "i2".into(),
            description: "desc".into(),
        };

        service
            .award_entitlement(&pid, &ent, 2, None)
            .expect("award entitlement");
        service
            .award_achievement(&pid, &ach)
            .expect("award achievement");
        let chain_len = service.ledger.chain.len();
        drop(service);

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), chain_len);
        let rewards = service.get_reward_state(&pid).expect("missing rewards");
        assert_eq!(rewards.entitlements.len(), 1);
        assert_eq!(rewards.achievements.len(), 1);
        assert_eq!(rewards.entitlements[0].entitlement_id, "ent2");
        assert_eq!(rewards.achievements[0].achievement_id, "ach2");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_rejects_tampered_blocks_on_restart() {
        let dir = "test_player_logs_tampered";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Mallory")
            .expect("create profile");
        let vec = BitVec::seed("TAMPERED", DEFAULT_DIM);
        service.set_vector(&pid, vec.clone()).expect("set vector");
        drop(service);

        let path = format!("{}/{}.log", dir, pid);
        let contents = std::fs::read_to_string(&path).expect("read log");
        let mut blocks: Vec<Block> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse block"))
            .collect();
        assert!(blocks.len() >= 2, "expected at least two blocks");
        blocks[1].previous_block_hash = "tampered".to_string();
        let tampered_contents = blocks
            .into_iter()
            .map(|block| serde_json::to_string(&block).expect("serialize block"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{}\n", tampered_contents)).expect("write log");

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), 2);
        let profile = service.get_profile(&pid).expect("missing profile");
        let default_vec = BitVec::new(DEFAULT_DIM);
        assert_eq!(hamming_distance(&profile.profile_vec, &default_vec), 0);
        assert_ne!(hamming_distance(&profile.profile_vec, &vec), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_rejects_out_of_order_blocks() {
        let dir = "test_player_logs_out_of_order";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "OutOfOrder")
            .expect("create profile");
        let vec1 = BitVec::seed("ORDER1", DEFAULT_DIM);
        let vec2 = BitVec::seed("ORDER2", DEFAULT_DIM);
        service
            .set_vector(&pid, vec1.clone())
            .expect("set vector 1");
        service
            .set_vector(&pid, vec2.clone())
            .expect("set vector 2");
        drop(service);

        let path = format!("{}/{}.log", dir, pid);
        let contents = std::fs::read_to_string(&path).expect("read log");
        let mut blocks: Vec<Block> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse block"))
            .collect();
        assert!(blocks.len() >= 3, "expected at least three blocks");
        blocks.swap(1, 2);
        let reordered_contents = blocks
            .into_iter()
            .map(|block| serde_json::to_string(&block).expect("serialize block"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{}\n", reordered_contents)).expect("write log");

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        assert_eq!(service.ledger.chain.len(), 3);
        let profile = service.get_profile(&pid).expect("missing profile");
        assert_eq!(hamming_distance(&profile.profile_vec, &vec1), 0);
        assert_ne!(hamming_distance(&profile.profile_vec, &vec2), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_submit_achievement_claim_is_idempotent_and_not_reward_mutation() {
        let dir = "test_player_logs_claims";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Claimant")
            .expect("create profile");

        let input = AchievementClaimInput {
            developer: "dev".into(),
            game: "game".into(),
            achievement_id: "ach-claim".into(),
            version: 1,
            claim_id: "claim-1".into(),
            session_id: "session-1".into(),
            client_sequence: 7,
            claimed_at: "2026-03-22T09:00:00Z".into(),
            evidence: Some("offline-run".into()),
        };

        let first = service
            .submit_achievement_claim(&pid, input.clone())
            .expect("submit claim");
        let second = service
            .submit_achievement_claim(&pid, input)
            .expect("submit duplicate claim");

        assert_eq!(first, second);
        let claims = service
            .get_achievement_claims(&pid)
            .expect("missing claims");
        assert_eq!(claims.len(), 1);
        let rewards = service.get_reward_state(&pid).expect("missing rewards");
        assert!(rewards.achievements.is_empty());
        assert_eq!(service.ledger.chain.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_achievement_claims_are_player_scoped() {
        let dir = "test_player_logs_claim_scoped";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let player_one = Uuid::new_v4().to_string();
        let player_two = Uuid::new_v4().to_string();
        service
            .create_profile(&player_one, "One")
            .expect("create profile one");
        service
            .create_profile(&player_two, "Two")
            .expect("create profile two");

        let base_claim = AchievementClaimInput {
            developer: "dev".into(),
            game: "game".into(),
            achievement_id: "ach-shared-id".into(),
            version: 1,
            claim_id: "claim-same".into(),
            session_id: "session-1".into(),
            client_sequence: 1,
            claimed_at: "2026-03-22T09:00:00Z".into(),
            evidence: None,
        };

        service
            .submit_achievement_claim(&player_one, base_claim.clone())
            .expect("submit claim one");
        service
            .submit_achievement_claim(&player_two, base_claim)
            .expect("submit claim two");

        assert_eq!(
            service
                .get_achievement_claims(&player_one)
                .expect("claims one")
                .len(),
            1
        );
        assert_eq!(
            service
                .get_achievement_claims(&player_two)
                .expect("claims two")
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_achievement_claims_persist_across_restart() {
        let dir = "test_player_logs_claim_restart";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "RestartClaims")
            .expect("create profile");

        service
            .submit_achievement_claim(
                &pid,
                AchievementClaimInput {
                    developer: "dev".into(),
                    game: "game".into(),
                    achievement_id: "ach-claim".into(),
                    version: 1,
                    claim_id: "claim-restart".into(),
                    session_id: "session-restart".into(),
                    client_sequence: 9,
                    claimed_at: "2026-03-22T10:00:00Z".into(),
                    evidence: Some("offline".into()),
                },
            )
            .expect("submit claim");
        drop(service);

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        let claims = service
            .get_achievement_claims(&pid)
            .expect("missing claims");
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim_id, "claim-restart");
        assert_eq!(claims[0].status, AchievementClaimStatus::Pending);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[derive(Default)]
    struct FlakyClaimStorage {
        blocks: std::sync::Mutex<std::collections::HashMap<Uuid, Vec<Block>>>,
        claims: std::sync::Mutex<std::collections::HashMap<Uuid, Vec<AchievementClaim>>>,
        save_calls: std::sync::atomic::AtomicUsize,
        fail_on_save_call: usize,
    }

    impl FlakyClaimStorage {
        fn new(fail_on_save_call: usize) -> Self {
            Self {
                fail_on_save_call,
                ..Self::default()
            }
        }
    }

    impl LedgerStorage for FlakyClaimStorage {
        fn append_block(&self, player_id: Uuid, block: &Block) -> std::io::Result<()> {
            self.blocks
                .lock()
                .expect("blocks mutex")
                .entry(player_id)
                .or_default()
                .push(block.clone());
            Ok(())
        }

        fn load_blocks(&self, player_id: Uuid) -> std::io::Result<Vec<Block>> {
            Ok(self
                .blocks
                .lock()
                .expect("blocks mutex")
                .get(&player_id)
                .cloned()
                .unwrap_or_default())
        }

        fn list_player_ids(&self) -> std::io::Result<Vec<Uuid>> {
            Ok(Vec::new())
        }

        fn load_achievement_claims(
            &self,
            player_id: Uuid,
        ) -> std::io::Result<Vec<AchievementClaim>> {
            Ok(self
                .claims
                .lock()
                .expect("claims mutex")
                .get(&player_id)
                .cloned()
                .unwrap_or_default())
        }

        fn save_achievement_claims(
            &self,
            player_id: Uuid,
            claims: &[AchievementClaim],
        ) -> std::io::Result<()> {
            let call = self
                .save_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if call == self.fail_on_save_call {
                return Err(std::io::Error::other("simulated claim persistence failure"));
            }
            self.claims
                .lock()
                .expect("claims mutex")
                .insert(player_id, claims.to_vec());
            Ok(())
        }
    }

    #[test]
    fn test_review_achievement_claim_does_not_double_award_if_final_claim_persist_fails() {
        let storage = FlakyClaimStorage::new(3);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Reviewer")
            .expect("create profile");

        service
            .submit_achievement_claim(
                &pid,
                AchievementClaimInput {
                    developer: "dev".into(),
                    game: "game".into(),
                    achievement_id: "ach-claim".into(),
                    version: 1,
                    claim_id: "claim-review".into(),
                    session_id: "session-review".into(),
                    client_sequence: 1,
                    claimed_at: "2026-03-22T11:00:00Z".into(),
                    evidence: Some("offline".into()),
                },
            )
            .expect("submit claim");

        let definition = crate::achievement_registry::AchievementDefinition::new(
            "dev",
            "game",
            "ach-claim",
            1,
            "Claimed",
            "Claimed and validated",
        );

        let error = service
            .review_achievement_claim(
                &pid,
                "claim-review",
                "dev",
                AchievementClaimReviewAction::Promote,
                Some("consortium validated".into()),
                Some(&definition),
            )
            .expect_err("final claim persistence should fail");
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(
            service
                .get_reward_state(&pid)
                .expect("rewards")
                .achievements
                .len(),
            1
        );
        assert_eq!(
            service
                .get_achievement_claim(&pid, "claim-review")
                .expect("claim")
                .status,
            AchievementClaimStatus::Promoted
        );

        let (claim, award) = service
            .review_achievement_claim(
                &pid,
                "claim-review",
                "dev",
                AchievementClaimReviewAction::Promote,
                Some("retry should be idempotent".into()),
                Some(&definition),
            )
            .expect("retry promote claim");

        assert_eq!(claim.status, AchievementClaimStatus::Promoted);
        assert!(award.is_none());
        assert_eq!(
            service
                .get_reward_state(&pid)
                .expect("rewards")
                .achievements
                .len(),
            1
        );
    }

    #[test]
    fn test_review_achievement_claim_promotes_and_persists_status() {
        let dir = "test_player_logs_claim_review";
        let storage = FileTopicLedgerStorage::new(dir);
        let mut service = PlayerProfileService::new(Box::new(storage));
        let pid = Uuid::new_v4().to_string();
        service
            .create_profile(&pid, "Reviewer")
            .expect("create profile");

        service
            .submit_achievement_claim(
                &pid,
                AchievementClaimInput {
                    developer: "dev".into(),
                    game: "game".into(),
                    achievement_id: "ach-claim".into(),
                    version: 1,
                    claim_id: "claim-review".into(),
                    session_id: "session-review".into(),
                    client_sequence: 1,
                    claimed_at: "2026-03-22T11:00:00Z".into(),
                    evidence: Some("offline".into()),
                },
            )
            .expect("submit claim");

        let definition = crate::achievement_registry::AchievementDefinition::new(
            "dev",
            "game",
            "ach-claim",
            1,
            "Claimed",
            "Claimed and validated",
        );

        let (claim, award) = service
            .review_achievement_claim(
                &pid,
                "claim-review",
                "dev",
                AchievementClaimReviewAction::Promote,
                Some("consortium validated".into()),
                Some(&definition),
            )
            .expect("promote claim");

        assert_eq!(claim.status, AchievementClaimStatus::Promoted);
        assert_eq!(claim.reviewer.as_deref(), Some("dev"));
        assert_eq!(claim.review_note.as_deref(), Some("consortium validated"));
        assert!(award.is_some());
        assert_eq!(
            service
                .get_reward_state(&pid)
                .expect("rewards")
                .achievements
                .len(),
            1
        );
        drop(service);

        let storage = FileTopicLedgerStorage::new(dir);
        let service = PlayerProfileService::new(Box::new(storage));
        let claims = service.get_achievement_claims(&pid).expect("claims");
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].status, AchievementClaimStatus::Promoted);
        assert!(claims[0].awarded_transaction_id.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
