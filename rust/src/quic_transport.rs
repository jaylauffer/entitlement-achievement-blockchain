//! Experimental secure-unicast data plane for EAB.
//!
//! Discovery remains a bounded UDP/multicast protocol. It returns a QUIC endpoint and
//! a SHA-256 certificate fingerprint; this module demonstrates that a client can use
//! that fingerprint to authenticate a TLS 1.3 QUIC connection before any application
//! bytes are accepted. The application protocol in this spike is deliberately only a
//! bounded echo acknowledgement. Claims and server acknowledgements remain transport
//! independent and will be layered on this channel after the transport decision.

use anyhow::{anyhow, Context, Result};
use eab_wire::{SecureMessage, MAX_SECURE_FRAME_LEN};
use loadngo_proactor::{ChannelPort, CompletionKind, ProactorHandle};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::rustls;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use tokio::sync::oneshot;

const AUTHORITY_SERVER_NAME: &str = "eab-authority";
const EAB_QUIC_ALPN: &[u8] = b"eab/2";
const MAX_SPIKE_REQUEST_LEN: usize = 4 * 1024;
const MAX_SPIKE_RESPONSE_LEN: usize = MAX_SPIKE_REQUEST_LEN + 64;
const ACK_PREFIX: &[u8] = b"EAB-QUIC-SPIKE-ACK\0";
const START_TIMEOUT: Duration = Duration::from_secs(5);
const ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(5);

/// Ephemeral identity helper for the spike.
///
/// Production providers should load a persistent key and certificate so the
/// advertised fingerprint remains stable across restarts.
pub struct QuicServerIdentity {
    certificate: CertificateDer<'static>,
    private_key: PrivateKeyDer<'static>,
}

impl QuicServerIdentity {
    pub fn generate_for_spike() -> Result<Self> {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec![AUTHORITY_SERVER_NAME.to_string()])
                .context("failed to generate QUIC spike identity")?;
        Ok(Self {
            certificate: cert.der().clone(),
            private_key: PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        })
    }

    pub fn certificate_fingerprint(&self) -> [u8; 32] {
        certificate_fingerprint(&self.certificate)
    }

    /// Loads a persistent authority identity from a DER certificate and a
    /// PKCS#8 DER private key. Key/certificate consistency is verified by
    /// rustls when the server configuration is constructed.
    pub fn from_pkcs8_der(certificate_der: Vec<u8>, private_key_der: Vec<u8>) -> Result<Self> {
        if certificate_der.is_empty() || private_key_der.is_empty() {
            return Err(anyhow!(
                "QUIC authority certificate and private key must be non-empty"
            ));
        }
        Ok(Self {
            certificate: CertificateDer::from(certificate_der),
            private_key: PrivatePkcs8KeyDer::from(private_key_der).into(),
        })
    }

    pub fn load_pkcs8_der_files(
        certificate_path: impl AsRef<Path>,
        private_key_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let certificate_path = certificate_path.as_ref();
        let private_key_path = private_key_path.as_ref();
        let certificate = fs::read(certificate_path).with_context(|| {
            format!(
                "failed to read QUIC authority certificate {}",
                certificate_path.display()
            )
        })?;
        let private_key = fs::read(private_key_path).with_context(|| {
            format!(
                "failed to read QUIC authority private key {}",
                private_key_path.display()
            )
        })?;
        Self::from_pkcs8_der(certificate, private_key)
    }
}

/// Running QUIC authority endpoint initiated through the EAB proactor.
///
/// Quinn's high-level API uses Tokio. For this spike the proactor owns startup
/// and shutdown, while a dedicated current-thread Tokio runtime owns QUIC I/O.
/// This keeps the integration boundary explicit until loadngo has a native QUIC
/// completion backend.
pub struct QuicSecureServer {
    local_addr: SocketAddr,
    certificate_fingerprint: [u8; 32],
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

/// Transport-independent application handler invoked only after the authority
/// TLS handshake succeeds and a complete bounded secure frame is decoded.
pub trait SecureRequestHandler: Send + Sync + 'static {
    fn handle(&self, request: SecureMessage) -> SecureMessage;
}

impl<F> SecureRequestHandler for F
where
    F: Fn(SecureMessage) -> SecureMessage + Send + Sync + 'static,
{
    fn handle(&self, request: SecureMessage) -> SecureMessage {
        self(request)
    }
}

#[derive(Clone)]
enum ServerApplication {
    SpikeEcho,
    SecureMessages(Arc<dyn SecureRequestHandler>),
}

impl QuicSecureServer {
    pub fn start_on_proactor(
        handle: &ProactorHandle<ChannelPort>,
        bind_addr: SocketAddr,
        identity: QuicServerIdentity,
    ) -> Result<Self> {
        Self::start_application_on_proactor(
            handle,
            bind_addr,
            identity,
            ServerApplication::SpikeEcho,
        )
    }

