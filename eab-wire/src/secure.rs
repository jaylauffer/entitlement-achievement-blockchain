use eab_core::{
    EabAwardReference, EabClaimAcknowledgement, EabClaimDecisionCode, EabClaimDisposition,
    EabClaimEnvelope, OfflineAchievementRecord, OfflineClaimReadiness,
};
use minicbor::data::Type;
use minicbor::{Decoder, Encoder};

use crate::WireError;

pub const SECURE_MAGIC: [u8; 4] = *b"EABS";
pub const SECURE_WIRE_VERSION: u16 = 2;
pub const SECURE_HEADER_LEN: usize = 12;
pub const MAX_SECURE_FRAME_LEN: usize = 64 * 1024;
pub const MAX_SESSION_TOKEN_LEN: usize = 2 * 1024;

const MAX_IDENTIFIER_LEN: usize = 256;
const MAX_TIMESTAMP_LEN: usize = 64;
const MAX_DIGEST_LEN: usize = 128;
const MAX_EVIDENCE_LEN: usize = 32 * 1024;
const MAX_ERROR_DETAIL_LEN: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum SecureMessageKind {
    SubmitClaimRequest = 1,
    SubmitClaimResponse = 2,
    ClaimStatusRequest = 3,
    ClaimStatusResponse = 4,
    ProtocolErrorResponse = 5,
}

