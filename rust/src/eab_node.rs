use anyhow::Error;
use eab_wire::{
    DiscoveryChallenge, DiscoveryMessage, DiscoveryProbe, DiscoveryQuery, DiscoveryResponse,
    AUTHORITY_FINGERPRINT_LEN, DISCOVERY_TOKEN_LEN, DISCOVERY_WIRE_VERSION,
    MAX_DISCOVERY_DATAGRAM_LEN,
};
use loadngo_network::{Config as NetworkConfig, MulticastConfig, Network};
use loadngo_proactor::{ChannelPort, CompletionKind, Proactor, ProactorHandle};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DEFAULT_EAB_MULTICAST_GROUP: Ipv6Addr =
    Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0x4541, 0x4200, 0x1);
const NETWORK_POLL_INTERVAL: Duration = Duration::from_millis(5);
const DISCOVERY_PROBE_INTERVAL: Duration = Duration::from_secs(42);
const DISCOVERY_COOKIE_BUCKET_SECONDS: u64 = 60;
const DISCOVERY_RESPONSE_TTL_SECONDS: u64 = 120;
const MAX_DISCOVERED_AUTHORITIES: usize = 128;

pub trait EabNodeStatusProvider: Send + Sync {
    fn snapshot(&self) -> NodeStatusSnapshot;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatusSnapshot {
    pub ledger_backend: String,
    pub qcoin_node_target: Option<String>,
    pub anchor_outbox_pending: usize,
    pub anchor_outbox_pending_submission: usize,
    pub anchor_outbox_accepted_not_included: usize,
    pub last_anchor_accepted_unix_seconds: Option<u64>,
    pub last_anchor_included_unix_seconds: Option<u64>,
    pub last_anchor_success_unix_seconds: Option<u64>,
    pub last_anchor_error: Option<String>,
    pub last_anchor_error_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct StaticStatusProvider {
    snapshot: NodeStatusSnapshot,
}

impl StaticStatusProvider {
    pub fn new(ledger_backend: impl Into<String>) -> Self {
        Self {
            snapshot: NodeStatusSnapshot {
                ledger_backend: ledger_backend.into(),
                qcoin_node_target: None,
                anchor_outbox_pending: 0,
                anchor_outbox_pending_submission: 0,
                anchor_outbox_accepted_not_included: 0,
                last_anchor_accepted_unix_seconds: None,
                last_anchor_included_unix_seconds: None,
                last_anchor_success_unix_seconds: None,
                last_anchor_error: None,
                last_anchor_error_unix_seconds: None,
            },
        }
    }
}

impl EabNodeStatusProvider for StaticStatusProvider {
    fn snapshot(&self) -> NodeStatusSnapshot {
        self.snapshot.clone()
    }
}

#[derive(Debug, Clone)]
struct LocalAuthorityAdvertisement {
    node_id: String,
    quic_endpoint: String,
    authority_fingerprint: [u8; AUTHORITY_FINGERPRINT_LEN],
}

#[derive(Debug, Clone)]
struct StartupConfig {
    bind_addr: SocketAddr,
    peers: Vec<SocketAddr>,
    multicast: Vec<MulticastConfig>,
    local_authority: Option<LocalAuthorityAdvertisement>,
    trusted_authority_pins: Vec<[u8; AUTHORITY_FINGERPRINT_LEN]>,
    default_multicast_enabled: bool,
}

#[derive(Debug)]
struct ActiveProbe {
    request_id: [u8; 16],
    client_nonce: [u8; DISCOVERY_TOKEN_LEN],
    created_at: Instant,
}

#[derive(Debug, Default)]
struct SyncState {
    active_probe: Option<ActiveProbe>,
    discovered_authorities: HashMap<SocketAddr, DiscoveryResponse>,
}

struct DiscoveryCookieIssuer {
    key: [u8; 32],
}

impl DiscoveryCookieIssuer {
    fn random() -> Self {
        let mut key = [0_u8; 32];
        key[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        key[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        Self { key }
    }

    fn issue(
        &self,
        source: SocketAddr,
        request_id: &[u8; 16],
        client_nonce: &[u8; DISCOVERY_TOKEN_LEN],
        unix_seconds: u64,
    ) -> [u8; DISCOVERY_TOKEN_LEN] {
        self.issue_for_bucket(
            source,
            request_id,
            client_nonce,
            unix_seconds / DISCOVERY_COOKIE_BUCKET_SECONDS,
        )
    }

    fn validate(
        &self,
        source: SocketAddr,
        request_id: &[u8; 16],
        client_nonce: &[u8; DISCOVERY_TOKEN_LEN],
        cookie: &[u8; DISCOVERY_TOKEN_LEN],
        unix_seconds: u64,
    ) -> bool {
        let bucket = unix_seconds / DISCOVERY_COOKIE_BUCKET_SECONDS;
        [bucket, bucket.saturating_sub(1)].iter().any(|candidate| {
            constant_time_equal(
                &self.issue_for_bucket(source, request_id, client_nonce, *candidate),
                cookie,
            )
        })
    }

    fn issue_for_bucket(
        &self,
        source: SocketAddr,
        request_id: &[u8; 16],
        client_nonce: &[u8; DISCOVERY_TOKEN_LEN],
        bucket: u64,
    ) -> [u8; DISCOVERY_TOKEN_LEN] {
        let mut input = b"eab-discovery-cookie\0".to_vec();
        input.extend_from_slice(source.to_string().as_bytes());
        input.extend_from_slice(request_id);
        input.extend_from_slice(client_nonce);
        input.extend_from_slice(&bucket.to_be_bytes());
        let digest = blake3::keyed_hash(&self.key, &input);
        let mut cookie = [0_u8; DISCOVERY_TOKEN_LEN];
        cookie.copy_from_slice(&digest.as_bytes()[..DISCOVERY_TOKEN_LEN]);
        cookie
    }
}

pub struct EabNodeRuntime {
    _service: EabNodeService,
    handle: ProactorHandle<ChannelPort>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for EabNodeRuntime {
    fn drop(&mut self) {
        let _ = self.handle.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[allow(dead_code)]
pub struct EabNodeService {
    inner: Arc<EabNodeServiceInner>,
}

struct EabNodeServiceInner {
    network: Arc<Network>,
    bootstrap_targets: Vec<SocketAddr>,
    local_addrs: HashSet<SocketAddr>,
    local_authority: Option<LocalAuthorityAdvertisement>,
    trusted_authority_pins: Vec<[u8; AUTHORITY_FINGERPRINT_LEN]>,
    cookie_issuer: DiscoveryCookieIssuer,
    handle: ProactorHandle<ChannelPort>,
    sync_state: std::sync::Mutex<SyncState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedAuthorityEndpoint {
    pub discovery_source: SocketAddr,
    pub node_id: String,
    pub quic_endpoint: String,
    pub authority_fingerprint: [u8; AUTHORITY_FINGERPRINT_LEN],
}

impl EabNodeRuntime {
    pub fn start_from_env(http_bind_ip: &str, http_bind_port: u16) -> Result<Option<Self>, String> {
        if env_flag("EAB_NODE_DISABLE") {
            return Ok(None);
        }

        let proactor = Proactor::new(ChannelPort::new());
        let handle = proactor.handle();
        let thread_handle = thread::spawn(move || {
            if let Err(err) = proactor.run_until_stopped() {
                eprintln!("EAB node runtime stopped with error: {err}");
            }
        });

        let service =
            EabNodeService::start_from_env_on_handle(http_bind_ip, http_bind_port, handle.clone())?
                .ok_or_else(|| "EAB node service unexpectedly disabled".to_string())?;

        Ok(Some(Self {
            _service: service,
            handle,
            thread: Some(thread_handle),
        }))
    }
}

impl EabNodeService {
    pub fn start_from_env_on_handle(
        http_bind_ip: &str,
        http_bind_port: u16,
        handle: ProactorHandle<ChannelPort>,
    ) -> Result<Option<Self>, String> {
        if env_flag("EAB_NODE_DISABLE") {
            return Ok(None);
        }

        let startup = resolve_startup_config(http_bind_ip, http_bind_port)?;
        let network = Arc::new(build_network(
            startup.bind_addr,
            &startup.peers,
            &startup.multicast,
        )?);
        let local_addrs = network
            .local_addrs()
            .map_err(|err| format!("failed to inspect EAB UDP local addrs: {err:#}"))?
            .into_iter()
            .collect::<HashSet<_>>();
        println!(
            "EAB node transport listening on {:?}",
            local_addrs.iter().copied().collect::<Vec<_>>()
        );
        if startup.default_multicast_enabled {
            println!(
                "EAB node transport using embedded IPv6 multicast group {}",
                DEFAULT_EAB_MULTICAST_GROUP
            );
        }

        let service = EabNodeService::start(
            network,
            startup.bind_addr,
            startup.peers,
            startup.multicast,
            startup.local_authority,
            startup.trusted_authority_pins,
            handle.clone(),
        )?;

        Ok(Some(service))
    }

    fn start(
        network: Arc<Network>,
        bind_addr: SocketAddr,
        peers: Vec<SocketAddr>,
        multicast: Vec<MulticastConfig>,
        local_authority: Option<LocalAuthorityAdvertisement>,
        trusted_authority_pins: Vec<[u8; AUTHORITY_FINGERPRINT_LEN]>,
        handle: ProactorHandle<ChannelPort>,
    ) -> Result<Self, String> {
        let bootstrap_targets = discovery_targets_for(bind_addr, &peers, &multicast);
        let local_addrs = network
            .local_addrs()
            .map_err(|err| format!("failed to inspect EAB UDP local addrs: {err:#}"))?
            .into_iter()
            .collect::<HashSet<_>>();

        let inner = Arc::new(EabNodeServiceInner {
            network,
            bootstrap_targets,
            local_addrs,
            local_authority,
            trusted_authority_pins,
            cookie_issuer: DiscoveryCookieIssuer::random(),
            handle,
            sync_state: std::sync::Mutex::new(SyncState::default()),
        });

        EabNodeServiceInner::schedule_discovery_probe(&inner, Duration::ZERO)?;
        EabNodeServiceInner::schedule_pump(&inner, Duration::ZERO, NETWORK_POLL_INTERVAL)?;

        Ok(Self { inner })
    }

    /// Selects one unexpired discovery candidate whose certificate fingerprint
    /// is explicitly pinned in configuration.
    pub fn selected_trusted_authority(&self) -> Option<TrustedAuthorityEndpoint> {
        self.inner.selected_trusted_authority(unix_seconds_now())
    }
}

impl EabNodeServiceInner {
    fn schedule_pump(
        this: &Arc<Self>,
        delay: Duration,
        idle_interval: Duration,
    ) -> Result<(), String> {
        let driver = Arc::clone(this);
        this.handle
            .defer_for(delay, CompletionKind::Net, 0, move |_| {
                driver.drain_and_report();
                if driver.handle.is_running() {
                    let _ = Self::schedule_pump(&driver, idle_interval, idle_interval);
                }
            })
            .map_err(|err| format!("failed to schedule EAB node pump: {err}"))
    }

    fn schedule_discovery_probe(this: &Arc<Self>, delay: Duration) -> Result<(), String> {
        let driver = Arc::clone(this);
        this.handle
            .defer_for(delay, CompletionKind::Net, 0, move |_| {
                if let Err(err) = driver.broadcast_discovery_probe() {
                    eprintln!("EAB discovery probe failed: {err}");
                }
                if driver.handle.is_running() {
                    let _ = Self::schedule_discovery_probe(&driver, DISCOVERY_PROBE_INTERVAL);
                }
            })
            .map_err(|err| format!("failed to schedule EAB discovery probe: {err}"))
    }

    fn drain_and_report(self: &Arc<Self>) {
        if let Err(err) = self.drain_frames() {
            eprintln!("EAB UDP dispatch failed: {err}");
        }
    }

    fn drain_frames(&self) -> Result<usize, String> {
        let mut buf = [0u8; MAX_DISCOVERY_DATAGRAM_LEN + 1];
        let mut handled = 0usize;
        loop {
            match self.network.recv_frame(&mut buf) {
                Ok((len, source)) => {
                    handled += 1;
                    self.handle_frame(source, &buf[..len])?;
                }
                Err(err) if is_would_block(&err) => return Ok(handled),
                Err(err) => return Err(format!("failed to receive EAB UDP frame: {err:#}")),
            }
        }
    }

    fn handle_frame(&self, source: SocketAddr, frame: &[u8]) -> Result<(), String> {
        if self.is_local_source(source) {
            return Ok(());
        }

        let message = match DiscoveryMessage::decode(frame) {
            Ok(message) => message,
            Err(err) => {
                eprintln!("Discarding invalid EAB UDP frame from {source}: {err}");
                return Ok(());
            }
        };

        match message {
            DiscoveryMessage::Probe(probe) => self.handle_discovery_probe(source, probe),
            DiscoveryMessage::Challenge(challenge) => {
                self.handle_discovery_challenge(source, challenge)
            }
            DiscoveryMessage::Query(query) => self.handle_discovery_query(source, query),
            DiscoveryMessage::Response(response) => {
                self.handle_discovery_response(source, response)
            }
        }
    }

    fn handle_discovery_probe(
        &self,
        source: SocketAddr,
        probe: DiscoveryProbe,
    ) -> Result<(), String> {
        if self.local_authority.is_none()
            || !version_ranges_overlap(
                probe.min_wire_version,
                probe.max_wire_version,
                DISCOVERY_WIRE_VERSION,
                DISCOVERY_WIRE_VERSION,
            )
        {
            return Ok(());
        }

        let cookie = self.cookie_issuer.issue(
            source,
            &probe.request_id,
            &probe.client_nonce,
            unix_seconds_now(),
        );
        self.send_discovery(
            source,
            DiscoveryMessage::Challenge(DiscoveryChallenge {
                request_id: probe.request_id,
                cookie,
            }),
        )
    }

    fn handle_discovery_challenge(
        &self,
        source: SocketAddr,
        challenge: DiscoveryChallenge,
    ) -> Result<(), String> {
        let client_nonce = {
            let sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
            match &sync_state.active_probe {
                Some(active)
                    if active.request_id == challenge.request_id
                        && active.created_at.elapsed() <= DISCOVERY_PROBE_INTERVAL =>
                {
                    active.client_nonce
                }
                _ => return Ok(()),
            }
        };

        self.send_discovery(
            source,
            DiscoveryMessage::Query(DiscoveryQuery {
                request_id: challenge.request_id,
                client_nonce,
                cookie: challenge.cookie,
            }),
        )
    }

    fn handle_discovery_query(
        &self,
        source: SocketAddr,
        query: DiscoveryQuery,
    ) -> Result<(), String> {
        let authority = match &self.local_authority {
            Some(authority) => authority,
            None => return Ok(()),
        };
        let now = unix_seconds_now();
        if !self.cookie_issuer.validate(
            source,
            &query.request_id,
            &query.client_nonce,
            &query.cookie,
            now,
        ) {
            return Ok(());
        }

        self.send_discovery(
            source,
            DiscoveryMessage::Response(DiscoveryResponse {
                request_id: query.request_id,
                node_id: authority.node_id.clone(),
                quic_endpoint: authority.quic_endpoint.clone(),
                authority_fingerprint: authority.authority_fingerprint,
                min_wire_version: DISCOVERY_WIRE_VERSION,
                max_wire_version: DISCOVERY_WIRE_VERSION,
                capabilities: Vec::new(),
                expires_at_unix_seconds: now + DISCOVERY_RESPONSE_TTL_SECONDS,
            }),
        )
    }

    fn handle_discovery_response(
        &self,
        source: SocketAddr,
        response: DiscoveryResponse,
    ) -> Result<(), String> {
        let now = unix_seconds_now();
        if response.expires_at_unix_seconds <= now
            || !version_ranges_overlap(
                response.min_wire_version,
                response.max_wire_version,
                DISCOVERY_WIRE_VERSION,
                DISCOVERY_WIRE_VERSION,
            )
        {
            return Ok(());
        }

        let changed = {
            let mut sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
            let matches_active_probe = sync_state
                .active_probe
                .as_ref()
                .is_some_and(|active| active.request_id == response.request_id);
            if !matches_active_probe {
                return Ok(());
            }

            sync_state
                .discovered_authorities
                .retain(|_, known| known.expires_at_unix_seconds > now);
            if !sync_state.discovered_authorities.contains_key(&source)
                && sync_state.discovered_authorities.len() >= MAX_DISCOVERED_AUTHORITIES
            {
                return Ok(());
            }

            match sync_state
                .discovered_authorities
                .insert(source, response.clone())
            {
                Some(existing) => existing != response,
                None => true,
            }
        };

        if changed {
            println!(
                "EAB authority candidate discovered at {source}: node={}, secure_endpoint={}",
                response.node_id, response.quic_endpoint
            );
        }
        Ok(())
    }

    fn broadcast_discovery_probe(&self) -> Result<(), String> {
        let active_probe = ActiveProbe {
            request_id: *Uuid::new_v4().as_bytes(),
            client_nonce: *Uuid::new_v4().as_bytes(),
            created_at: Instant::now(),
        };
        let message = DiscoveryMessage::Probe(DiscoveryProbe {
            request_id: active_probe.request_id,
            client_nonce: active_probe.client_nonce,
            min_wire_version: DISCOVERY_WIRE_VERSION,
            max_wire_version: DISCOVERY_WIRE_VERSION,
        });
        {
            let mut sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
            sync_state.active_probe = Some(active_probe);
            let now = unix_seconds_now();
            sync_state
                .discovered_authorities
                .retain(|_, known| known.expires_at_unix_seconds > now);
        }

        for target in self.bootstrap_targets() {
            if let Err(err) = self.send_discovery(target, message.clone()) {
                if self.should_ignore_bootstrap_send_error(target, &err) {
                    continue;
                }
                return Err(err);
            }
        }
        Ok(())
    }

    fn send_discovery(&self, target: SocketAddr, message: DiscoveryMessage) -> Result<(), String> {
        let frame = message.encode().map_err(|err| err.to_string())?;
        self.network
            .send_frame_with_retries(target, &frame)
            .map_err(|err| format!("failed to send EAB discovery message to {target}: {err:#}"))?;
        Ok(())
    }

    fn bootstrap_targets(&self) -> Vec<SocketAddr> {
        self.bootstrap_targets.clone()
    }

    fn is_local_source(&self, source: SocketAddr) -> bool {
        self.local_addrs.contains(&source)
    }

    fn should_ignore_bootstrap_send_error(&self, target: SocketAddr, err: &str) -> bool {
        target.ip().is_multicast()
            && [
                "No route to host",
                "Network is unreachable",
                "Host is unreachable",
            ]
            .iter()
            .any(|needle| err.contains(needle))
    }

    fn selected_trusted_authority(&self, now: u64) -> Option<TrustedAuthorityEndpoint> {
        let sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
        select_trusted_authority(
            &sync_state.discovered_authorities,
            &self.trusted_authority_pins,
            now,
        )
    }

    #[cfg(test)]
    fn known_authorities(&self) -> Vec<(SocketAddr, DiscoveryResponse)> {
        let sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
        let mut authorities = sync_state
            .discovered_authorities
            .iter()
            .map(|(source, response)| (*source, response.clone()))
            .collect::<Vec<_>>();
        authorities.sort_by_key(|entry| entry.0.to_string());
        authorities
    }
}

fn resolve_startup_config(
    http_bind_ip: &str,
    http_bind_port: u16,
) -> Result<StartupConfig, String> {
    let bind_addr = resolve_node_bind_addr(http_bind_ip, http_bind_port)?;
    let peers = resolve_peer_addrs(&env::var("EAB_NODE_PEERS").unwrap_or_default(), bind_addr)?;
    let (multicast, default_multicast_enabled) = resolve_multicast_configs(bind_addr)?;
    let node_name = env::var("EAB_NODE_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "eab-node".to_string());
    let local_authority = resolve_local_authority(node_name)?;
    let trusted_authority_pins = resolve_authority_fingerprint_list(
        &env::var("EAB_TRUSTED_AUTHORITY_FINGERPRINTS").unwrap_or_default(),
    )?;

    Ok(StartupConfig {
        bind_addr,
        peers,
        multicast,
        local_authority,
        trusted_authority_pins,
        default_multicast_enabled,
    })
}

fn resolve_authority_fingerprint_list(
    spec: &str,
) -> Result<Vec<[u8; AUTHORITY_FINGERPRINT_LEN]>, String> {
    let mut pins = Vec::new();
    for encoded in spec
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let bytes = hex::decode(encoded)
            .map_err(|err| format!("invalid trusted authority fingerprint {encoded}: {err}"))?;
        let pin: [u8; AUTHORITY_FINGERPRINT_LEN] = bytes.try_into().map_err(|_| {
            format!(
                "trusted authority fingerprint must encode exactly {AUTHORITY_FINGERPRINT_LEN} bytes"
            )
        })?;
        if pin.iter().all(|byte| *byte == 0) {
            return Err("trusted authority fingerprint must be non-zero".to_string());
        }
        if !pins.contains(&pin) {
            pins.push(pin);
        }
    }
    Ok(pins)
}

fn select_trusted_authority(
    candidates: &HashMap<SocketAddr, DiscoveryResponse>,
    pins: &[[u8; AUTHORITY_FINGERPRINT_LEN]],
    now: u64,
) -> Option<TrustedAuthorityEndpoint> {
    let mut eligible = candidates
        .iter()
        .filter_map(|(source, response)| {
            let pin_rank = pins
                .iter()
                .position(|pin| pin == &response.authority_fingerprint)?;
            if response.expires_at_unix_seconds <= now
                || !version_ranges_overlap(
                    response.min_wire_version,
                    response.max_wire_version,
                    DISCOVERY_WIRE_VERSION,
                    DISCOVERY_WIRE_VERSION,
                )
            {
                return None;
            }
            Some((
                pin_rank,
                response.node_id.as_str(),
                response.quic_endpoint.as_str(),
                source.to_string(),
                TrustedAuthorityEndpoint {
                    discovery_source: *source,
                    node_id: response.node_id.clone(),
                    quic_endpoint: response.quic_endpoint.clone(),
                    authority_fingerprint: response.authority_fingerprint,
                },
            ))
        })
        .collect::<Vec<_>>();

    eligible.sort_by(|left, right| {
        (&left.0, left.1, left.2, &left.3).cmp(&(&right.0, right.1, right.2, &right.3))
    });
    eligible
        .into_iter()
        .next()
        .map(|(_, _, _, _, selected)| selected)
}

fn resolve_node_bind_addr(http_bind_ip: &str, http_bind_port: u16) -> Result<SocketAddr, String> {
    if let Ok(explicit) = env::var("EAB_NODE_BIND") {
        if !explicit.trim().is_empty() {
            return resolve_socket_addr(&explicit);
        }
    }

    let port = env::var("EAB_NODE_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(http_bind_port);

    if let Ok(ip) = http_bind_ip.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }

    resolve_socket_addr(&format!("{http_bind_ip}:{port}"))
}

fn resolve_local_authority(node_id: String) -> Result<Option<LocalAuthorityAdvertisement>, String> {
    let endpoint = env::var("EAB_QUIC_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let fingerprint = env::var("EAB_AUTHORITY_FINGERPRINT_HEX")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    match (endpoint, fingerprint) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(
            "EAB_QUIC_ENDPOINT requires EAB_AUTHORITY_FINGERPRINT_HEX; refusing partial authority advertisement"
                .to_string(),
        ),
        (None, Some(_)) => Err(
            "EAB_AUTHORITY_FINGERPRINT_HEX requires EAB_QUIC_ENDPOINT; refusing partial authority advertisement"
                .to_string(),
        ),
        (Some(quic_endpoint), Some(encoded_fingerprint)) => {
            let bytes = hex::decode(&encoded_fingerprint)
                .map_err(|err| format!("invalid EAB_AUTHORITY_FINGERPRINT_HEX: {err}"))?;
            let authority_fingerprint: [u8; AUTHORITY_FINGERPRINT_LEN] = bytes
                .try_into()
                .map_err(|_| {
                    format!(
                        "EAB_AUTHORITY_FINGERPRINT_HEX must encode exactly {AUTHORITY_FINGERPRINT_LEN} bytes"
                    )
                })?;
            let advertisement = LocalAuthorityAdvertisement {
                node_id,
                quic_endpoint,
                authority_fingerprint,
            };
            validate_local_authority(&advertisement)?;
            Ok(Some(advertisement))
        }
    }
}

fn validate_local_authority(authority: &LocalAuthorityAdvertisement) -> Result<(), String> {
    DiscoveryMessage::Response(DiscoveryResponse {
        request_id: [1; 16],
        node_id: authority.node_id.clone(),
        quic_endpoint: authority.quic_endpoint.clone(),
        authority_fingerprint: authority.authority_fingerprint,
        min_wire_version: DISCOVERY_WIRE_VERSION,
        max_wire_version: DISCOVERY_WIRE_VERSION,
        capabilities: Vec::new(),
        expires_at_unix_seconds: 1,
    })
    .validate()
    .map_err(|err| format!("invalid local EAB authority advertisement: {err}"))
}

fn resolve_peer_addrs(spec: &str, self_bind_addr: SocketAddr) -> Result<Vec<SocketAddr>, String> {
    let mut resolved = Vec::new();
    for raw in spec
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let addr = resolve_socket_addr(raw)?;
        if addr != self_bind_addr && !resolved.contains(&addr) {
            resolved.push(addr);
        }
    }
    Ok(resolved)
}

fn resolve_socket_addr(spec: &str) -> Result<SocketAddr, String> {
    let mut addrs = spec
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve socket address {spec}: {err}"))?;
    addrs
        .next()
        .ok_or_else(|| format!("socket address {spec} did not resolve"))
}

fn resolve_multicast_configs(
    bind_addr: SocketAddr,
) -> Result<(Vec<MulticastConfig>, bool), String> {
    if env_flag("EAB_DISABLE_DEFAULT_MULTICAST") {
        return Ok((Vec::new(), false));
    }

    let group = env::var("EAB_MULTICAST_V6_GROUP")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .parse::<Ipv6Addr>()
                .map_err(|err| format!("invalid EAB_MULTICAST_V6_GROUP: {err}"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_EAB_MULTICAST_GROUP);

    let interfaces = if let Ok(explicit) = env::var("EAB_MULTICAST_V6_INTERFACE") {
        let trimmed = explicit.trim();
        if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed
                .parse::<u32>()
                .map_err(|err| format!("invalid EAB_MULTICAST_V6_INTERFACE: {err}"))?]
        }
    } else {
        Vec::new()
    };

    let interfaces = if interfaces.is_empty() {
        match resolve_ipv6_multicast_interfaces(bind_addr) {
            Ok(indices) => indices,
            Err(err) => {
                eprintln!("Default EAB multicast bootstrap disabled: {err}");
                return Ok((Vec::new(), true));
            }
        }
    } else {
        interfaces
    };

    let mut multicast = Vec::new();
    for interface in interfaces {
        let resolved = MulticastConfig::V6 { group, interface };
        if !multicast.contains(&resolved) {
            multicast.push(resolved);
        }
    }
    Ok((multicast, true))
}

fn discovery_targets_for(
    bind_addr: SocketAddr,
    peers: &[SocketAddr],
    multicast: &[MulticastConfig],
) -> Vec<SocketAddr> {
    let mut targets = Vec::new();
    for target in peers
        .iter()
        .copied()
        .chain(multicast_targets(bind_addr, multicast))
    {
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

fn multicast_targets(bind_addr: SocketAddr, multicast: &[MulticastConfig]) -> Vec<SocketAddr> {
    multicast
        .iter()
        .map(|entry| match *entry {
            MulticastConfig::V4 { group, .. } => {
                SocketAddr::V4(SocketAddrV4::new(group, bind_addr.port()))
            }
            MulticastConfig::V6 { group, interface } => {
                SocketAddr::V6(SocketAddrV6::new(group, bind_addr.port(), 0, interface))
            }
        })
        .collect()
}

fn build_network(
    bind_addr: SocketAddr,
    peers: &[SocketAddr],
    multicast: &[MulticastConfig],
) -> Result<Network, String> {
    let need_v4 = bind_addr.is_ipv4()
        || peers.iter().any(|peer| peer.is_ipv4())
        || multicast
            .iter()
            .any(|entry| matches!(entry, MulticastConfig::V4 { .. }));
    let need_v6 = bind_addr.is_ipv6()
        || peers.iter().any(|peer| peer.is_ipv6())
        || multicast
            .iter()
            .any(|entry| matches!(entry, MulticastConfig::V6 { .. }));
    let mut config = NetworkConfig {
        bind_addr,
        multicast: multicast.to_vec(),
        timeout: Duration::from_millis(200),
        retries: 3,
        ..NetworkConfig::default()
    };

    if need_v4 && need_v6 {
        if bind_addr.is_ipv4() {
            config
                .extra_bind_addrs
                .push(SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::UNSPECIFIED,
                    bind_addr.port(),
                    0,
                    0,
                )));
        } else {
            config
                .extra_bind_addrs
                .push(SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::UNSPECIFIED,
                    bind_addr.port(),
                )));
        }
    }

    let mut network = Network::with_config(config);
    network
        .init()
        .map_err(|err| format!("failed to initialize EAB UDP transport: {err:#}"))?;
    Ok(network)
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn version_ranges_overlap(left_min: u16, left_max: u16, right_min: u16, right_max: u16) -> bool {
    left_min <= right_max && right_min <= left_max
}

fn constant_time_equal<const N: usize>(left: &[u8; N], right: &[u8; N]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn is_would_block(err: &Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .map(|io_err| {
            io_err.kind() == ErrorKind::WouldBlock || io_err.kind() == ErrorKind::TimedOut
        })
        .unwrap_or(false)
}

fn resolve_ipv6_multicast_interfaces(bind_addr: SocketAddr) -> Result<Vec<u32>, String> {
    if let SocketAddr::V6(addr) = bind_addr {
        if addr.scope_id() != 0 {
            return Ok(vec![addr.scope_id()]);
        }
    }
    #[cfg(unix)]
    if let Some(indices) = discover_ipv6_multicast_interfaces_for_bind_addr(bind_addr)? {
        return Ok(indices);
    }
    discover_ipv6_multicast_interfaces()
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InterfaceCandidate {
    index: u32,
    ip: IpAddr,
    is_loopback: bool,
}

#[cfg(unix)]
fn discover_ipv6_multicast_interfaces_for_bind_addr(
    bind_addr: SocketAddr,
) -> Result<Option<Vec<u32>>, String> {
    let bind_ip = match bind_addr {
        SocketAddr::V4(addr) if !addr.ip().is_unspecified() => IpAddr::V4(*addr.ip()),
        SocketAddr::V6(addr) if !addr.ip().is_unspecified() => IpAddr::V6(*addr.ip()),
        _ => return Ok(None),
    };

    let candidates = discover_multicast_interface_candidates()?;
    let preferred = select_multicast_interfaces_for_bind_ip(bind_ip, &candidates);
    if preferred.is_empty() {
        Ok(None)
    } else {
        Ok(Some(preferred))
    }
}

#[cfg(unix)]
fn select_multicast_interfaces_for_bind_ip(
    bind_ip: IpAddr,
    candidates: &[InterfaceCandidate],
) -> Vec<u32> {
    let mut non_loopback = Vec::new();
    let mut loopback = Vec::new();

    for candidate in candidates {
        if candidate.ip != bind_ip {
            continue;
        }
        let target = if candidate.is_loopback {
            &mut loopback
        } else {
            &mut non_loopback
        };
        if !target.contains(&candidate.index) {
            target.push(candidate.index);
        }
    }

    if !non_loopback.is_empty() {
        return non_loopback;
    }
    loopback
}

#[cfg(unix)]
fn discover_ipv6_multicast_interfaces() -> Result<Vec<u32>, String> {
    let candidates = discover_multicast_interface_candidates()?;
    let mut non_loopback = Vec::new();
    let mut loopback = Vec::new();

    for candidate in candidates {
        if !matches!(candidate.ip, IpAddr::V6(_)) {
            continue;
        }
        let target = if candidate.is_loopback {
            &mut loopback
        } else {
            &mut non_loopback
        };
        if !target.contains(&candidate.index) {
            target.push(candidate.index);
        }
    }

    if !non_loopback.is_empty() {
        return Ok(non_loopback);
    }
    if !loopback.is_empty() {
        return Ok(loopback);
    }
    Err("no IPv6 multicast-capable interfaces found".to_string())
}

#[cfg(unix)]
fn discover_multicast_interface_candidates() -> Result<Vec<InterfaceCandidate>, String> {
    use std::ptr;

    let mut head = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(format!(
            "getifaddrs failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut candidates = Vec::new();
    let mut current = head;
    while !current.is_null() {
        let entry = unsafe { &*current };
        if !entry.ifa_addr.is_null() {
            let flags = entry.ifa_flags as i32;
            let is_up = (flags & libc::IFF_UP) != 0;
            let supports_multicast = (flags & libc::IFF_MULTICAST) != 0;
            if is_up && supports_multicast {
                let family = unsafe { (*entry.ifa_addr).sa_family as i32 };
                let ip = match family {
                    libc::AF_INET => {
                        let sockaddr = unsafe { *(entry.ifa_addr as *const libc::sockaddr_in) };
                        IpAddr::V4(Ipv4Addr::from(u32::from_be(sockaddr.sin_addr.s_addr)))
                    }
                    libc::AF_INET6 => {
                        let sockaddr = unsafe { *(entry.ifa_addr as *const libc::sockaddr_in6) };
                        IpAddr::V6(Ipv6Addr::from(sockaddr.sin6_addr.s6_addr))
                    }
                    _ => {
                        current = entry.ifa_next;
                        continue;
                    }
                };

                let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }
                    .to_string_lossy()
                    .into_owned();
                let index = unsafe { libc::if_nametoindex(entry.ifa_name) };
                if index != 0 {
                    candidates.push(InterfaceCandidate {
                        index,
                        ip,
                        is_loopback: (flags & libc::IFF_LOOPBACK) != 0 || name == "lo0",
                    });
                }
            }
        }
        current = entry.ifa_next;
    }

    unsafe { libc::freeifaddrs(head) };
    Ok(candidates)
}

#[cfg(not(unix))]
fn discover_ipv6_multicast_interfaces() -> Result<Vec<u32>, String> {
    Err(
        "automatic IPv6 multicast interface discovery is only implemented on Unix hosts"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        constant_time_equal, discovery_targets_for, resolve_authority_fingerprint_list,
        select_multicast_interfaces_for_bind_ip, select_trusted_authority, DiscoveryCookieIssuer,
        EabNodeService, InterfaceCandidate, LocalAuthorityAdvertisement,
        DEFAULT_EAB_MULTICAST_GROUP, DISCOVERY_COOKIE_BUCKET_SECONDS,
    };
    use eab_wire::DiscoveryResponse;
    use loadngo_network::{Config as NetworkConfig, MulticastConfig, Network};
    use loadngo_proactor::{ChannelPort, Proactor};
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_for_authority(service: &EabNodeService, peer: SocketAddr, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if service
                .inner
                .known_authorities()
                .iter()
                .any(|(source, _)| *source == peer)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for EAB authority discovery from {label}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn build_network(bind_addr: SocketAddr, peers: &[SocketAddr]) -> Network {
        let need_v4 = bind_addr.is_ipv4() || peers.iter().any(|peer| peer.is_ipv4());
        let need_v6 = bind_addr.is_ipv6() || peers.iter().any(|peer| peer.is_ipv6());
        let mut config = NetworkConfig {
            bind_addr,
            timeout: Duration::from_millis(200),
            retries: 3,
            ..NetworkConfig::default()
        };
        if need_v4 && need_v6 {
            if bind_addr.is_ipv4() {
                config
                    .extra_bind_addrs
                    .push(SocketAddr::V6(SocketAddrV6::new(
                        Ipv6Addr::UNSPECIFIED,
                        bind_addr.port(),
                        0,
                        0,
                    )));
            } else {
                config
                    .extra_bind_addrs
                    .push(SocketAddr::V4(SocketAddrV4::new(
                        Ipv4Addr::UNSPECIFIED,
                        bind_addr.port(),
                    )));
            }
        }
        let mut network = Network::with_config(config);
        network.init().unwrap();
        network
    }

    #[test]
    fn discovery_cookie_is_bound_to_source_request_nonce_and_recent_time() {
        let issuer = DiscoveryCookieIssuer { key: [0x42; 32] };
        let source: SocketAddr = "127.0.0.1:41000".parse().unwrap();
        let other_source: SocketAddr = "127.0.0.1:41001".parse().unwrap();
        let request_id = [1; 16];
        let client_nonce = [2; 16];
        let now = DISCOVERY_COOKIE_BUCKET_SECONDS * 20;
        let cookie = issuer.issue(source, &request_id, &client_nonce, now);

        assert!(issuer.validate(source, &request_id, &client_nonce, &cookie, now));
        assert!(issuer.validate(
            source,
            &request_id,
            &client_nonce,
            &cookie,
            now + DISCOVERY_COOKIE_BUCKET_SECONDS
        ));
        assert!(!issuer.validate(
            source,
            &request_id,
            &client_nonce,
            &cookie,
            now + (2 * DISCOVERY_COOKIE_BUCKET_SECONDS)
        ));
        assert!(!issuer.validate(other_source, &request_id, &client_nonce, &cookie, now));
        assert!(!issuer.validate(source, &[3; 16], &client_nonce, &cookie, now));
        assert!(!issuer.validate(source, &request_id, &[4; 16], &cookie, now));
        assert!(constant_time_equal(&cookie, &cookie));
    }

    #[test]
    fn trusted_authority_selection_filters_pins_and_is_deterministic() {
        let preferred_source: SocketAddr = "127.0.0.1:42001".parse().unwrap();
        let fallback_source: SocketAddr = "127.0.0.1:42002".parse().unwrap();
        let untrusted_source: SocketAddr = "127.0.0.1:42003".parse().unwrap();
        let expired_source: SocketAddr = "127.0.0.1:42004".parse().unwrap();
        let incompatible_source: SocketAddr = "127.0.0.1:42005".parse().unwrap();
        let mut candidates = HashMap::new();
        candidates.insert(
            fallback_source,
            discovery_response("fallback", "127.0.0.1:4543", [0xbb; 32], 200),
        );
        candidates.insert(
            preferred_source,
            discovery_response("preferred", "127.0.0.1:4542", [0xaa; 32], 200),
        );
        candidates.insert(
            untrusted_source,
            discovery_response("untrusted", "127.0.0.1:4544", [0xcc; 32], 200),
        );
        candidates.insert(
            expired_source,
            discovery_response("expired", "127.0.0.1:4545", [0xaa; 32], 99),
        );
        let mut incompatible =
            discovery_response("incompatible", "127.0.0.1:4546", [0xaa; 32], 200);
        incompatible.min_wire_version = 3;
        incompatible.max_wire_version = 3;
        candidates.insert(incompatible_source, incompatible);

        let selected = select_trusted_authority(&candidates, &[[0xaa; 32], [0xbb; 32]], 100)
            .expect("a trusted authority should be selected");
        assert_eq!(selected.discovery_source, preferred_source);
        assert_eq!(selected.node_id, "preferred");

        let selected = select_trusted_authority(&candidates, &[[0xbb; 32], [0xaa; 32]], 100)
            .expect("pin order should define preference");
        assert_eq!(selected.discovery_source, fallback_source);

        assert!(select_trusted_authority(&candidates, &[[0xdd; 32]], 100).is_none());
        assert!(select_trusted_authority(&candidates, &[], 100).is_none());
    }

    #[test]
    fn trusted_authority_pin_configuration_is_bounded_and_fail_closed() {
        let pin_a = "11".repeat(32);
        let pin_b = "22".repeat(32);
        let parsed =
            resolve_authority_fingerprint_list(&format!("{pin_a}, {pin_b}\n{pin_a}")).unwrap();
        assert_eq!(parsed, vec![[0x11; 32], [0x22; 32]]);

        assert!(resolve_authority_fingerprint_list("").unwrap().is_empty());
        assert!(resolve_authority_fingerprint_list("00").is_err());
        assert!(resolve_authority_fingerprint_list(&"00".repeat(32)).is_err());
        assert!(resolve_authority_fingerprint_list("not-hex").is_err());
    }

    fn discovery_response(
        node_id: &str,
        endpoint: &str,
        fingerprint: [u8; 32],
        expiry: u64,
    ) -> DiscoveryResponse {
        DiscoveryResponse {
            request_id: [1; 16],
            node_id: node_id.to_string(),
            quic_endpoint: endpoint.to_string(),
            authority_fingerprint: fingerprint,
            min_wire_version: 2,
            max_wire_version: 2,
            capabilities: Vec::new(),
            expires_at_unix_seconds: expiry,
        }
    }

    #[test]
    fn discovery_targets_include_ipv6_multicast_group() {
        let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080));
        let peer = "127.0.0.1:9800".parse().unwrap();
        let multicast = [MulticastConfig::V6 {
            group: DEFAULT_EAB_MULTICAST_GROUP,
            interface: 7,
        }];

        let targets = discovery_targets_for(bind_addr, &[peer], &multicast);
        assert!(targets.contains(&peer));
        assert!(targets.contains(&SocketAddr::V6(SocketAddrV6::new(
            DEFAULT_EAB_MULTICAST_GROUP,
            8080,
            0,
            7,
        ))));
    }

    #[cfg(unix)]
    #[test]
    fn bind_ip_prefers_matching_non_loopback_multicast_interface() {
        let selected = select_multicast_interfaces_for_bind_ip(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 146)),
            &[
                InterfaceCandidate {
                    index: 7,
                    ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
                    is_loopback: false,
                },
                InterfaceCandidate {
                    index: 16,
                    ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 146)),
                    is_loopback: false,
                },
                InterfaceCandidate {
                    index: 20,
                    ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
                    is_loopback: false,
                },
            ],
        );

        assert_eq!(selected, vec![16]);
    }

    #[test]
    fn eab_node_service_completes_bounded_discovery_cookie_exchange() {
        let network_a = Arc::new(build_network(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &[],
        ));
        let addr_a = network_a.local_addr().unwrap();

        let network_b = Arc::new(build_network(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &[addr_a],
        ));
        let addr_b = network_b.local_addr().unwrap();

        let proactor_a = Proactor::new(ChannelPort::new());
        let handle_a = proactor_a.handle();
        let worker_a = thread::spawn(move || proactor_a.run_until_stopped().unwrap());
        let service_a = EabNodeService::start(
            Arc::clone(&network_a),
            addr_a,
            vec![addr_b],
            Vec::new(),
            Some(LocalAuthorityAdvertisement {
                node_id: "authority-a".to_string(),
                quic_endpoint: "127.0.0.1:4542".to_string(),
                authority_fingerprint: [0xaa; 32],
            }),
            vec![[0xbb; 32]],
            handle_a.clone(),
        )
        .unwrap();

        let proactor_b = Proactor::new(ChannelPort::new());
        let handle_b = proactor_b.handle();
        let worker_b = thread::spawn(move || proactor_b.run_until_stopped().unwrap());
        let service_b = EabNodeService::start(
            Arc::clone(&network_b),
            addr_b,
            vec![addr_a],
            Vec::new(),
            Some(LocalAuthorityAdvertisement {
                node_id: "authority-b".to_string(),
                quic_endpoint: "127.0.0.1:4543".to_string(),
                authority_fingerprint: [0xbb; 32],
            }),
            vec![[0xaa; 32]],
            handle_b.clone(),
        )
        .unwrap();

        wait_for_authority(&service_a, addr_b, "authority B");
        wait_for_authority(&service_b, addr_a, "authority A");

        let authority_b = service_a.inner.known_authorities().remove(0).1;
        assert_eq!(authority_b.node_id, "authority-b");
        assert_eq!(authority_b.quic_endpoint, "127.0.0.1:4543");
        assert_eq!(authority_b.authority_fingerprint, [0xbb; 32]);
        assert_eq!(
            service_a
                .selected_trusted_authority()
                .expect("authority B should match configured pin")
                .node_id,
            "authority-b"
        );

        handle_a.stop().unwrap();
        handle_b.stop().unwrap();
        worker_a.join().unwrap();
        worker_b.join().unwrap();
    }

    #[test]
    fn discovery_only_client_does_not_advertise_a_nonexistent_secure_service() {
        let network_a = Arc::new(build_network(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &[],
        ));
        let addr_a = network_a.local_addr().unwrap();

        let network_b = Arc::new(build_network(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &[addr_a],
        ));
        let addr_b = network_b.local_addr().unwrap();

        let proactor_a = Proactor::new(ChannelPort::new());
        let handle_a = proactor_a.handle();
        let worker_a = thread::spawn(move || proactor_a.run_until_stopped().unwrap());
        let service_a = EabNodeService::start(
            Arc::clone(&network_a),
            addr_a,
            vec![addr_b],
            Vec::new(),
            Some(LocalAuthorityAdvertisement {
                node_id: "authority".to_string(),
                quic_endpoint: "127.0.0.1:4542".to_string(),
                authority_fingerprint: [0xaa; 32],
            }),
            Vec::new(),
            handle_a.clone(),
        )
        .unwrap();

        let proactor_b = Proactor::new(ChannelPort::new());
        let handle_b = proactor_b.handle();
        let worker_b = thread::spawn(move || proactor_b.run_until_stopped().unwrap());
        let service_b = EabNodeService::start(
            Arc::clone(&network_b),
            addr_b,
            vec![addr_a],
            Vec::new(),
            None,
            vec![[0xaa; 32]],
            handle_b.clone(),
        )
        .unwrap();

        wait_for_authority(&service_b, addr_a, "authority node");
        thread::sleep(Duration::from_millis(50));
        assert!(service_a.inner.known_authorities().is_empty());

        handle_a.stop().unwrap();
        handle_b.stop().unwrap();
        worker_a.join().unwrap();
        worker_b.join().unwrap();
    }
}