    pub fn start_secure_message_service_on_proactor(
        handle: &ProactorHandle<ChannelPort>,
        bind_addr: SocketAddr,
        identity: QuicServerIdentity,
        handler: Arc<dyn SecureRequestHandler>,
    ) -> Result<Self> {
        Self::start_application_on_proactor(
            handle,
            bind_addr,
            identity,
            ServerApplication::SecureMessages(handler),
        )
    }

    fn start_application_on_proactor(
        handle: &ProactorHandle<ChannelPort>,
        bind_addr: SocketAddr,
        identity: QuicServerIdentity,
        application: ServerApplication,
    ) -> Result<Self> {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        handle
            .enqueue(CompletionKind::Net, 0, move |_| {
                // Do not hold up the completion loop while socket/runtime setup occurs.
                thread::spawn(move || {
                    let _ = result_tx.send(Self::start(bind_addr, identity, application));
                });
            })
            .context("failed to enqueue QUIC startup on the EAB proactor")?;

        result_rx
            .recv_timeout(START_TIMEOUT)
            .context("timed out waiting for QUIC startup completion")?
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn certificate_fingerprint(&self) -> [u8; 32] {
        self.certificate_fingerprint
    }

    fn start(
        bind_addr: SocketAddr,
        identity: QuicServerIdentity,
        application: ServerApplication,
    ) -> Result<Self> {
        let fingerprint = identity.certificate_fingerprint();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let worker = thread::Builder::new()
            .name("eab-quic-authority".to_string())
            .spawn(move || run_server(bind_addr, identity, application, shutdown_rx, ready_tx))
            .context("failed to spawn QUIC authority worker")?;

        let local_addr = match ready_rx
            .recv_timeout(START_TIMEOUT)
            .context("timed out waiting for QUIC authority socket")?
        {
            Ok(addr) => addr,
            Err(err) => {
                let _ = worker.join();
                return Err(err);
            }
        };

        Ok(Self {
            local_addr,
            certificate_fingerprint: fingerprint,
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
        })
    }
}

impl Drop for QuicSecureServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Opens a certificate-pinned QUIC connection and exercises the bounded spike
/// acknowledgement protocol.
pub fn secure_spike_round_trip(
    target: SocketAddr,
    expected_certificate_fingerprint: [u8; 32],
    request: &[u8],
) -> Result<Vec<u8>> {
    if expected_certificate_fingerprint
        .iter()
        .all(|byte| *byte == 0)
    {
        return Err(anyhow!("QUIC authority certificate pin must be non-zero"));
    }
    if request.len() > MAX_SPIKE_REQUEST_LEN {
        return Err(anyhow!(
            "QUIC spike request exceeds {MAX_SPIKE_REQUEST_LEN} bytes"
        ));
    }

    let request = request.to_vec();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create QUIC client runtime")?;
    runtime.block_on(async move {
        tokio::time::timeout(
            ROUND_TRIP_TIMEOUT,
            secure_bytes_round_trip_async(
                target,
                expected_certificate_fingerprint,
                request,
                MAX_SPIKE_RESPONSE_LEN,
            ),
        )
        .await
        .context("QUIC spike round trip timed out")?
    })
}

/// Sends one bounded `eab-wire` secure request and requires a correlated
/// response on a certificate-pinned QUIC connection.
pub fn secure_message_round_trip(
    target: SocketAddr,
    expected_certificate_fingerprint: [u8; 32],
    request: SecureMessage,
) -> Result<SecureMessage> {
    if expected_certificate_fingerprint
        .iter()
        .all(|byte| *byte == 0)
    {
        return Err(anyhow!("QUIC authority certificate pin must be non-zero"));
    }
    let request_id = request.request_id();
    let request = request
        .encode()
        .context("invalid EAB secure request message")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create QUIC client runtime")?;
    let response = runtime.block_on(async move {
        tokio::time::timeout(
            ROUND_TRIP_TIMEOUT,
            secure_bytes_round_trip_async(
                target,
                expected_certificate_fingerprint,
                request,
                MAX_SECURE_FRAME_LEN,
            ),
        )
        .await
        .context("QUIC secure message round trip timed out")?
    })?;
    let response = SecureMessage::decode(&response).context("invalid EAB secure response frame")?;
    if response.request_id() != request_id {
        return Err(anyhow!("EAB secure response request_id mismatch"));
    }
    Ok(response)
}

fn run_server(
    bind_addr: SocketAddr,
    identity: QuicServerIdentity,
    application: ServerApplication,
    shutdown: oneshot::Receiver<()>,
    ready: mpsc::SyncSender<Result<SocketAddr>>,
) {
    let startup_ready = ready.clone();
    let result = (|| -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to create QUIC authority runtime")?;
        runtime.block_on(async move {
            let endpoint = quinn::Endpoint::server(server_config(identity)?, bind_addr)
                .context("failed to bind QUIC authority endpoint")?;
            let local_addr = endpoint
                .local_addr()
                .context("failed to inspect QUIC authority address")?;
            if startup_ready.send(Ok(local_addr)).is_err() {
                return Ok(());
            }

            let mut shutdown = shutdown;
            loop {
                match shutdown.try_recv() {
                    Ok(()) | Err(oneshot::error::TryRecvError::Closed) => {
                        endpoint.close(0_u32.into(), b"EAB authority shutdown");
                        endpoint.wait_idle().await;
                        break;
                    }
                    Err(oneshot::error::TryRecvError::Empty) => {}
                }

                let incoming =
                    match tokio::time::timeout(Duration::from_millis(100), endpoint.accept()).await
                    {
                        Ok(Some(incoming)) => incoming,
                        Ok(None) => break,
                        Err(_) => continue,
                    };
                let application = application.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(incoming, application).await {
                        eprintln!("EAB QUIC connection rejected: {err:#}");
                    }
                });
            }
            Ok(())
        })
    })();

    if let Err(err) = result {
        let _ = ready.send(Err(err));
    }
}

