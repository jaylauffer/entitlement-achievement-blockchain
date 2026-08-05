use std::error::Error;

use crate::{
    EabClaimAcknowledgement, EabClaimEnvelope, EabClaimEnvelopeError, EabClient,
    OfflineAchievementRecord, SdkError,
};

/// Transport boundary for continuing an immutable offline EAB record online.
///
/// Implementations own their authenticated player binding and endpoint selection. The
/// caller supplies only the offline record, so bearer tokens, multicast discovery, and
/// wire-specific details do not leak into `eab-core` or the game achievement evaluator.
pub trait EabClaimTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    /// Idempotently submits the record using its existing `claim_id`.
    fn submit_claim(
        &self,
        record: &OfflineAchievementRecord,
    ) -> Result<EabClaimAcknowledgement, Self::Error>;

    /// Returns the known online state for a claim, or `None` if the authority has not seen it.
    fn claim_status(&self, claim_id: &str) -> Result<Option<EabClaimAcknowledgement>, Self::Error>;
}

/// Compatibility implementation of [`EabClaimTransport`] over the current HTTP API.
///
/// This adapter owns the player id and session token. It intentionally does not implement
/// discovery; a future loadngo adapter can use multicast discovery followed by authenticated
/// unicast while preserving this same trait contract.
pub struct HttpEabClaimTransport {
    client: EabClient,
    player_id: String,
    player_token: String,
}

impl HttpEabClaimTransport {
    pub fn new(
        client: EabClient,
        player_id: impl Into<String>,
        player_token: impl Into<String>,
    ) -> Self {
        Self {
            client,
            player_id: player_id.into(),
            player_token: player_token.into(),
        }
    }

    pub fn player_id(&self) -> &str {
        &self.player_id
    }
}

impl EabClaimTransport for HttpEabClaimTransport {
    type Error = SdkError;

    fn submit_claim(
        &self,
        record: &OfflineAchievementRecord,
    ) -> Result<EabClaimAcknowledgement, Self::Error> {
        let envelope = EabClaimEnvelope::try_from(record).map_err(|error| match error {
            EabClaimEnvelopeError::NotReady(readiness) => {
                SdkError::OfflineClaimNotReady(format!("{readiness:?}"))
            }
            other => SdkError::OfflineRecord(other.to_string()),
        })?;
        self.client.submit_canonical_achievement_claim(
            &self.player_id,
            &self.player_token,
            &envelope,
        )
    }

    fn claim_status(&self, claim_id: &str) -> Result<Option<EabClaimAcknowledgement>, Self::Error> {
        self.client
            .get_claim_acknowledgement(&self.player_id, &self.player_token, claim_id)
    }
}

impl EabClient {
    /// Creates an authenticated HTTP claim transport for one EAB player session.
    pub fn claim_transport(
        &self,
        player_id: impl Into<String>,
        player_token: impl Into<String>,
    ) -> HttpEabClaimTransport {
        HttpEabClaimTransport::new(self.clone(), player_id, player_token)
    }
}