impl TryFrom<u16> for SecureMessageKind {
    type Error = WireError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::SubmitClaimRequest),
            2 => Ok(Self::SubmitClaimResponse),
            3 => Ok(Self::ClaimStatusRequest),
            4 => Ok(Self::ClaimStatusResponse),
            5 => Ok(Self::ProtocolErrorResponse),
            other => Err(WireError::UnknownMessageKind(other)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitClaimRequest {
    pub request_id: [u8; 16],
    /// Transitional bearer credential carried only inside authenticated QUIC.
    /// The server derives the destination player from this token.
    pub session_token: String,
    pub envelope: EabClaimEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitClaimResponse {
    pub request_id: [u8; 16],
    pub acknowledgement: EabClaimAcknowledgement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimStatusRequest {
    pub request_id: [u8; 16],
    pub session_token: String,
    pub claim_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimStatusResponse {
    pub request_id: [u8; 16],
    pub claim_id: String,
    pub acknowledgement: Option<EabClaimAcknowledgement>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ProtocolErrorCode {
    InvalidRequest = 1,
    AuthenticationFailed = 2,
    UnsupportedMessage = 3,
    Internal = 4,
}

impl TryFrom<u16> for ProtocolErrorCode {
    type Error = WireError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::InvalidRequest),
            2 => Ok(Self::AuthenticationFailed),
            3 => Ok(Self::UnsupportedMessage),
            4 => Ok(Self::Internal),
            _ => Err(WireError::InvalidField("protocol error code")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolErrorResponse {
    pub request_id: [u8; 16],
    pub code: ProtocolErrorCode,
    pub retryable: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
// Secure messages are short-lived request/response values. Keeping their
// public variants direct avoids an extra allocation around already-owned
// strings and the canonical envelope.
#[allow(clippy::large_enum_variant)]
pub enum SecureMessage {
    SubmitClaimRequest(SubmitClaimRequest),
    SubmitClaimResponse(SubmitClaimResponse),
    ClaimStatusRequest(ClaimStatusRequest),
    ClaimStatusResponse(ClaimStatusResponse),
    ProtocolErrorResponse(ProtocolErrorResponse),
}

impl SecureMessage {
    pub fn kind(&self) -> SecureMessageKind {
        match self {
            Self::SubmitClaimRequest(_) => SecureMessageKind::SubmitClaimRequest,
            Self::SubmitClaimResponse(_) => SecureMessageKind::SubmitClaimResponse,
            Self::ClaimStatusRequest(_) => SecureMessageKind::ClaimStatusRequest,
            Self::ClaimStatusResponse(_) => SecureMessageKind::ClaimStatusResponse,
            Self::ProtocolErrorResponse(_) => SecureMessageKind::ProtocolErrorResponse,
        }
    }

    pub fn request_id(&self) -> [u8; 16] {
        match self {
            Self::SubmitClaimRequest(value) => value.request_id,
            Self::SubmitClaimResponse(value) => value.request_id,
            Self::ClaimStatusRequest(value) => value.request_id,
            Self::ClaimStatusResponse(value) => value.request_id,
            Self::ProtocolErrorResponse(value) => value.request_id,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.validate()?;
        let mut payload = Vec::new();
        let mut encoder = Encoder::new(&mut payload);
        match self {
            Self::SubmitClaimRequest(value) => encode_submit_request(&mut encoder, value)?,
            Self::SubmitClaimResponse(value) => encode_submit_response(&mut encoder, value)?,
            Self::ClaimStatusRequest(value) => encode_status_request(&mut encoder, value)?,
            Self::ClaimStatusResponse(value) => encode_status_response(&mut encoder, value)?,
            Self::ProtocolErrorResponse(value) => encode_protocol_error(&mut encoder, value)?,
        }
        if SECURE_HEADER_LEN + payload.len() > MAX_SECURE_FRAME_LEN {
            return Err(WireError::FrameTooLarge {
                actual: SECURE_HEADER_LEN + payload.len(),
                maximum: MAX_SECURE_FRAME_LEN,
            });
        }
        let payload_len = u32::try_from(payload.len()).map_err(|_| WireError::FrameTooLarge {
            actual: SECURE_HEADER_LEN + payload.len(),
            maximum: MAX_SECURE_FRAME_LEN,
        })?;
        let mut frame = Vec::with_capacity(SECURE_HEADER_LEN + payload.len());
        frame.extend_from_slice(&SECURE_MAGIC);
        frame.extend_from_slice(&SECURE_WIRE_VERSION.to_be_bytes());
        frame.extend_from_slice(&(self.kind() as u16).to_be_bytes());
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    pub fn decode(frame: &[u8]) -> Result<Self, WireError> {
        if frame.len() > MAX_SECURE_FRAME_LEN {
            return Err(WireError::FrameTooLarge {
                actual: frame.len(),
                maximum: MAX_SECURE_FRAME_LEN,
            });
        }
        if frame.len() < SECURE_HEADER_LEN {
            return Err(WireError::TruncatedHeader {
                actual: frame.len(),
                required: SECURE_HEADER_LEN,
            });
        }
        if frame[..4] != SECURE_MAGIC {
            return Err(WireError::InvalidMagic);
        }
        let version = u16::from_be_bytes([frame[4], frame[5]]);
        if version != SECURE_WIRE_VERSION {
            return Err(WireError::UnsupportedWireVersion(version));
        }
        let kind = SecureMessageKind::try_from(u16::from_be_bytes([frame[6], frame[7]]))?;
        let declared = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]) as usize;
        let actual = frame.len() - SECURE_HEADER_LEN;
        if declared != actual {
            return Err(WireError::LengthMismatch { declared, actual });
        }
        let payload = &frame[SECURE_HEADER_LEN..];
        let mut decoder = Decoder::new(payload);
        let message = match kind {
            SecureMessageKind::SubmitClaimRequest => {
                Self::SubmitClaimRequest(decode_submit_request(&mut decoder)?)
            }
            SecureMessageKind::SubmitClaimResponse => {
                Self::SubmitClaimResponse(decode_submit_response(&mut decoder)?)
            }
            SecureMessageKind::ClaimStatusRequest => {
                Self::ClaimStatusRequest(decode_status_request(&mut decoder)?)
            }
            SecureMessageKind::ClaimStatusResponse => {
                Self::ClaimStatusResponse(decode_status_response(&mut decoder)?)
            }
            SecureMessageKind::ProtocolErrorResponse => {
                Self::ProtocolErrorResponse(decode_protocol_error(&mut decoder)?)
            }
        };
        if decoder.position() != payload.len() {
            return Err(WireError::TrailingPayloadBytes(
                payload.len() - decoder.position(),
            ));
        }
        message.validate()?;
        Ok(message)
    }

    pub fn validate(&self) -> Result<(), WireError> {
        validate_request_id(&self.request_id())?;
        match self {
            Self::SubmitClaimRequest(value) => {
                validate_text(&value.session_token, MAX_SESSION_TOKEN_LEN, "session_token")?;
                validate_envelope(&value.envelope)
            }
            Self::SubmitClaimResponse(value) => validate_ack(&value.acknowledgement),
            Self::ClaimStatusRequest(value) => {
                validate_text(&value.session_token, MAX_SESSION_TOKEN_LEN, "session_token")?;
                validate_text(&value.claim_id, MAX_IDENTIFIER_LEN, "claim_id")
            }
            Self::ClaimStatusResponse(value) => {
                validate_text(&value.claim_id, MAX_IDENTIFIER_LEN, "claim_id")?;
                if let Some(ack) = &value.acknowledgement {
                    validate_ack(ack)?;
                    if ack.claim_id != value.claim_id {
                        return Err(WireError::InvalidField("status claim_id mismatch"));
                    }
                }
                Ok(())
            }
            Self::ProtocolErrorResponse(value) => {
                validate_text(&value.detail, MAX_ERROR_DETAIL_LEN, "error detail")
            }
        }
    }
}

fn encode_submit_request(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &SubmitClaimRequest,
) -> Result<(), WireError> {
    cbor(encoder.array(3))?;
    cbor(encoder.bytes(&value.request_id))?;
    cbor(encoder.str(&value.session_token))?;
    encode_envelope(encoder, &value.envelope)
}

fn encode_submit_response(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &SubmitClaimResponse,
) -> Result<(), WireError> {
    cbor(encoder.array(2))?;
    cbor(encoder.bytes(&value.request_id))?;
    encode_ack(encoder, &value.acknowledgement)
}

fn encode_status_request(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &ClaimStatusRequest,
) -> Result<(), WireError> {
    cbor(encoder.array(3))?;
    cbor(encoder.bytes(&value.request_id))?;
    cbor(encoder.str(&value.session_token))?;
    cbor(encoder.str(&value.claim_id))?;
    Ok(())
}

fn encode_status_response(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &ClaimStatusResponse,
) -> Result<(), WireError> {
    cbor(encoder.array(3))?;
    cbor(encoder.bytes(&value.request_id))?;
    cbor(encoder.str(&value.claim_id))?;
    match &value.acknowledgement {
        Some(ack) => encode_ack(encoder, ack)?,
        None => {
            cbor(encoder.null())?;
        }
    }
    Ok(())
}

fn encode_protocol_error(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &ProtocolErrorResponse,
) -> Result<(), WireError> {
    cbor(encoder.array(4))?;
    cbor(encoder.bytes(&value.request_id))?;
    cbor(encoder.u16(value.code as u16))?;
    cbor(encoder.bool(value.retryable))?;
    cbor(encoder.str(&value.detail))?;
    Ok(())
}

fn encode_envelope(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &EabClaimEnvelope,
) -> Result<(), WireError> {
    cbor(encoder.array(2))?;
    cbor(encoder.u32(value.schema_version))?;
    encode_record(encoder, &value.record)
}

fn encode_record(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &OfflineAchievementRecord,
) -> Result<(), WireError> {
    cbor(encoder.array(21))?;
    cbor(encoder.u32(value.schema_version))?;
    cbor(encoder.str(&value.local_award_id))?;
    cbor(encoder.str(&value.claim_id))?;
    cbor(encoder.str(&value.developer))?;
    cbor(encoder.str(&value.game))?;
    cbor(encoder.str(&value.achievement_id))?;
    cbor(encoder.u32(value.version))?;
    cbor(encoder.str(&value.definition_digest))?;
    cbor(encoder.str(&value.local_player_id))?;
    cbor(encoder.str(&value.save_id))?;
    cbor(encoder.str(&value.installation_id))?;
    cbor(encoder.str(&value.session_id))?;
    cbor(encoder.u64(value.client_sequence))?;
    cbor(encoder.str(&value.earned_at_local))?;
    cbor(encoder.str(&value.recorded_at_local))?;
    cbor(encoder.str(&value.game_build))?;
    cbor(encoder.str(&value.event_key))?;
    cbor(encoder.u64(value.event_value))?;
    match &value.evidence {
        Some(evidence) => {
            cbor(encoder.str(evidence))?;
        }
        None => {
            cbor(encoder.null())?;
        }
    }
    cbor(encoder.u8(readiness_to_u8(&value.claim_readiness)))?;
    cbor(encoder.str(&value.local_record_hash))?;
    Ok(())
}

fn encode_ack(
    encoder: &mut Encoder<&mut Vec<u8>>,
    value: &EabClaimAcknowledgement,
) -> Result<(), WireError> {
    cbor(encoder.array(11))?;
    cbor(encoder.u32(value.schema_version))?;
    cbor(encoder.str(&value.claim_id))?;
    cbor(encoder.str(&value.developer))?;
    cbor(encoder.str(&value.game))?;
    cbor(encoder.str(&value.achievement_id))?;
    cbor(encoder.u32(value.version))?;
    cbor(encoder.u8(disposition_to_u8(value.disposition)))?;
    cbor(encoder.u8(decision_to_u8(value.code)))?;
    cbor(encoder.str(&value.first_observed_at))?;
    match &value.decided_at {
        Some(timestamp) => {
            cbor(encoder.str(timestamp))?;
        }
        None => {
            cbor(encoder.null())?;
        }
    }
    match &value.award {
        Some(award) => {
            cbor(encoder.array(2))?;
            cbor(encoder.str(&award.transaction_id))?;
            cbor(encoder.str(&award.block_hash))?;
        }
        None => {
            cbor(encoder.null())?;
        }
    }
    Ok(())
}

fn decode_submit_request(decoder: &mut Decoder<'_>) -> Result<SubmitClaimRequest, WireError> {
    expect_array(decoder, "submit claim request", 3)?;
    Ok(SubmitClaimRequest {
        request_id: decode_fixed_bytes(decoder, "request_id")?,
        session_token: decode_text(decoder, MAX_SESSION_TOKEN_LEN, "session_token")?,
        envelope: decode_envelope(decoder)?,
    })
}

fn decode_submit_response(decoder: &mut Decoder<'_>) -> Result<SubmitClaimResponse, WireError> {
    expect_array(decoder, "submit claim response", 2)?;
    Ok(SubmitClaimResponse {
        request_id: decode_fixed_bytes(decoder, "request_id")?,
        acknowledgement: decode_ack(decoder)?,
    })
}

fn decode_status_request(decoder: &mut Decoder<'_>) -> Result<ClaimStatusRequest, WireError> {
    expect_array(decoder, "claim status request", 3)?;
    Ok(ClaimStatusRequest {
        request_id: decode_fixed_bytes(decoder, "request_id")?,
        session_token: decode_text(decoder, MAX_SESSION_TOKEN_LEN, "session_token")?,
        claim_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "claim_id")?,
    })
}

fn decode_status_response(decoder: &mut Decoder<'_>) -> Result<ClaimStatusResponse, WireError> {
    expect_array(decoder, "claim status response", 3)?;
    let request_id = decode_fixed_bytes(decoder, "request_id")?;
    let claim_id = decode_text(decoder, MAX_IDENTIFIER_LEN, "claim_id")?;
    let acknowledgement = if datatype(decoder)? == Type::Null {
        decode_null(decoder)?;
        None
    } else {
        Some(decode_ack(decoder)?)
    };
    Ok(ClaimStatusResponse {
        request_id,
        claim_id,
        acknowledgement,
    })
}

fn decode_protocol_error(decoder: &mut Decoder<'_>) -> Result<ProtocolErrorResponse, WireError> {
    expect_array(decoder, "protocol error response", 4)?;
    Ok(ProtocolErrorResponse {
        request_id: decode_fixed_bytes(decoder, "request_id")?,
        code: ProtocolErrorCode::try_from(decode_u16(decoder)?)?,
        retryable: decoder
            .bool()
            .map_err(|error| WireError::CborDecode(error.to_string()))?,
        detail: decode_text(decoder, MAX_ERROR_DETAIL_LEN, "error detail")?,
    })
}

fn decode_envelope(decoder: &mut Decoder<'_>) -> Result<EabClaimEnvelope, WireError> {
    expect_array(decoder, "claim envelope", 2)?;
    Ok(EabClaimEnvelope {
        schema_version: decode_u32(decoder)?,
        record: decode_record(decoder)?,
    })
}

fn decode_record(decoder: &mut Decoder<'_>) -> Result<OfflineAchievementRecord, WireError> {
    expect_array(decoder, "offline achievement record", 21)?;
    Ok(OfflineAchievementRecord {
        schema_version: decode_u32(decoder)?,
        local_award_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "local_award_id")?,
        claim_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "claim_id")?,
        developer: decode_text(decoder, MAX_IDENTIFIER_LEN, "developer")?,
        game: decode_text(decoder, MAX_IDENTIFIER_LEN, "game")?,
        achievement_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "achievement_id")?,
        version: decode_u32(decoder)?,
        definition_digest: decode_text(decoder, MAX_DIGEST_LEN, "definition_digest")?,
        local_player_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "local_player_id")?,
        save_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "save_id")?,
        installation_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "installation_id")?,
        session_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "session_id")?,
        client_sequence: decode_u64(decoder)?,
        earned_at_local: decode_text(decoder, MAX_TIMESTAMP_LEN, "earned_at_local")?,
        recorded_at_local: decode_text(decoder, MAX_TIMESTAMP_LEN, "recorded_at_local")?,
        game_build: decode_text(decoder, MAX_IDENTIFIER_LEN, "game_build")?,
        event_key: decode_text(decoder, MAX_IDENTIFIER_LEN, "event_key")?,
        event_value: decode_u64(decoder)?,
        evidence: decode_optional_text(decoder, MAX_EVIDENCE_LEN, "evidence")?,
        claim_readiness: readiness_from_u8(decode_u8(decoder)?)?,
        local_record_hash: decode_text(decoder, MAX_DIGEST_LEN, "local_record_hash")?,
    })
}

