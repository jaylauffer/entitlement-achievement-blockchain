#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use eab_wire::{SecureMessage, MAX_SECURE_FRAME_LEN};
use quinn::crypto::rustls::QuicClientConfig;
use quinn::rustls;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

const AUTHORITY_SERVER_NAME: &str = "eab-authority";
const EAB_QUIC_ALPN: &[u8] = b"eab/2";
const ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum QuicClientError {
    #[error("authority certificate pin must be non-zero")]
    EmptyTrust,
    #[error("invalid EAB secure message: {0}")]
    Wire(String),
    #[error("QUIC secure message timed out")]
    Timeout,
    #[error("QUIC transport failed: {0}")]
    Transport(String),
    #[error("EAB secure response request_id mismatch")]
    Correlation,
}

#[derive(Clone, Debug)]
pub struct PinnedQuicClient {
    target: SocketAddr,
    authority_certificate_fingerprint: [u8; 32],
}

impl PinnedQuicClient {
    pub fn new(
        target: SocketAddr,
        authority_certificate_fingerprint: [u8; 32],
    ) -> Result<Self, QuicClientError> {
        if authority_certificate_fingerprint
            .iter()
            .all(|byte| *byte == 0)
        {
            return Err(QuicClientError::EmptyTrust);
        }
        Ok(Self {
            target,
            authority_certificate_fingerprint,
        })
    }

    pub fn target(&self) -> SocketAddr {
        self.target
    }

    pub fn round_trip(&self, request: SecureMessage) -> Result<SecureMessage, QuicClientError> {
        let request_id = request.request_id();
        let request = request
            .encode()
            .map_err(|error| QuicClientError::Wire(error.to_string()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(transport)?;
        let response = runtime.block_on(async {
            tokio::time::timeout(ROUND_TRIP_TIMEOUT, self.round_trip_async(request))
                .await
                .map_err(|_| QuicClientError::Timeout)?
        })?;
        let response = SecureMessage::decode(&response)
            .map_err(|error| QuicClientError::Wire(error.to_string()))?;
        if response.request_id() != request_id {
            return Err(QuicClientError::Correlation);
        }
        Ok(response)
    }

    async fn round_trip_async(&self, request: Vec<u8>) -> Result<Vec<u8>, QuicClientError> {
        let bind_addr = match self.target.ip() {
            IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
        };
        let mut endpoint = quinn::Endpoint::client(bind_addr).map_err(transport)?;
        endpoint.set_default_client_config(client_config(self.authority_certificate_fingerprint)?);
        let connection = endpoint
            .connect(self.target, AUTHORITY_SERVER_NAME)
            .map_err(transport)?
            .await
            .map_err(transport)?;
        let (mut send, mut receive) = connection.open_bi().await.map_err(transport)?;
        send.write_all(&request).await.map_err(transport)?;
        send.finish().map_err(transport)?;
        let response = receive
            .read_to_end(MAX_SECURE_FRAME_LEN)
            .await
            .map_err(transport)?;
        connection.close(0_u32.into(), b"EAB request complete");
        endpoint.wait_idle().await;
        Ok(response)
    }
}

fn client_config(expected_fingerprint: [u8; 32]) -> Result<quinn::ClientConfig, QuicClientError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedCertificateVerifier {
        expected_fingerprint,
        provider: provider.clone(),
    });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(transport)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    tls.alpn_protocols = vec![EAB_QUIC_ALPN.to_vec()];
    tls.enable_early_data = false;
    let crypto = QuicClientConfig::try_from(tls).map_err(transport)?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(Arc::new(transport_config()?));
    Ok(config)
}

fn transport_config() -> Result<quinn::TransportConfig, QuicClientError> {
    let mut transport_config = quinn::TransportConfig::default();
    transport_config.max_concurrent_uni_streams(0_u8.into());
    transport_config.max_concurrent_bidi_streams(8_u8.into());
    transport_config.max_idle_timeout(Some(Duration::from_secs(10).try_into().map_err(transport)?));
    Ok(transport_config)
}

fn certificate_fingerprint(certificate: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(certificate.as_ref()).into()
}

struct PinnedCertificateVerifier {
    expected_fingerprint: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl fmt::Debug for PinnedCertificateVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedCertificateVerifier")
            .field("expected_fingerprint", &"[redacted]")
            .finish()
    }
}

impl ServerCertVerifier for PinnedCertificateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if certificate_fingerprint(end_entity) != self.expected_fingerprint {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ));
        }
        let mut roots = RootCertStore::empty();
        roots.add(end_entity.clone())?;
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            self.provider.clone(),
        )
        .build()
        .map_err(|error| rustls::Error::General(error.to_string()))?;
        verifier.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn transport(error: impl std::fmt::Display) -> QuicClientError {
    QuicClientError::Transport(error.to_string())
}