async fn handle_connection(
    incoming: quinn::Incoming,
    application: ServerApplication,
) -> Result<()> {
    let connection = incoming.await.context("QUIC handshake failed")?;
    loop {
        let (mut send, mut receive) = match connection.accept_bi().await {
            Ok(streams) => streams,
            Err(quinn::ConnectionError::ApplicationClosed(_)) => return Ok(()),
            Err(err) => return Err(err).context("failed to accept QUIC stream"),
        };
        let application = application.clone();
        tokio::spawn(async move {
            let result: Result<()> = async {
                let maximum = match &application {
                    ServerApplication::SpikeEcho => MAX_SPIKE_REQUEST_LEN,
                    ServerApplication::SecureMessages(_) => MAX_SECURE_FRAME_LEN,
                };
                let request = receive.read_to_end(maximum).await.with_context(|| {
                    format!("invalid or oversized QUIC request; maximum is {maximum} bytes")
                })?;
                let response = match application {
                    ServerApplication::SpikeEcho => {
                        let mut response = Vec::with_capacity(ACK_PREFIX.len() + request.len());
                        response.extend_from_slice(ACK_PREFIX);
                        response.extend_from_slice(&request);
                        response
                    }
                    ServerApplication::SecureMessages(handler) => {
                        let request = SecureMessage::decode(&request)
                            .context("invalid EAB secure request frame")?;
                        handler
                            .handle(request)
                            .encode()
                            .context("failed to encode EAB secure response frame")?
                    }
                };
                send.write_all(&response)
                    .await
                    .context("failed to write QUIC response")?;
                send.finish().context("failed to finish QUIC response")?;
                Ok(())
            }
            .await;
            if let Err(err) = result {
                eprintln!("EAB QUIC stream rejected: {err:#}");
            }
        });
    }
}

async fn secure_bytes_round_trip_async(
    target: SocketAddr,
    expected_certificate_fingerprint: [u8; 32],
    request: Vec<u8>,
    maximum_response_len: usize,
) -> Result<Vec<u8>> {
    let bind_addr = match target.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let mut endpoint = quinn::Endpoint::client(bind_addr)
        .context("failed to bind ephemeral QUIC client endpoint")?;
    endpoint.set_default_client_config(client_config(expected_certificate_fingerprint)?);

    let connection = endpoint
        .connect(target, AUTHORITY_SERVER_NAME)
        .context("invalid QUIC authority endpoint")?
        .await
        .context("QUIC authority authentication or handshake failed")?;
    let (mut send, mut receive) = connection
        .open_bi()
        .await
        .context("failed to open QUIC spike stream")?;
    send.write_all(&request)
        .await
        .context("failed to write QUIC spike request")?;
    send.finish()
        .context("failed to finish QUIC spike request")?;
    let response = receive
        .read_to_end(maximum_response_len)
        .await
        .context("invalid or oversized QUIC spike response")?;
    connection.close(0_u32.into(), b"EAB spike complete");
    endpoint.wait_idle().await;
    Ok(response)
}

fn server_config(identity: QuicServerIdentity) -> Result<quinn::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("failed to restrict QUIC authority TLS to 1.3")?
        .with_no_client_auth()
        .with_single_cert(vec![identity.certificate], identity.private_key.clone_key())
        .context("invalid QUIC authority certificate or key")?;
    tls.alpn_protocols = vec![EAB_QUIC_ALPN.to_vec()];
    let crypto = QuicServerConfig::try_from(tls).context("invalid QUIC server TLS config")?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(Arc::new(transport_config()?));
    Ok(config)
}