fn decode_ack(decoder: &mut Decoder<'_>) -> Result<EabClaimAcknowledgement, WireError> {
    expect_array(decoder, "claim acknowledgement", 11)?;
    let schema_version = decode_u32(decoder)?;
    let claim_id = decode_text(decoder, MAX_IDENTIFIER_LEN, "claim_id")?;
    let developer = decode_text(decoder, MAX_IDENTIFIER_LEN, "developer")?;
    let game = decode_text(decoder, MAX_IDENTIFIER_LEN, "game")?;
    let achievement_id = decode_text(decoder, MAX_IDENTIFIER_LEN, "achievement_id")?;
    let version = decode_u32(decoder)?;
    let disposition = disposition_from_u8(decode_u8(decoder)?)?;
    let code = decision_from_u8(decode_u8(decoder)?)?;
    let first_observed_at = decode_text(decoder, MAX_TIMESTAMP_LEN, "first_observed_at")?;
    let decided_at = decode_optional_text(decoder, MAX_TIMESTAMP_LEN, "decided_at")?;
    let award = if datatype(decoder)? == Type::Null {
        decode_null(decoder)?;
        None
    } else {
        expect_array(decoder, "award reference", 2)?;
        Some(EabAwardReference {
            transaction_id: decode_text(decoder, MAX_IDENTIFIER_LEN, "transaction_id")?,
            block_hash: decode_text(decoder, MAX_DIGEST_LEN, "block_hash")?,
        })
    };
    Ok(EabClaimAcknowledgement {
        schema_version,
        claim_id,
        developer,
        game,
        achievement_id,
        version,
        disposition,
        code,
        first_observed_at,
        decided_at,
        award,
    })
}

