//! Versioned, bounded wire contracts shared by EAB transport adapters.
//!
//! This crate deliberately contains no socket, runtime, storage, or
//! authorization code. Callers must authenticate an advertised authority on a
//! secure unicast channel before sending any credential or private EAB data.

#![forbid(unsafe_code)]

mod secure;

pub use secure::{
    ClaimStatusRequest, ClaimStatusResponse, ProtocolErrorCode, ProtocolErrorResponse,
    SecureMessage, SecureMessageKind, SubmitClaimRequest, SubmitClaimResponse,
    MAX_SECURE_FRAME_LEN, MAX_SESSION_TOKEN_LEN, SECURE_HEADER_LEN, SECURE_MAGIC,
    SECURE_WIRE_VERSION,
};

use std::collections::BTreeSet;

use minicbor::{Decoder, Encoder};
use thiserror::Error;

/// Identifies an EAB V2 raw discovery datagram.
pub const DISCOVERY_MAGIC: [u8; 4] = *b"EAB2";

/// Current discovery frame version.
pub const DISCOVERY_WIRE_VERSION: u16 = 2;

/// Fixed bytes before a discovery CBOR payload.
pub const DISCOVERY_HEADER_LEN: usize = 10;

/// Conservative upper bound for an entire discovery datagram.
pub const MAX_DISCOVERY_DATAGRAM_LEN: usize = 1_200;

/// Largest node identifier accepted from unauthenticated discovery traffic.
pub const MAX_NODE_ID_LEN: usize = 64;

/// Largest textual QUIC endpoint accepted from discovery traffic.
pub const MAX_ENDPOINT_LEN: usize = 255;

/// Number of bytes in the first supported SHA-256 authority fingerprint.
pub const AUTHORITY_FINGERPRINT_LEN: usize = 32;

/// Size of client nonces and opaque server reachability cookies.
pub const DISCOVERY_TOKEN_LEN: usize = 16;

/// Maximum advertised capabilities in one discovery response.
pub const MAX_CAPABILITIES: usize = 32;

/// Capability identifier for a secure canonical achievement-claim service.
///
/// A node must not advertise this until its authenticated unicast service is
/// configured and ready.
pub const CAPABILITY_ACHIEVEMENT_CLAIM: u16 = 1;

/// Capability identifier for authenticated detailed node status.
pub const CAPABILITY_NODE_STATUS: u16 = 2;

/// Stable numeric type ids in the raw EAB V2 discovery header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum DiscoveryMessageKind {
    Probe = 1,
    Challenge = 2,
    Query = 3,
    Response = 4,
}

impl TryFrom<u16> for DiscoveryMessageKind {
    type Error = WireError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Probe),
            2 => Ok(Self::Challenge),
            3 => Ok(Self::Query),
            4 => Ok(Self::Response),
            other => Err(WireError::UnknownMessageKind(other)),
        }
    }
}

/// A small discovery request. Responses correlate on `request_id`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryProbe {
    pub request_id: [u8; 16],
    pub client_nonce: [u8; DISCOVERY_TOKEN_LEN],
    pub min_wire_version: u16,
    pub max_wire_version: u16,
}

/// A small server response carrying an opaque, source-bound cookie.
///
/// The challenge frame is no larger than the probe. Cookie generation and
/// expiry are transport concerns; the wire crate only carries the token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryChallenge {
    pub request_id: [u8; 16],
    pub cookie: [u8; DISCOVERY_TOKEN_LEN],
}

/// A client proves return reachability by echoing its nonce and server cookie.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryQuery {
    pub request_id: [u8; 16],
    pub client_nonce: [u8; DISCOVERY_TOKEN_LEN],
    pub cookie: [u8; DISCOVERY_TOKEN_LEN],
}

