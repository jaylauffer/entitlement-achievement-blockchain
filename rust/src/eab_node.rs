use anyhow::Error;
use loadngo_network::{Config as NetworkConfig, MulticastConfig, Network};
use loadngo_proactor::{ChannelPort, CompletionKind, Proactor, ProactorHandle};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const EAB_WIRE_MAGIC: [u8; 4] = *b"EAB1";
const EAB_WIRE_VERSION: u16 = 1;
const EAB_MIN_COMPATIBLE_WIRE_VERSION: u16 = 1;
const DEFAULT_EAB_MULTICAST_GROUP: Ipv6Addr =
    Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0x4541, 0x4200, 0x1);
const NETWORK_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PRESENCE_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(42);
const NODE_INFO_RESPONSE_MIN_INTERVAL: Duration = Duration::from_secs(42);
const STATUS_REQUEST_MIN_INTERVAL: Duration = Duration::from_secs(42);

pub trait EabNodeStatusProvider: Send + Sync {
    fn snapshot(&self) -> NodeStatusSnapshot;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatusSnapshot {
    pub ledger_backend: String,
    pub qcoin_node_target: Option<String>,
    pub anchor_outbox_pending: usize,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NodeInfo {
    wire_version: u16,
    min_compatible_wire_version: u16,
    software_version: String,
    node_name: String,
    http_base_url: Option<String>,
    capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StatusResponse {
    node_info: NodeInfo,
    status: NodeStatusSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum WireMessage {
    PresenceAnnounce,
    NodeInfo(NodeInfo),
    StatusRequest,
    StatusResponse(StatusResponse),
}

#[derive(Debug, Clone)]
struct StartupConfig {
    bind_addr: SocketAddr,
    peers: Vec<SocketAddr>,
    multicast: Vec<MulticastConfig>,
    local_node_info: NodeInfo,
    default_multicast_enabled: bool,
}

#[derive(Debug, Default)]
struct SyncState {
    known_peers: HashSet<SocketAddr>,
    peer_node_info: HashMap<SocketAddr, NodeInfo>,
    peer_status: HashMap<SocketAddr, StatusResponse>,
    peer_last_presence_seen_at: HashMap<SocketAddr, Instant>,
    peer_last_node_info_sent_at: HashMap<SocketAddr, Instant>,
    peer_last_status_probe_at: HashMap<SocketAddr, Instant>,
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
struct EabNodeService {
    inner: Arc<EabNodeServiceInner>,
}

struct EabNodeServiceInner {
    network: Arc<Network>,
    bootstrap_targets: Vec<SocketAddr>,
    local_addrs: HashSet<SocketAddr>,
    local_node_info: NodeInfo,
    status_provider: Arc<dyn EabNodeStatusProvider>,
    handle: ProactorHandle<ChannelPort>,
    sync_state: std::sync::Mutex<SyncState>,
}

impl EabNodeRuntime {
    pub fn start_from_env(
        http_bind_ip: &str,
        http_bind_port: u16,
        status_provider: Arc<dyn EabNodeStatusProvider>,
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

        let proactor = Proactor::new(ChannelPort::new());
        let handle = proactor.handle();
        let thread_handle = thread::spawn(move || {
            if let Err(err) = proactor.run_until_stopped() {
                eprintln!("EAB node runtime stopped with error: {err}");
            }
        });

        let service = EabNodeService::start(
            network,
            startup.bind_addr,
            startup.peers,
            startup.multicast,
            startup.local_node_info,
            status_provider,
            handle.clone(),
        )?;

        Ok(Some(Self {
            _service: service,
            handle,
            thread: Some(thread_handle),
        }))
    }
}

impl EabNodeService {
    fn start(
        network: Arc<Network>,
        bind_addr: SocketAddr,
        peers: Vec<SocketAddr>,
        multicast: Vec<MulticastConfig>,
        local_node_info: NodeInfo,
        status_provider: Arc<dyn EabNodeStatusProvider>,
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
            local_node_info,
            status_provider,
            handle,
            sync_state: std::sync::Mutex::new(SyncState::default()),
        });

        EabNodeServiceInner::schedule_presence_announce(&inner, Duration::ZERO)?;
        EabNodeServiceInner::schedule_pump(&inner, Duration::ZERO, NETWORK_POLL_INTERVAL)?;

        Ok(Self { inner })
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

    fn schedule_presence_announce(this: &Arc<Self>, delay: Duration) -> Result<(), String> {
        let driver = Arc::clone(this);
        this.handle
            .defer_for(delay, CompletionKind::Net, 0, move |_| {
                if let Err(err) = driver.broadcast_presence_announces() {
                    eprintln!("EAB presence announce failed: {err}");
                }
                if driver.handle.is_running() {
                    let _ = Self::schedule_presence_announce(&driver, PRESENCE_ANNOUNCE_INTERVAL);
                }
            })
            .map_err(|err| format!("failed to schedule EAB presence announce: {err}"))
    }

    fn drain_and_report(self: &Arc<Self>) {
        if let Err(err) = self.drain_frames() {
            eprintln!("EAB UDP dispatch failed: {err}");
        }
    }

    fn drain_frames(&self) -> Result<usize, String> {
        let mut buf = [0u8; 64 * 1024];
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

        let message = match decode_wire_message(frame) {
            Ok(message) => message,
            Err(err) => {
                eprintln!("Discarding invalid EAB UDP frame from {source}: {err}");
                return Ok(());
            }
        };

        match message {
            WireMessage::PresenceAnnounce => self.handle_presence_announce(source),
            WireMessage::NodeInfo(node_info) => self.handle_node_info(source, node_info),
            WireMessage::StatusRequest => self.handle_status_request(source),
            WireMessage::StatusResponse(response) => self.handle_status_response(source, response),
        }
    }

    fn handle_presence_announce(&self, source: SocketAddr) -> Result<(), String> {
        let should_reply = {
            let mut sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
            let now = Instant::now();
            sync_state.known_peers.insert(source);
            sync_state.peer_last_presence_seen_at.insert(source, now);
            sync_state
                .peer_last_node_info_sent_at
                .get(&source)
                .is_none_or(|last| now.duration_since(*last) >= NODE_INFO_RESPONSE_MIN_INTERVAL)
                .then(|| {
                    sync_state.peer_last_node_info_sent_at.insert(source, now);
                })
                .is_some()
        };

        if should_reply {
            self.send_local_node_info(source)?;
        }
        Ok(())
    }

    fn handle_node_info(&self, source: SocketAddr, node_info: NodeInfo) -> Result<(), String> {
        let (changed, should_request_status) = {
            let mut sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
            sync_state.known_peers.insert(source);
            let changed = match sync_state.peer_node_info.insert(source, node_info.clone()) {
                Some(existing) => existing != node_info,
                None => true,
            };
            let now = Instant::now();
            let should_request_status = sync_state
                .peer_last_status_probe_at
                .get(&source)
                .is_none_or(|last| now.duration_since(*last) >= STATUS_REQUEST_MIN_INTERVAL);
            if should_request_status {
                sync_state.peer_last_status_probe_at.insert(source, now);
            }
            (changed, should_request_status)
        };

        if changed {
            println!(
                "EAB node discovered {source}{}",
                node_info
                    .http_base_url
                    .as_deref()
                    .map(|url| format!(" -> {url}"))
                    .unwrap_or_default()
            );
        }

        if should_request_status {
            self.request_status(source)?;
        }

        Ok(())
    }

    fn handle_status_request(&self, source: SocketAddr) -> Result<(), String> {
        self.send_wire(
            source,
            WireMessage::StatusResponse(StatusResponse {
                node_info: self.local_node_info.clone(),
                status: self.status_provider.snapshot(),
            }),
        )
    }

    fn handle_status_response(
        &self,
        source: SocketAddr,
        response: StatusResponse,
    ) -> Result<(), String> {
        let changed = {
            let mut sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
            sync_state.known_peers.insert(source);
            sync_state
                .peer_node_info
                .insert(source, response.node_info.clone());
            match sync_state.peer_status.insert(source, response.clone()) {
                Some(existing) => existing != response,
                None => true,
            }
        };

        if changed {
            let qcoin_target = response
                .status
                .qcoin_node_target
                .as_deref()
                .unwrap_or("unconfigured");
            println!(
                "EAB node status from {source}: backend={}, qcoin_target={}, outbox_pending={}",
                response.status.ledger_backend, qcoin_target, response.status.anchor_outbox_pending
            );
        }

        Ok(())
    }

    fn broadcast_presence_announces(&self) -> Result<(), String> {
        for target in self.bootstrap_targets() {
            if let Err(err) = self.send_wire(target, WireMessage::PresenceAnnounce) {
                if self.should_ignore_bootstrap_send_error(target, &err) {
                    continue;
                }
                return Err(err);
            }
        }
        Ok(())
    }

    fn send_local_node_info(&self, target: SocketAddr) -> Result<(), String> {
        self.send_wire(target, WireMessage::NodeInfo(self.local_node_info.clone()))
    }

    fn request_status(&self, target: SocketAddr) -> Result<(), String> {
        self.send_wire(target, WireMessage::StatusRequest)
    }

    fn send_wire(&self, target: SocketAddr, message: WireMessage) -> Result<(), String> {
        let frame = encode_wire_message(&message)?;
        self.network
            .send_frame_with_retries(target, &frame)
            .map_err(|err| format!("failed to send EAB wire message to {target}: {err:#}"))?;
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

    #[cfg(test)]
    fn known_peers(&self) -> Vec<SocketAddr> {
        let sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
        let mut peers = sync_state.known_peers.iter().copied().collect::<Vec<_>>();
        peers.sort_by(|left, right| left.to_string().cmp(&right.to_string()));
        peers
    }

    #[cfg(test)]
    fn peer_status(&self, peer: SocketAddr) -> Option<StatusResponse> {
        let sync_state = self.sync_state.lock().expect("EAB sync state poisoned");
        sync_state.peer_status.get(&peer).cloned()
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
    let local_node_info = NodeInfo {
        wire_version: EAB_WIRE_VERSION,
        min_compatible_wire_version: EAB_MIN_COMPATIBLE_WIRE_VERSION,
        software_version: env!("CARGO_PKG_VERSION").to_string(),
        node_name,
        http_base_url: advertised_http_base_url(http_bind_ip, http_bind_port),
        capabilities: vec![
            "http-api".to_string(),
            "multicast-discovery".to_string(),
            "qcoin-anchor-outbox".to_string(),
        ],
    };

    Ok(StartupConfig {
        bind_addr,
        peers,
        multicast,
        local_node_info,
        default_multicast_enabled,
    })
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

fn advertised_http_base_url(http_bind_ip: &str, http_bind_port: u16) -> Option<String> {
    if let Ok(explicit) = env::var("EAB_PUBLIC_HTTP_URL") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let ip = http_bind_ip.parse::<IpAddr>().ok()?;
    if ip.is_unspecified() {
        return None;
    }

    Some(match ip {
        IpAddr::V4(addr) => format!("http://{addr}:{http_bind_port}"),
        IpAddr::V6(addr) => format!("http://[{addr}]:{http_bind_port}"),
    })
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

fn encode_wire_message(message: &WireMessage) -> Result<Vec<u8>, String> {
    let mut frame = EAB_WIRE_MAGIC.to_vec();
    let payload = bincode::serialize(message).map_err(|err| err.to_string())?;
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_wire_message(frame: &[u8]) -> Result<WireMessage, String> {
    if frame.len() < EAB_WIRE_MAGIC.len() {
        return Err("frame shorter than EAB wire header".to_string());
    }
    if frame[..EAB_WIRE_MAGIC.len()] != EAB_WIRE_MAGIC {
        return Err("frame does not match EAB wire magic".to_string());
    }
    bincode::deserialize(&frame[EAB_WIRE_MAGIC.len()..]).map_err(|err| err.to_string())
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
        decode_wire_message, discovery_targets_for, encode_wire_message,
        select_multicast_interfaces_for_bind_ip, EabNodeService, InterfaceCandidate, NodeInfo,
        NodeStatusSnapshot, StaticStatusProvider, StatusResponse, WireMessage,
        DEFAULT_EAB_MULTICAST_GROUP,
    };
    use loadngo_network::{Config as NetworkConfig, MulticastConfig, Network};
    use loadngo_proactor::{ChannelPort, Proactor};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn wait_for_handshake(service: &EabNodeService, peer: SocketAddr, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if service.inner.known_peers().contains(&peer) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for EAB node handshake with {label}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_status(service: &EabNodeService, peer: SocketAddr, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if service.inner.peer_status(peer).is_some() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for EAB node status from {label}"
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
    fn wire_round_trips_node_info() {
        let encoded = encode_wire_message(&WireMessage::NodeInfo(NodeInfo {
            wire_version: 1,
            min_compatible_wire_version: 1,
            software_version: "0.1.0".to_string(),
            node_name: "eab-node".to_string(),
            http_base_url: Some("http://127.0.0.1:8080".to_string()),
            capabilities: vec!["http-api".to_string()],
        }))
        .unwrap();

        let decoded = decode_wire_message(&encoded).unwrap();
        assert!(matches!(decoded, WireMessage::NodeInfo(_)));
    }

    #[test]
    fn wire_round_trips_status_response() {
        let encoded = encode_wire_message(&WireMessage::StatusResponse(StatusResponse {
            node_info: NodeInfo {
                wire_version: 1,
                min_compatible_wire_version: 1,
                software_version: "0.1.0".to_string(),
                node_name: "eab-node".to_string(),
                http_base_url: Some("http://127.0.0.1:8080".to_string()),
                capabilities: vec!["http-api".to_string()],
            },
            status: NodeStatusSnapshot {
                ledger_backend: "qcoin".to_string(),
                qcoin_node_target: Some("127.0.0.1:9700".to_string()),
                anchor_outbox_pending: 2,
                last_anchor_success_unix_seconds: Some(10),
                last_anchor_error: None,
                last_anchor_error_unix_seconds: None,
            },
        }))
        .unwrap();

        let decoded = decode_wire_message(&encoded).unwrap();
        assert!(matches!(decoded, WireMessage::StatusResponse(_)));
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
    fn eab_node_service_exchanges_node_info_over_presence_announce() {
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
            NodeInfo {
                wire_version: 1,
                min_compatible_wire_version: 1,
                software_version: "0.1.0".to_string(),
                node_name: "a".to_string(),
                http_base_url: Some("http://127.0.0.1:8080".to_string()),
                capabilities: vec!["http-api".to_string()],
            },
            Arc::new(StaticStatusProvider {
                snapshot: NodeStatusSnapshot {
                    ledger_backend: "qcoin".to_string(),
                    qcoin_node_target: Some("127.0.0.1:9700".to_string()),
                    anchor_outbox_pending: 1,
                    last_anchor_success_unix_seconds: Some(11),
                    last_anchor_error: None,
                    last_anchor_error_unix_seconds: None,
                },
            }),
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
            NodeInfo {
                wire_version: 1,
                min_compatible_wire_version: 1,
                software_version: "0.1.0".to_string(),
                node_name: "b".to_string(),
                http_base_url: Some("http://127.0.0.1:8081".to_string()),
                capabilities: vec!["http-api".to_string()],
            },
            Arc::new(StaticStatusProvider {
                snapshot: NodeStatusSnapshot {
                    ledger_backend: "file".to_string(),
                    qcoin_node_target: None,
                    anchor_outbox_pending: 0,
                    last_anchor_success_unix_seconds: None,
                    last_anchor_error: None,
                    last_anchor_error_unix_seconds: None,
                },
            }),
            handle_b.clone(),
        )
        .unwrap();

        wait_for_handshake(&service_a, addr_b, "node B");
        wait_for_handshake(&service_b, addr_a, "node A");
        wait_for_status(&service_a, addr_b, "node B");
        wait_for_status(&service_b, addr_a, "node A");

        assert_eq!(
            service_a
                .inner
                .peer_status(addr_b)
                .unwrap()
                .status
                .ledger_backend,
            "file"
        );
        assert_eq!(
            service_b
                .inner
                .peer_status(addr_a)
                .unwrap()
                .status
                .qcoin_node_target,
            Some("127.0.0.1:9700".to_string())
        );

        handle_a.stop().unwrap();
        handle_b.stop().unwrap();
        worker_a.join().unwrap();
        worker_b.join().unwrap();
    }
}