fn validate_envelope(envelope: &EabClaimEnvelope) -> Result<(), WireError> {
    let record = &envelope.record;
    validate_text(&record.local_award_id, MAX_IDENTIFIER_LEN, "local_award_id")?;
    validate_text(&record.claim_id, MAX_IDENTIFIER_LEN, "claim_id")?;
    validate_text(&record.developer, MAX_IDENTIFIER_LEN, "developer")?;
    validate_text(&record.game, MAX_IDENTIFIER_LEN, "game")?;
    validate_text(&record.achievement_id, MAX_IDENTIFIER_LEN, "achievement_id")?;
    validate_text(
        &record.definition_digest,
        MAX_DIGEST_LEN,
        "definition_digest",
    )?;
    validate_text(
        &record.local_player_id,
        MAX_IDENTIFIER_LEN,
        "local_player_id",
    )?;
    validate_text(&record.save_id, MAX_IDENTIFIER_LEN, "save_id")?;
    validate_text(
        &record.installation_id,
        MAX_IDENTIFIER_LEN,
        "installation_id",
    )?;
    validate_text(&record.session_id, MAX_IDENTIFIER_LEN, "session_id")?;
    validate_text(
        &record.earned_at_local,
        MAX_TIMESTAMP_LEN,
        "earned_at_local",
    )?;
    validate_text(
        &record.recorded_at_local,
        MAX_TIMESTAMP_LEN,
        "recorded_at_local",
    )?;
    validate_text(&record.game_build, MAX_IDENTIFIER_LEN, "game_build")?;
    validate_text(&record.event_key, MAX_IDENTIFIER_LEN, "event_key")?;
    if let Some(evidence) = &record.evidence {
        validate_text_allow_controls(evidence, MAX_EVIDENCE_LEN, "evidence")?;
    }
    validate_text(
        &record.local_record_hash,
        MAX_DIGEST_LEN,
        "local_record_hash",
    )
}