/// Public, low-trust authority metadata returned by discovery.
///
/// The fingerprint is only a hint for selecting a configured authority. The
/// secure unicast handshake must prove possession of the corresponding key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryResponse {
    pub request_id: [u8; 16],
    pub node_id: String,
    pub quic_endpoint: String,
    pub authority_fingerprint: [u8; AUTHORITY_FINGERPRINT_LEN],
    pub min_wire_version: u16,
    pub max_wire_version: u16,
    pub capabilities: Vec<u16>,
    pub expires_at_unix_seconds: u64,
}

/// All messages permitted on the raw multicast discovery plane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryMessage {
    Probe(DiscoveryProbe),
    Challenge(DiscoveryChallenge),
    Query(DiscoveryQuery),
    Response(DiscoveryResponse),
}

impl DiscoveryMessage {
    pub fn kind(&self) -> DiscoveryMessageKind {
        match self {
            Self::Probe(_) => DiscoveryMessageKind::Probe,
            Self::Challenge(_) => DiscoveryMessageKind::Challenge,
            Self::Query(_) => DiscoveryMessageKind::Query,
            Self::Response(_) => DiscoveryMessageKind::Response,
        }
    }

    /// Encode the fixed V2 header and deterministic, definite-length CBOR body.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.validate()?;

        let mut payload = Vec::new();
        let mut encoder = Encoder::new(&mut payload);
        match self {
            Self::Probe(probe) => encode_probe(&mut encoder, probe)?,
            Self::Challenge(challenge) => encode_challenge(&mut encoder, challenge)?,
            Self::Query(query) => encode_query(&mut encoder, query)?,
            Self::Response(response) => encode_response(&mut encoder, response)?,
        }

        if payload.len() > MAX_DISCOVERY_DATAGRAM_LEN - DISCOVERY_HEADER_LEN {
            return Err(WireError::DatagramTooLarge {
                actual: DISCOVERY_HEADER_LEN + payload.len(),
                maximum: MAX_DISCOVERY_DATAGRAM_LEN,
            });
        }

        let payload_len =
            u16::try_from(payload.len()).map_err(|_| WireError::DatagramTooLarge {
                actual: DISCOVERY_HEADER_LEN + payload.len(),
                maximum: MAX_DISCOVERY_DATAGRAM_LEN,
            })?;