fn client_config(expected_certificate_fingerprint: [u8; 32]) -> Result<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedCertificateVerifier {
        expected_certificate_fingerprint,
        provider: provider.clone(),
    });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("failed to restrict QUIC client TLS to 1.3")?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    tls.alpn_protocols = vec![EAB_QUIC_ALPN.to_vec()];
    tls.enable_early_data = false;
    let crypto = QuicClientConfig::try_from(tls).context("invalid QUIC client TLS config")?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(Arc::new(transport_config()?));
    Ok(config)
}

fn transport_config() -> Result<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_uni_streams(0_u8.into());
    transport.max_concurrent_bidi_streams(8_u8.into());
    transport.max_idle_timeout(Some(
        Duration::from_secs(10)
            .try_into()
            .context("invalid QUIC idle timeout")?,
    ));
    Ok(transport)
}

fn certificate_fingerprint(certificate: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(certificate.as_ref()).into()
}

struct PinnedCertificateVerifier {
    expected_certificate_fingerprint: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl fmt::Debug for PinnedCertificateVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedCertificateVerifier")
            .field("expected_certificate_fingerprint", &"[redacted]")
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
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if certificate_fingerprint(end_entity) != self.expected_certificate_fingerprint {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ));
        }

        // The pin selects the exact trust anchor, but WebPKI still enforces
        // certificate parsing, validity period, server name, and chain rules.
        let mut roots = RootCertStore::empty();
        roots.add(end_entity.clone())?;
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            self.provider.clone(),
        )
        .build()
        .map_err(|err| rustls::Error::General(err.to_string()))?;
        verifier.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
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
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use loadngo_proactor::{ChannelPort, Proactor};

    fn running_proactor() -> (ProactorHandle<ChannelPort>, thread::JoinHandle<()>) {
        let proactor = Proactor::new(ChannelPort::new());
        let handle = proactor.handle();
        let worker = thread::spawn(move || proactor.run_until_stopped().unwrap());
        (handle, worker)
    }

    #[test]
    fn pinned_quic_spike_round_trip_succeeds() {
        let (handle, worker) = running_proactor();
        let identity = QuicServerIdentity::generate_for_spike().unwrap();
        let expected_pin = identity.certificate_fingerprint();
        let server =
            QuicSecureServer::start_on_proactor(&handle, "127.0.0.1:0".parse().unwrap(), identity)
                .unwrap();

        assert_eq!(server.certificate_fingerprint(), expected_pin);
        let response =
            secure_spike_round_trip(server.local_addr(), expected_pin, b"probe").unwrap();
        assert_eq!(response, [ACK_PREFIX, b"probe"].concat());

        drop(server);
        handle.stop().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn pinned_quic_spike_rejects_wrong_authority() {
        let (handle, worker) = running_proactor();
        let identity = QuicServerIdentity::generate_for_spike().unwrap();
        let server =
            QuicSecureServer::start_on_proactor(&handle, "127.0.0.1:0".parse().unwrap(), identity)
                .unwrap();

        assert!(secure_spike_round_trip(server.local_addr(), [0xA5; 32], b"probe").is_err());

        drop(server);
        handle.stop().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn spike_rejects_oversized_payload_before_network_io() {
        let request = vec![0_u8; MAX_SPIKE_REQUEST_LEN + 1];
        let result = secure_spike_round_trip("127.0.0.1:9".parse().unwrap(), [1_u8; 32], &request);
        assert!(result.is_err());
    }

    #[test]
    fn spike_rejects_empty_trust_before_network_io() {
        let result = secure_spike_round_trip("127.0.0.1:9".parse().unwrap(), [0_u8; 32], b"probe");
        assert!(result.is_err());
    }

    #[test]
    fn persistent_der_identity_loads_and_serves_the_same_pin() {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec![AUTHORITY_SERVER_NAME.to_string()]).unwrap();
        let root = std::env::temp_dir().join(format!("eab-quic-identity-{}", uuid::Uuid::new_v4()));
        let certificate_path = root.join("authority-cert.der");
        let private_key_path = root.join("authority-key.pk8");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&certificate_path, cert.der().as_ref()).unwrap();
        std::fs::write(&private_key_path, signing_key.serialize_der()).unwrap();

        let expected_pin = certificate_fingerprint(cert.der());
        let identity =
            QuicServerIdentity::load_pkcs8_der_files(&certificate_path, &private_key_path).unwrap();
        assert_eq!(identity.certificate_fingerprint(), expected_pin);

        let (handle, worker) = running_proactor();
        let server =
            QuicSecureServer::start_on_proactor(&handle, "127.0.0.1:0".parse().unwrap(), identity)
                .unwrap();
        assert_eq!(
            secure_spike_round_trip(server.local_addr(), expected_pin, b"persistent").unwrap(),
            [ACK_PREFIX, b"persistent"].concat()
        );

        drop(server);
        handle.stop().unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