fn validate_ack(ack: &EabClaimAcknowledgement) -> Result<(), WireError> {
    validate_text(&ack.claim_id, MAX_IDENTIFIER_LEN, "claim_id")?;
    validate_text(&ack.developer, MAX_IDENTIFIER_LEN, "developer")?;
    validate_text(&ack.game, MAX_IDENTIFIER_LEN, "game")?;
    validate_text(&ack.achievement_id, MAX_IDENTIFIER_LEN, "achievement_id")?;
    validate_text(
        &ack.first_observed_at,
        MAX_TIMESTAMP_LEN,
        "first_observed_at",
    )?;
    if let Some(timestamp) = &ack.decided_at {
        validate_text(timestamp, MAX_TIMESTAMP_LEN, "decided_at")?;
    }
    if let Some(award) = &ack.award {
        validate_text(&award.transaction_id, MAX_IDENTIFIER_LEN, "transaction_id")?;
        validate_text(&award.block_hash, MAX_DIGEST_LEN, "block_hash")?;
    }
    Ok(())
}

fn readiness_to_u8(value: &OfflineClaimReadiness) -> u8 {
    match value {
        OfflineClaimReadiness::Ready => 1,
        OfflineClaimReadiness::NotAllowedByIssuancePolicy => 2,
        OfflineClaimReadiness::MissingRequiredEvidence => 3,
    }
}