        let mut frame = Vec::with_capacity(DISCOVERY_HEADER_LEN + payload.len());
        frame.extend_from_slice(&DISCOVERY_MAGIC);
        frame.extend_from_slice(&DISCOVERY_WIRE_VERSION.to_be_bytes());
        frame.extend_from_slice(&(self.kind() as u16).to_be_bytes());
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    /// Decode one complete discovery datagram after enforcing its outer bound.
    pub fn decode(frame: &[u8]) -> Result<Self, WireError> {
        if frame.len() > MAX_DISCOVERY_DATAGRAM_LEN {
            return Err(WireError::DatagramTooLarge {
                actual: frame.len(),
                maximum: MAX_DISCOVERY_DATAGRAM_LEN,
            });
        }
        if frame.len() < DISCOVERY_HEADER_LEN {
            return Err(WireError::TruncatedHeader {
                actual: frame.len(),
                required: DISCOVERY_HEADER_LEN,
            });
        }
        if frame[..4] != DISCOVERY_MAGIC {
            return Err(WireError::InvalidMagic);
        }

        let version = u16::from_be_bytes([frame[4], frame[5]]);
        if version != DISCOVERY_WIRE_VERSION {
            return Err(WireError::UnsupportedWireVersion(version));
        }

        let kind = DiscoveryMessageKind::try_from(u16::from_be_bytes([frame[6], frame[7]]))?;
        let declared_payload_len = u16::from_be_bytes([frame[8], frame[9]]) as usize;
        let actual_payload_len = frame.len() - DISCOVERY_HEADER_LEN;
        if declared_payload_len != actual_payload_len {
            return Err(WireError::LengthMismatch {
                declared: declared_payload_len,
                actual: actual_payload_len,
            });
        }

        let payload = &frame[DISCOVERY_HEADER_LEN..];
        let mut decoder = Decoder::new(payload);
        let message = match kind {
            DiscoveryMessageKind::Probe => Self::Probe(decode_probe(&mut decoder)?),
            DiscoveryMessageKind::Challenge => Self::Challenge(decode_challenge(&mut decoder)?),
            DiscoveryMessageKind::Query => Self::Query(decode_query(&mut decoder)?),
            DiscoveryMessageKind::Response => Self::Response(decode_response(&mut decoder)?),
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
        match self {
            Self::Probe(probe) => validate_probe(probe),
            Self::Challenge(challenge) => validate_challenge(challenge),
            Self::Query(query) => validate_query(query),
            Self::Response(response) => validate_response(response),
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WireError {
    #[error("discovery datagram is {actual} bytes; maximum is {maximum}")]
    DatagramTooLarge { actual: usize, maximum: usize },
    #[error("secure frame is {actual} bytes; maximum is {maximum}")]
    FrameTooLarge { actual: usize, maximum: usize },
    #[error("discovery header is truncated: got {actual} bytes; need {required}")]
    TruncatedHeader { actual: usize, required: usize },
    #[error("invalid EAB discovery magic")]
    InvalidMagic,
    #[error("unsupported EAB discovery wire version {0}")]
    UnsupportedWireVersion(u16),
    #[error("unknown EAB discovery message kind {0}")]
    UnknownMessageKind(u16),
    #[error("discovery payload length mismatch: header says {declared}; datagram has {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("invalid discovery field: {0}")]
    InvalidField(&'static str),
    #[error("indefinite-length CBOR is not allowed for {0}")]
    IndefiniteLength(&'static str),
    #[error("unexpected CBOR array length for {field}: expected {expected}, got {actual}")]
    UnexpectedArrayLength {
        field: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("discovery payload has {0} trailing bytes")]
    TrailingPayloadBytes(usize),
    #[error("CBOR encode failed: {0}")]
    CborEncode(String),
    #[error("CBOR decode failed: {0}")]
    CborDecode(String),
}

fn encode_probe(
    encoder: &mut Encoder<&mut Vec<u8>>,
    probe: &DiscoveryProbe,
) -> Result<(), WireError> {
    encoder
        .array(4)
        .and_then(|encoder| encoder.bytes(&probe.request_id))
        .and_then(|encoder| encoder.bytes(&probe.client_nonce))
        .and_then(|encoder| encoder.u16(probe.min_wire_version))
        .and_then(|encoder| encoder.u16(probe.max_wire_version))
        .map(|_| ())
        .map_err(|error| WireError::CborEncode(error.to_string()))
}

fn encode_challenge(
    encoder: &mut Encoder<&mut Vec<u8>>,
    challenge: &DiscoveryChallenge,
) -> Result<(), WireError> {
    encoder
        .array(2)
        .and_then(|encoder| encoder.bytes(&challenge.request_id))
        .and_then(|encoder| encoder.bytes(&challenge.cookie))
        .map(|_| ())
        .map_err(|error| WireError::CborEncode(error.to_string()))
}

fn encode_query(
    encoder: &mut Encoder<&mut Vec<u8>>,
    query: &DiscoveryQuery,
) -> Result<(), WireError> {
    encoder
        .array(3)
        .and_then(|encoder| encoder.bytes(&query.request_id))
        .and_then(|encoder| encoder.bytes(&query.client_nonce))
        .and_then(|encoder| encoder.bytes(&query.cookie))
        .map(|_| ())
        .map_err(|error| WireError::CborEncode(error.to_string()))
}

fn encode_response(
    encoder: &mut Encoder<&mut Vec<u8>>,
    response: &DiscoveryResponse,
) -> Result<(), WireError> {
    encoder
        .array(8)
        .and_then(|encoder| encoder.bytes(&response.request_id))
        .and_then(|encoder| encoder.str(&response.node_id))
        .and_then(|encoder| encoder.str(&response.quic_endpoint))
        .and_then(|encoder| encoder.bytes(&response.authority_fingerprint))
        .and_then(|encoder| encoder.u16(response.min_wire_version))
        .and_then(|encoder| encoder.u16(response.max_wire_version))
        .and_then(|encoder| encoder.array(response.capabilities.len() as u64))
        .map_err(|error| WireError::CborEncode(error.to_string()))?;
    for capability in &response.capabilities {
        encoder
            .u16(*capability)
            .map_err(|error| WireError::CborEncode(error.to_string()))?;
    }
    encoder
        .u64(response.expires_at_unix_seconds)
        .map(|_| ())
        .map_err(|error| WireError::CborEncode(error.to_string()))
}

fn decode_probe(decoder: &mut Decoder<'_>) -> Result<DiscoveryProbe, WireError> {
    expect_array_len(decoder, "probe", 4)?;
    Ok(DiscoveryProbe {
        request_id: decode_fixed_bytes::<16>(decoder, "request_id")?,
        client_nonce: decode_fixed_bytes::<DISCOVERY_TOKEN_LEN>(decoder, "client_nonce")?,
        min_wire_version: decoder
            .u16()
            .map_err(|error| WireError::CborDecode(error.to_string()))?,
        max_wire_version: decoder
            .u16()
            .map_err(|error| WireError::CborDecode(error.to_string()))?,
    })
}

fn decode_challenge(decoder: &mut Decoder<'_>) -> Result<DiscoveryChallenge, WireError> {
    expect_array_len(decoder, "challenge", 2)?;
    Ok(DiscoveryChallenge {
        request_id: decode_fixed_bytes::<16>(decoder, "request_id")?,
        cookie: decode_fixed_bytes::<DISCOVERY_TOKEN_LEN>(decoder, "cookie")?,
    })
}

fn decode_query(decoder: &mut Decoder<'_>) -> Result<DiscoveryQuery, WireError> {
    expect_array_len(decoder, "query", 3)?;
    Ok(DiscoveryQuery {
        request_id: decode_fixed_bytes::<16>(decoder, "request_id")?,
        client_nonce: decode_fixed_bytes::<DISCOVERY_TOKEN_LEN>(decoder, "client_nonce")?,
        cookie: decode_fixed_bytes::<DISCOVERY_TOKEN_LEN>(decoder, "cookie")?,
    })
}

fn decode_response(decoder: &mut Decoder<'_>) -> Result<DiscoveryResponse, WireError> {
    expect_array_len(decoder, "response", 8)?;
    let request_id = decode_fixed_bytes::<16>(decoder, "request_id")?;
    let node_id = decoder
        .str()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    validate_text(node_id, MAX_NODE_ID_LEN, "node_id")?;
    let node_id = node_id.to_owned();
    let quic_endpoint = decoder
        .str()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    validate_text(quic_endpoint, MAX_ENDPOINT_LEN, "quic_endpoint")?;
    let quic_endpoint = quic_endpoint.to_owned();
    let authority_fingerprint =
        decode_fixed_bytes::<AUTHORITY_FINGERPRINT_LEN>(decoder, "authority_fingerprint")?;
    let min_wire_version = decoder
        .u16()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    let max_wire_version = decoder
        .u16()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    let capability_count = decoder
        .array()
        .map_err(|error| WireError::CborDecode(error.to_string()))?
        .ok_or(WireError::IndefiniteLength("capabilities"))?;
    if capability_count > MAX_CAPABILITIES as u64 {
        return Err(WireError::InvalidField("capabilities exceeds maximum"));
    }
    let mut capabilities = Vec::with_capacity(capability_count as usize);
    for _ in 0..capability_count {
        capabilities.push(
            decoder
                .u16()
                .map_err(|error| WireError::CborDecode(error.to_string()))?,
        );
    }
    let expires_at_unix_seconds = decoder
        .u64()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    Ok(DiscoveryResponse {
        request_id,
        node_id,
        quic_endpoint,
        authority_fingerprint,
        min_wire_version,
        max_wire_version,
        capabilities,
        expires_at_unix_seconds,
    })
}

fn expect_array_len(
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
    let bytes = decoder
        .bytes()
        .map_err(|error| WireError::CborDecode(error.to_string()))?;
    bytes.try_into().map_err(|_| WireError::InvalidField(field))
}

fn validate_probe(probe: &DiscoveryProbe) -> Result<(), WireError> {
    validate_request_id(&probe.request_id)?;
    validate_token(&probe.client_nonce, "client_nonce must be non-zero")?;
    validate_version_range(probe.min_wire_version, probe.max_wire_version)
}

fn validate_challenge(challenge: &DiscoveryChallenge) -> Result<(), WireError> {
    validate_request_id(&challenge.request_id)?;
    validate_token(&challenge.cookie, "cookie must be non-zero")
}

fn validate_query(query: &DiscoveryQuery) -> Result<(), WireError> {
    validate_request_id(&query.request_id)?;
    validate_token(&query.client_nonce, "client_nonce must be non-zero")?;
    validate_token(&query.cookie, "cookie must be non-zero")
}

fn validate_response(response: &DiscoveryResponse) -> Result<(), WireError> {
    validate_request_id(&response.request_id)?;
    validate_token(
        &response.authority_fingerprint,
        "authority_fingerprint must be non-zero",
    )?;
    validate_version_range(response.min_wire_version, response.max_wire_version)?;
    validate_text(&response.node_id, MAX_NODE_ID_LEN, "node_id")?;
    validate_text(&response.quic_endpoint, MAX_ENDPOINT_LEN, "quic_endpoint")?;
    if response.expires_at_unix_seconds == 0 {
        return Err(WireError::InvalidField(
            "expires_at_unix_seconds must be non-zero",
        ));
    }
    if response.capabilities.len() > MAX_CAPABILITIES {
        return Err(WireError::InvalidField("capabilities exceeds maximum"));
    }
    let mut unique = BTreeSet::new();
    for capability in &response.capabilities {
        if *capability == 0 {
            return Err(WireError::InvalidField("capability id must be non-zero"));
        }
        if !unique.insert(*capability) {
            return Err(WireError::InvalidField("capability ids must be unique"));
        }
    }
    if !response
        .capabilities
        .windows(2)
        .all(|pair| pair[0] < pair[1])
    {
        return Err(WireError::InvalidField(
            "capability ids must be in ascending order",
        ));
    }
    Ok(())
}

fn validate_request_id(request_id: &[u8; 16]) -> Result<(), WireError> {
    if request_id.iter().all(|byte| *byte == 0) {
        return Err(WireError::InvalidField("request_id must be non-zero"));
    }
    Ok(())
}

fn validate_token<const N: usize>(token: &[u8; N], error: &'static str) -> Result<(), WireError> {
    if token.iter().all(|byte| *byte == 0) {
        return Err(WireError::InvalidField(error));
    }
    Ok(())
}

fn validate_version_range(minimum: u16, maximum: u16) -> Result<(), WireError> {
    if minimum == 0 || maximum == 0 || minimum > maximum {
        return Err(WireError::InvalidField("invalid wire version range"));
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize, field: &'static str) -> Result<(), WireError> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(WireError::InvalidField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_id() -> [u8; 16] {
        [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ]
    }

    fn client_nonce() -> [u8; DISCOVERY_TOKEN_LEN] {
        [0x55; DISCOVERY_TOKEN_LEN]
    }

    fn cookie() -> [u8; DISCOVERY_TOKEN_LEN] {
        [0xcc; DISCOVERY_TOKEN_LEN]
    }

    fn response() -> DiscoveryResponse {
        DiscoveryResponse {
            request_id: request_id(),
            node_id: "authority-1".to_owned(),
            quic_endpoint: "[fe80::1%3]:4542".to_owned(),
            authority_fingerprint: [0xa5; AUTHORITY_FINGERPRINT_LEN],
            min_wire_version: 2,
            max_wire_version: 2,
            capabilities: vec![CAPABILITY_ACHIEVEMENT_CLAIM, CAPABILITY_NODE_STATUS],
            expires_at_unix_seconds: 1_800_000_000,
        }
    }

    #[test]
    fn probe_has_a_stable_golden_vector() {
        let message = DiscoveryMessage::Probe(DiscoveryProbe {
            request_id: request_id(),
            client_nonce: client_nonce(),
            min_wire_version: 2,
            max_wire_version: 2,
        });

        let encoded = message.encode().expect("probe should encode");
        let expected = hex_literal(&[
            0x45, 0x41, 0x42, 0x32, // EAB2
            0x00, 0x02, // wire version 2
            0x00, 0x01, // probe
            0x00, 0x25, // 37-byte payload
            0x84, 0x50, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
            0x0c, 0x0d, 0x0e, 0x0f, 0x50, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55,
            0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x02, 0x02,
        ]);
        assert_eq!(encoded, expected);
        assert_eq!(DiscoveryMessage::decode(&encoded), Ok(message));
    }

    #[test]
    fn challenge_is_no_larger_than_probe_and_query_echoes_the_cookie() {
        let probe = DiscoveryMessage::Probe(DiscoveryProbe {
            request_id: request_id(),
            client_nonce: client_nonce(),
            min_wire_version: 2,
            max_wire_version: 2,
        });
        let challenge = DiscoveryMessage::Challenge(DiscoveryChallenge {
            request_id: request_id(),
            cookie: cookie(),
        });
        let query = DiscoveryMessage::Query(DiscoveryQuery {
            request_id: request_id(),
            client_nonce: client_nonce(),
            cookie: cookie(),
        });

        let probe_bytes = probe.encode().expect("probe should encode");
        let challenge_bytes = challenge.encode().expect("challenge should encode");
        assert!(challenge_bytes.len() <= probe_bytes.len());
        assert_eq!(DiscoveryMessage::decode(&challenge_bytes), Ok(challenge));
        assert_eq!(
            DiscoveryMessage::decode(&query.encode().expect("query should encode")),
            Ok(query)
        );
    }

    #[test]
    fn challenge_query_and_response_have_stable_golden_vectors() {
        let challenge = DiscoveryMessage::Challenge(DiscoveryChallenge {
            request_id: request_id(),
            cookie: cookie(),
        });
        assert_golden(
            challenge,
            &[
                0x45, 0x41, 0x42, 0x32, 0x00, 0x02, 0x00, 0x02, 0x00, 0x23, 0x82, 0x50, 0x00, 0x01,
                0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
                0x50, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc, 0xcc,
            ],
        );

        let query = DiscoveryMessage::Query(DiscoveryQuery {
            request_id: request_id(),
            client_nonce: client_nonce(),
            cookie: cookie(),
        });
        assert_golden(
            query,
            &[
                0x45, 0x41, 0x42, 0x32, 0x00, 0x02, 0x00, 0x03, 0x00, 0x34, 0x83, 0x50, 0x00, 0x01,
                0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
                0x50, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55,
                0x55, 0x55, 0x55, 0x50, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
            ],
        );

        let response = DiscoveryMessage::Response(response());
        assert_golden(
            response,
            &[
                0x45, 0x41, 0x42, 0x32, 0x00, 0x02, 0x00, 0x04, 0x00, 0x5b, 0x88, 0x50, 0x00, 0x01,
                0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
                0x6b, 0x61, 0x75, 0x74, 0x68, 0x6f, 0x72, 0x69, 0x74, 0x79, 0x2d, 0x31, 0x70, 0x5b,
                0x66, 0x65, 0x38, 0x30, 0x3a, 0x3a, 0x31, 0x25, 0x33, 0x5d, 0x3a, 0x34, 0x35, 0x34,
                0x32, 0x58, 0x20, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5,
                0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5,
                0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0x02, 0x02, 0x82, 0x01, 0x02, 0x1a, 0x6b,
                0x49, 0xd2, 0x00,
            ],
        );
    }

    #[test]
    fn response_round_trips_deterministically() {
        let message = DiscoveryMessage::Response(response());
        let first = message.encode().expect("response should encode");
        let decoded = DiscoveryMessage::decode(&first).expect("response should decode");
        let second = decoded.encode().expect("decoded response should re-encode");

        assert_eq!(decoded, message);
        assert_eq!(second, first);
        assert!(first.len() <= MAX_DISCOVERY_DATAGRAM_LEN);
    }

    #[test]
    fn outer_bound_is_checked_before_header_or_cbor_decoding() {
        let oversized = vec![0_u8; MAX_DISCOVERY_DATAGRAM_LEN + 1];
        assert_eq!(
            DiscoveryMessage::decode(&oversized),
            Err(WireError::DatagramTooLarge {
                actual: MAX_DISCOVERY_DATAGRAM_LEN + 1,
                maximum: MAX_DISCOVERY_DATAGRAM_LEN,
            })
        );
    }

    #[test]
    fn malformed_headers_and_payload_lengths_are_rejected() {
        assert!(matches!(
            DiscoveryMessage::decode(b"EAB2"),
            Err(WireError::TruncatedHeader { .. })
        ));

        let mut encoded = DiscoveryMessage::Response(response())
            .encode()
            .expect("response should encode");
        encoded[5] = 3;
        assert_eq!(
            DiscoveryMessage::decode(&encoded),
            Err(WireError::UnsupportedWireVersion(3))
        );

        encoded[5] = 2;
        encoded[7] = 99;
        assert_eq!(
            DiscoveryMessage::decode(&encoded),
            Err(WireError::UnknownMessageKind(99))
        );

        encoded[7] = DiscoveryMessageKind::Response as u8;
        encoded.push(0);
        assert!(matches!(
            DiscoveryMessage::decode(&encoded),
            Err(WireError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn invalid_field_bounds_are_rejected_before_encoding() {
        let mut invalid = response();
        invalid.node_id = "x".repeat(MAX_NODE_ID_LEN + 1);
        assert_eq!(
            DiscoveryMessage::Response(invalid).encode(),
            Err(WireError::InvalidField("node_id"))
        );

        let mut invalid = response();
        invalid.capabilities = vec![1, 1];
        assert_eq!(
            DiscoveryMessage::Response(invalid).encode(),
            Err(WireError::InvalidField("capability ids must be unique"))
        );

        let mut invalid = response();
        invalid.authority_fingerprint = [0; AUTHORITY_FINGERPRINT_LEN];
        assert_eq!(
            DiscoveryMessage::Response(invalid).encode(),
            Err(WireError::InvalidField(
                "authority_fingerprint must be non-zero"
            ))
        );

        let invalid = DiscoveryProbe {
            request_id: [0; 16],
            client_nonce: client_nonce(),
            min_wire_version: 2,
            max_wire_version: 2,
        };
        assert_eq!(
            DiscoveryMessage::Probe(invalid).encode(),
            Err(WireError::InvalidField("request_id must be non-zero"))
        );
    }

    #[test]
    fn trailing_cbor_data_is_rejected_even_with_a_matching_outer_length() {
        let mut encoded = DiscoveryMessage::Probe(DiscoveryProbe {
            request_id: request_id(),
            client_nonce: client_nonce(),
            min_wire_version: 2,
            max_wire_version: 2,
        })
        .encode()
        .expect("probe should encode");
        encoded.push(0);
        let payload_len = (encoded.len() - DISCOVERY_HEADER_LEN) as u16;
        encoded[8..10].copy_from_slice(&payload_len.to_be_bytes());

        assert_eq!(
            DiscoveryMessage::decode(&encoded),
            Err(WireError::TrailingPayloadBytes(1))
        );
    }

    fn hex_literal(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    fn assert_golden(message: DiscoveryMessage, expected: &[u8]) {
        let encoded = message.encode().expect("golden message should encode");
        assert_eq!(encoded, expected);
        assert_eq!(DiscoveryMessage::decode(expected), Ok(message));
    }
}
