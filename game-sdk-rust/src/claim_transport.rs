use std::error::Error;
use std::net::SocketAddr;

use eab_quic_client::{PinnedQuicClient, QuicClientError};
use eab_wire::{ClaimStatusRequest, ProtocolErrorCode, SecureMessage, SubmitClaimRequest};
use uuid::Uuid;

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

#[derive(Debug, thiserror::Error)]
pub enum QuicClaimTransportError {
    #[error("invalid QUIC claim transport configuration: {0}")]
    Configuration(String),
    #[error("offline EAB record cannot be submitted: {0}")]
    OfflineRecord(String),
    #[error("claim submission outcome is unknown; reconcile the same claim_id: {0}")]
    OutcomeUnknown(String),
    #[error(transparent)]
    Quic(#[from] QuicClientError),
    #[error("authority rejected the request ({code:?}, retryable={retryable}): {detail}")]
    Authority {
        code: ProtocolErrorCode,
        retryable: bool,
        detail: String,
    },
    #[error("authority returned an unexpected secure message")]
    UnexpectedResponse,
}

/// Static-endpoint QUIC implementation of [`EabClaimTransport`].
///
/// It authenticates the authority with an exact DER certificate fingerprint
/// and carries the player session only inside TLS 1.3. It deliberately has no
/// `player_id` field: the authority resolves the destination account from the
/// session token.
pub struct QuicEabClaimTransport {
    client: PinnedQuicClient,
    session_token: String,
}

impl QuicEabClaimTransport {
    pub fn new(
        endpoint: SocketAddr,
        authority_certificate_fingerprint: [u8; 32],
        session_token: impl Into<String>,
    ) -> Result<Self, QuicClaimTransportError> {
        let session_token = session_token.into();
        if session_token.is_empty() {
            return Err(QuicClaimTransportError::Configuration(
                "player session token must be non-empty".into(),
            ));
        }
        Ok(Self {
            client: PinnedQuicClient::new(endpoint, authority_certificate_fingerprint)?,
            session_token,
        })
    }

    pub fn endpoint(&self) -> SocketAddr {
        self.client.target()
    }

    fn request_id() -> [u8; 16] {
        *Uuid::new_v4().as_bytes()
    }

    fn map_response(response: SecureMessage) -> Result<SecureMessage, QuicClaimTransportError> {
        match response {
            SecureMessage::ProtocolErrorResponse(error) => {
                Err(QuicClaimTransportError::Authority {
                    code: error.code,
                    retryable: error.retryable,
                    detail: error.detail,
                })
            }
            response => Ok(response),
        }
    }
}

impl EabClaimTransport for QuicEabClaimTransport {
    type Error = QuicClaimTransportError;

    fn submit_claim(
        &self,
        record: &OfflineAchievementRecord,
    ) -> Result<EabClaimAcknowledgement, Self::Error> {
        let expected_claim_id = record.claim_id.clone();
        let envelope = EabClaimEnvelope::try_from(record)
            .map_err(|error| QuicClaimTransportError::OfflineRecord(error.to_string()))?;
        let response = self
            .client
            .round_trip(SecureMessage::SubmitClaimRequest(SubmitClaimRequest {
                request_id: Self::request_id(),
                session_token: self.session_token.clone(),
                envelope,
            }))
            .map_err(|error| match error {
                QuicClientError::Timeout | QuicClientError::Transport(_) => {
                    QuicClaimTransportError::OutcomeUnknown(error.to_string())
                }
                other => QuicClaimTransportError::Quic(other),
            })?;
        match Self::map_response(response)? {
            SecureMessage::SubmitClaimResponse(response)
                if response.acknowledgement.claim_id == expected_claim_id =>
            {
                Ok(response.acknowledgement)
            }
            _ => Err(QuicClaimTransportError::UnexpectedResponse),
        }
    }

    fn claim_status(&self, claim_id: &str) -> Result<Option<EabClaimAcknowledgement>, Self::Error> {
        let response =
            self.client
                .round_trip(SecureMessage::ClaimStatusRequest(ClaimStatusRequest {
                    request_id: Self::request_id(),
                    session_token: self.session_token.clone(),
                    claim_id: claim_id.to_string(),
                }))?;
        match Self::map_response(response)? {
            SecureMessage::ClaimStatusResponse(response) if response.claim_id == claim_id => {
                Ok(response.acknowledgement)
            }
            _ => Err(QuicClaimTransportError::UnexpectedResponse),
        }
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