fn readiness_from_u8(value: u8) -> Result<OfflineClaimReadiness, WireError> {
    match value {
        1 => Ok(OfflineClaimReadiness::Ready),
        2 => Ok(OfflineClaimReadiness::NotAllowedByIssuancePolicy),
        3 => Ok(OfflineClaimReadiness::MissingRequiredEvidence),
        _ => Err(WireError::InvalidField("claim_readiness")),
    }
}

fn disposition_to_u8(value: EabClaimDisposition) -> u8 {
    match value {
        EabClaimDisposition::Pending => 1,
        EabClaimDisposition::Acknowledged => 2,
        EabClaimDisposition::Rejected => 3,
        EabClaimDisposition::Conflict => 4,
    }
}

fn disposition_from_u8(value: u8) -> Result<EabClaimDisposition, WireError> {
    match value {
        1 => Ok(EabClaimDisposition::Pending),
        2 => Ok(EabClaimDisposition::Acknowledged),
        3 => Ok(EabClaimDisposition::Rejected),
        4 => Ok(EabClaimDisposition::Conflict),
        _ => Err(WireError::InvalidField("claim disposition")),
    }
}

fn decision_to_u8(value: EabClaimDecisionCode) -> u8 {
    match value {
        EabClaimDecisionCode::PendingReview => 1,
        EabClaimDecisionCode::Acknowledged => 2,
        EabClaimDecisionCode::AlreadyAcknowledged => 3,
        EabClaimDecisionCode::InvalidEnvelope => 4,
        EabClaimDecisionCode::ClaimNotReady => 5,
        EabClaimDecisionCode::ClaimIdPayloadMismatch => 6,
        EabClaimDecisionCode::DefinitionNotFound => 7,
        EabClaimDecisionCode::DefinitionIdentityMismatch => 8,
        EabClaimDecisionCode::DefinitionDigestMismatch => 9,
        EabClaimDecisionCode::IssuanceModeDisallowsClaim => 10,
        EabClaimDecisionCode::EvidenceRequired => 11,
        EabClaimDecisionCode::EventMismatch => 12,
        EabClaimDecisionCode::ThresholdNotMet => 13,
        EabClaimDecisionCode::RepeatableNotSupported => 14,
    }
}

fn decision_from_u8(value: u8) -> Result<EabClaimDecisionCode, WireError> {
    match value {
        1 => Ok(EabClaimDecisionCode::PendingReview),
        2 => Ok(EabClaimDecisionCode::Acknowledged),
        3 => Ok(EabClaimDecisionCode::AlreadyAcknowledged),
        4 => Ok(EabClaimDecisionCode::InvalidEnvelope),
        5 => Ok(EabClaimDecisionCode::ClaimNotReady),
        6 => Ok(EabClaimDecisionCode::ClaimIdPayloadMismatch),
        7 => Ok(EabClaimDecisionCode::DefinitionNotFound),
        8 => Ok(EabClaimDecisionCode::DefinitionIdentityMismatch),
        9 => Ok(EabClaimDecisionCode::DefinitionDigestMismatch),
        10 => Ok(EabClaimDecisionCode::IssuanceModeDisallowsClaim),
        11 => Ok(EabClaimDecisionCode::EvidenceRequired),
        12 => Ok(EabClaimDecisionCode::EventMismatch),
        13 => Ok(EabClaimDecisionCode::ThresholdNotMet),
        14 => Ok(EabClaimDecisionCode::RepeatableNotSupported),
        _ => Err(WireError::InvalidField("claim decision code")),
    }
}

fn cbor<T>(
    result: Result<T, minicbor::encode::Error<std::convert::Infallible>>,
) -> Result<T, WireError> {
    result.map_err(|error| WireError::CborEncode(error.to_string()))
}

fn expect_array(
    decoder: &mut Decoder<'_>,
    field: &'static str,
    expected: u64,
) -> Result<(), WireError> {
    let actual = decoder
        .array()
        .map_err(|error| WireError::CborDecode(error.to_string()))?
        .ok_or(WireError::IndefiniteLength(field))?;
    if actual != expected {
        return Err(WireError::UnexpectedArrayLength {
            field,
            expected,
            actual,
        });
    }
    Ok(())
}

fn decode_fixed_bytes<const N: usize>(
    decoder: &mut Decoder<'_>,
    field: &'static str,
) -> Result<[u8; N], WireError> {
    decoder
        .bytes()
        .map_err(|error| WireError::CborDecode(error.to_string()))?
        .try_into()
        .map_err(|_| WireError::InvalidField(field))
}

fn decode_text(
    decoder: &mut Decoder<'_>,
    maximum: usize,
    field: &'static str,
) -> Result<String, WireError> {
    let value = decoder
        .str()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    if field == "evidence" {
        validate_text_allow_controls(value, maximum, field)?;
    } else {
        validate_text(value, maximum, field)?;
    }
    Ok(value.to_owned())
}

fn decode_optional_text(
    decoder: &mut Decoder<'_>,
    maximum: usize,
    field: &'static str,
) -> Result<Option<String>, WireError> {
    if datatype(decoder)? == Type::Null {
        decode_null(decoder)?;
        Ok(None)
    } else {
        decode_text(decoder, maximum, field).map(Some)
    }
}

fn datatype(decoder: &Decoder<'_>) -> Result<Type, WireError> {
    decoder
        .datatype()
        .map_err(|error| WireError::CborDecode(error.to_string()))
}

fn decode_null(decoder: &mut Decoder<'_>) -> Result<(), WireError> {
    decoder
        .null()
        .map_err(|error| WireError::CborDecode(error.to_string()))
}

fn decode_u8(decoder: &mut Decoder<'_>) -> Result<u8, WireError> {
    decoder
        .u8()
        .map_err(|error| WireError::CborDecode(error.to_string()))
}

fn decode_u16(decoder: &mut Decoder<'_>) -> Result<u16, WireError> {
    decoder
        .u16()
        .map_err(|error| WireError::CborDecode(error.to_string()))
}

fn decode_u32(decoder: &mut Decoder<'_>) -> Result<u32, WireError> {
    decoder
        .u32()
        .map_err(|error| WireError::CborDecode(error.to_string()))
}

fn decode_u64(decoder: &mut Decoder<'_>) -> Result<u64, WireError> {
    decoder
        .u64()
        .map_err(|error| WireError::CborDecode(error.to_string()))
}

fn validate_request_id(request_id: &[u8; 16]) -> Result<(), WireError> {
    if request_id.iter().all(|byte| *byte == 0) {
        return Err(WireError::InvalidField("request_id must be non-zero"));
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize, field: &'static str) -> Result<(), WireError> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(WireError::InvalidField(field));
    }
    Ok(())
}

fn validate_text_allow_controls(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> Result<(), WireError> {
    if value.is_empty() || value.len() > maximum {
        return Err(WireError::InvalidField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> EabClaimEnvelope {
        EabClaimEnvelope {
            schema_version: 1,
            record: OfflineAchievementRecord {
                schema_version: 1,
                local_award_id: "local-award".into(),
                claim_id: "claim-1".into(),
                developer: "developer".into(),
                game: "game".into(),
                achievement_id: "first-win".into(),
                version: 1,
                definition_digest: "11".repeat(32),
                local_player_id: "local-player".into(),
                save_id: "save".into(),
                installation_id: "install".into(),
                session_id: "offline-session".into(),
                client_sequence: 7,
                earned_at_local: "2026-08-06T01:02:03Z".into(),
                recorded_at_local: "2026-08-06T01:02:04Z".into(),
                game_build: "1.0.0".into(),
                event_key: "match.won".into(),
                event_value: 1,
                evidence: Some("evidence\nbody".into()),
                claim_readiness: OfflineClaimReadiness::Ready,
                local_record_hash: "22".repeat(32),
            },
        }
    }

    fn acknowledgement() -> EabClaimAcknowledgement {
        EabClaimAcknowledgement {
            schema_version: 1,
            claim_id: "claim-1".into(),
            developer: "developer".into(),
            game: "game".into(),
            achievement_id: "first-win".into(),
            version: 1,
            disposition: EabClaimDisposition::Acknowledged,
            code: EabClaimDecisionCode::Acknowledged,
            first_observed_at: "2026-08-06T01:03:00Z".into(),
            decided_at: Some("2026-08-06T01:03:01Z".into()),
            award: Some(EabAwardReference {
                transaction_id: "transaction-1".into(),
                block_hash: "33".repeat(32),
            }),
        }
    }

    fn round_trip(message: SecureMessage) {
        let first = message.encode().unwrap();
        assert!(first.len() <= MAX_SECURE_FRAME_LEN);
        let decoded = SecureMessage::decode(&first).unwrap();
        assert_eq!(decoded, message);
        assert_eq!(decoded.encode().unwrap(), first);
    }

    #[test]
    fn canonical_claim_messages_round_trip_deterministically() {
        round_trip(SecureMessage::SubmitClaimRequest(SubmitClaimRequest {
            request_id: [1; 16],
            session_token: "secret-session".into(),
            envelope: envelope(),
        }));
        round_trip(SecureMessage::SubmitClaimResponse(SubmitClaimResponse {
            request_id: [1; 16],
            acknowledgement: acknowledgement(),
        }));
        round_trip(SecureMessage::ClaimStatusRequest(ClaimStatusRequest {
            request_id: [2; 16],
            session_token: "secret-session".into(),
            claim_id: "claim-1".into(),
        }));
        round_trip(SecureMessage::ClaimStatusResponse(ClaimStatusResponse {
            request_id: [2; 16],
            claim_id: "claim-1".into(),
            acknowledgement: Some(acknowledgement()),
        }));
        round_trip(SecureMessage::ClaimStatusResponse(ClaimStatusResponse {
            request_id: [2; 16],
            claim_id: "unknown-claim".into(),
            acknowledgement: None,
        }));
    }

    #[test]
    fn secure_decoder_rejects_outer_bounds_and_trailing_data() {
        assert!(matches!(
            SecureMessage::decode(&vec![0; MAX_SECURE_FRAME_LEN + 1]),
            Err(WireError::FrameTooLarge { .. })
        ));

        let mut encoded = SecureMessage::ClaimStatusRequest(ClaimStatusRequest {
            request_id: [2; 16],
            session_token: "secret-session".into(),
            claim_id: "claim-1".into(),
        })
        .encode()
        .unwrap();
        encoded.push(0);
        let payload_len = (encoded.len() - SECURE_HEADER_LEN) as u32;
        encoded[8..12].copy_from_slice(&payload_len.to_be_bytes());
        assert!(matches!(
            SecureMessage::decode(&encoded),
            Err(WireError::TrailingPayloadBytes(1))
        ));
    }

    #[test]
    fn claim_status_request_has_a_stable_golden_vector() {
        let message = SecureMessage::ClaimStatusRequest(ClaimStatusRequest {
            request_id: [1; 16],
            session_token: "s".into(),
            claim_id: "c".into(),
        });
        let expected = vec![
            0x45, 0x41, 0x42, 0x53, // EABS
            0x00, 0x02, // wire version 2
            0x00, 0x03, // ClaimStatusRequest
            0x00, 0x00, 0x00, 0x16, // 22-byte payload
            0x83, 0x50, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x01, 0x01, 0x01, 0x01, 0x61, 0x73, 0x61, 0x63,
        ];
        assert_eq!(message.encode().unwrap(), expected);
        assert_eq!(SecureMessage::decode(&expected), Ok(message));
    }

    #[test]
    fn secure_messages_reject_empty_auth_and_oversized_evidence() {
        let mut request = SubmitClaimRequest {
            request_id: [1; 16],
            session_token: String::new(),
            envelope: envelope(),
        };
        assert_eq!(
            SecureMessage::SubmitClaimRequest(request.clone()).encode(),
            Err(WireError::InvalidField("session_token"))
        );
        request.session_token = "session".into();
        request.envelope.record.evidence = Some("x".repeat(MAX_EVIDENCE_LEN + 1));
        assert_eq!(
            SecureMessage::SubmitClaimRequest(request).encode(),
            Err(WireError::InvalidField("evidence"))
        );
    }
}
