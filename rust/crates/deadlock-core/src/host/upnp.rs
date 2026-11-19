//! UPnP IGD port mappings for the hosted server (ADR 0015, second amendment).
//!
//! An opt-in switch on the Host page asks the home router to forward the game port (TCP and
//! UDP) and the map download port (TCP) to this PC while the server runs, and to remove those
//! mappings again when it stops. This is the Internet Gateway Device protocol:
//!
//! 1. SSDP discovery: one `M-SEARCH` datagram to `239.255.255.250:1900` (multicast TTL 2, so it
//!    never leaves the local network) and the routers' replies, which carry a `LOCATION` URL.
//! 2. The device description at that URL (plain HTTP on the LAN), for the `WANIPConnection` or
//!    `WANPPPConnection` service and its control URL.
//! 3. SOAP calls to that control URL: `AddPortMapping`, `DeletePortMapping` and
//!    `GetExternalIPAddress`.
//!
//! Safety: a reply can only point Sigil at the device that sent it. `LOCATION` and the control
//! URL must be `http://` on an IPv4 literal equal to the replying address, and that address must
//! be private (10/8, 172.16/12, 192.168/16), link-local or loopback. Hostnames, https, public
//! addresses and other hosts are skipped without a connection. Datagrams over 2 KiB and HTTP
//! answers over 64 KiB are refused. Every socket has a short timeout. Nothing is logged.
//!
//! Std only: no UPnP crate. The messages are small and fixed, so minimal HTTP and string
//! scanning of the XML are enough.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The SSDP multicast group and port routers listen on.
pub const SSDP_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
/// How long [`open_ports`] waits for a router to answer the search.
pub const DISCOVER_TIMEOUT: Duration = Duration::from_secs(3);
/// Lease asked for each mapping, in seconds. A mapping Sigil could not remove (a crash, a power
/// cut) expires on its own after this. Routers that only take permanent leases get 0.
pub const LEASE_SECONDS: u32 = 43_200;

const SEARCH_TARGETS: [&str; 2] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
];
/// Services that can map ports, best first.
const SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];
const MAX_DATAGRAM: usize = 2048;
const MAX_RESPONSE: usize = 64 * 1024;
const MAX_URL: usize = 512;
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const HTTP_TOTAL: Duration = Duration::from_secs(6);
/// Release (also from Drop) waits less: the server is stopping and the app may be closing.
const RELEASE_TIMEOUT: Duration = Duration::from_millis(1500);

const NO_ROUTER: &str = "No router answered the UPnP search. UPnP may be off on your router. Turn it on there, or forward the ports by hand.";
const OUTSIDE: &str = "A device answered the UPnP search with an address outside your local network. Sigil did not contact it.";

/// A transport protocol of one mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortProtocol {
    Tcp,
    Udp,
}

impl PortProtocol {
    /// `TCP` or `UDP`, as the router expects it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

/// One port to forward to this PC: the same port number outside and inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortRequest {
    pub protocol: PortProtocol,
    pub port: u16,
}

impl PortRequest {
    pub fn tcp(port: u16) -> Self {
        Self { protocol: PortProtocol::Tcp, port }
    }

    pub fn udp(port: u16) -> Self {
        Self { protocol: PortProtocol::Udp, port }
    }
}

/// A router that can map ports, found on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gateway {
    /// Host and port of the control URL. Always a local-network address.
    pub router: SocketAddrV4,
    /// Path of the control URL.
    pub control_path: String,
    /// The service the calls go to (`WANIPConnection` or `WANPPPConnection`).
    pub service_type: String,
    /// This PC's address on the way to the router: where the router forwards to.
    pub local_ip: Ipv4Addr,
}

/// What the router said about one port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingResult {
    pub protocol: PortProtocol,
    pub port: u16,
    /// None when it worked; otherwise a sentence for the player.
    pub error: Option<String>,
}

impl MappingResult {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Ports the router forwards to this PC until [`UpnpLease::release`] or drop.
#[derive(Debug)]
pub struct UpnpLease {
    gateway: Gateway,
    mapped: Vec<PortRequest>,
    /// One entry per requested port, in order.
    pub results: Vec<MappingResult>,
    /// The router's internet address, when it told us. Only shown to the host.
    pub external_ip: Option<Ipv4Addr>,
}

impl UpnpLease {
    /// The ports still forwarded.
    pub fn mapped(&self) -> Vec<PortRequest> {
        self.mapped.clone()
    }

    pub fn any_mapped(&self) -> bool {
        !self.mapped.is_empty()
    }

    pub fn gateway(&self) -> &Gateway {
        &self.gateway
    }

    /// Remove every mapping still in place. A second call does nothing and returns nothing.
    /// A mapping the router no longer has counts as removed.
    pub fn release(&mut self) -> Vec<MappingResult> {
        let mapped = std::mem::take(&mut self.mapped);
        mapped
            .into_iter()
            .map(|request| {
                let error = match delete_mapping(&self.gateway, request, RELEASE_TIMEOUT) {
                    Ok(()) | Err(SoapError::Fault { code: 714, .. }) => None,
                    Err(error) => Some(error.message(request.port)),
                };
                MappingResult { protocol: request.protocol, port: request.port, error }
            })
            .collect()
    }
}

impl Drop for UpnpLease {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

/// Search the local network for a router that maps ports.
pub fn discover(timeout: Duration) -> Result<Gateway> {
    discover_at(SocketAddr::V4(SSDP_ADDR), timeout)
}

/// [`discover`], with the search sent to `search` (tests aim it at a fake router on localhost).
pub fn discover_at(search: SocketAddr, timeout: Duration) -> Result<Gateway> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|e| Error::msg(format!("Sigil could not open a network socket for the UPnP search ({e}).")))?;
    let _ = socket.set_multicast_ttl_v4(2);
    for target in SEARCH_TARGETS {
        let message = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n"
        );
        socket
            .send_to(message.as_bytes(), search)
            .map_err(|e| Error::msg(format!("The UPnP search could not be sent ({e}).")))?;
    }
    let deadline = Instant::now() + timeout;
    let mut buffer = vec![0u8; MAX_DATAGRAM + 1];
    let mut tried: Vec<String> = Vec::new();
    let mut last_error: Option<Error> = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = socket.set_read_timeout(Some(left.max(Duration::from_millis(1))));
        let (length, from) = match socket.recv_from(&mut buffer) {
            Ok(received) => received,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            // Windows reports an ICMP "port unreachable" as a reset on the next read.
            Err(_) => continue,
        };
        if length > MAX_DATAGRAM {
            continue;
        }
        let SocketAddr::V4(from) = from else { continue };
        let Ok(text) = std::str::from_utf8(&buffer[..length]) else { continue };
        let Some(location) = ssdp_location(text) else { continue };
        if tried.contains(&location) {
            continue;
        }
        tried.push(location.clone());
        let Some(target) = location_target(&location, *from.ip()) else {
            last_error = Some(Error::msg(OUTSIDE));
            continue;
        };
        match gateway_from(target) {
            Ok(gateway) => return Ok(gateway),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| Error::msg(NO_ROUTER)))
}

/// Find the router and forward `ports` to this PC. The player's switch on the Host page and
/// their Start press are the approval.
pub fn open_ports(ports: &[PortRequest], description: &str, approve: bool) -> Result<UpnpLease> {
    super::require_approval(approve, "Opening router ports")?;
    if ports.is_empty() {
        return Err(Error::msg("There are no ports to open."));
    }
    let gateway = discover(DISCOVER_TIMEOUT)?;
    open_ports_on(gateway, ports, description)
}

/// Forward `ports` through a router already found. Each port is tried; the lease holds the ones
/// that worked. An error when none did, with the first reason.
pub fn open_ports_on(gateway: Gateway, ports: &[PortRequest], description: &str) -> Result<UpnpLease> {
    let description = clean_description(description);
    let mut lease = UpnpLease { gateway, mapped: Vec::new(), results: Vec::new(), external_ip: None };
    for &request in ports {
        let mut outcome = add_mapping(&lease.gateway, request, &description, LEASE_SECONDS);
        if matches!(outcome, Err(SoapError::Fault { code: 725, .. })) {
            outcome = add_mapping(&lease.gateway, request, &description, 0);
        }
        let error = match outcome {
            Ok(()) => {
                lease.mapped.push(request);
                None
            }
            Err(error) => Some(error.message(request.port)),
        };
        lease.results.push(MappingResult { protocol: request.protocol, port: request.port, error });
    }
    if lease.mapped.is_empty() {
        let reason = lease
            .results
            .iter()
            .find_map(|r| r.error.clone())
            .unwrap_or_else(|| "The router did not open any port.".to_string());
        return Err(Error::msg(reason));
    }
    lease.external_ip = external_ip(&lease.gateway);
    Ok(lease)
}

// ---------------------------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------------------------

/// An address on the local network: private, link-local or loopback.
fn local_network(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_loopback()
}

/// `http://<ipv4>[:port][/path]` to (address, path). Anything else is None.
fn parse_http_url(url: &str) -> Option<(SocketAddrV4, String)> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let scheme_end = url.find("://")?;
    if !url[..scheme_end].eq_ignore_ascii_case("http") {
        return None;
    }
    let rest = &url[scheme_end + 3..];
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()?),
        None => (authority, 80),
    };
    let ip: Ipv4Addr = host.parse().ok()?;
    if port == 0 {
        return None;
    }
    let path = path.split('#').next().unwrap_or("/");
    let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    Some((SocketAddrV4::new(ip, port), path))
}

/// The description URL a reply from `responder` may be fetched at: http, an IPv4 literal equal
/// to the responder, on the local network.
fn location_target(location: &str, responder: Ipv4Addr) -> Option<(SocketAddrV4, String)> {
    let (addr, path) = parse_http_url(location)?;
    (*addr.ip() == responder && local_network(responder)).then_some((addr, path))
}

/// Resolve a URL from a description against `base`. An absolute one must be on the same host.
fn resolve(base: &(SocketAddrV4, String), reference: &str) -> Option<(SocketAddrV4, String)> {
    let reference = reference.trim();
    if reference.is_empty() || reference.len() > MAX_URL || reference.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    if reference.contains("://") {
        let (addr, path) = parse_http_url(reference)?;
        return (addr.ip() == base.0.ip()).then_some((addr, path));
    }
    if reference.starts_with("//") {
        return None;
    }
    if reference.starts_with('/') {
        return Some((base.0, reference.to_string()));
    }
    let base_path = base.1.split(['?', '#']).next().unwrap_or("/");
    let dir = &base_path[..base_path.rfind('/').map_or(0, |at| at + 1)];
    let dir = if dir.is_empty() { "/" } else { dir };
    Some((base.0, format!("{dir}{reference}")))
}

// ---------------------------------------------------------------------------------------------
// SSDP and the description
// ---------------------------------------------------------------------------------------------

/// The `LOCATION` of an SSDP search reply, when the reply is from a gateway.
fn ssdp_location(text: &str) -> Option<String> {
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let status = lines.next()?.trim().to_ascii_uppercase();
    if !(status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200")) {
        return None;
    }
    let mut location = None;
    let mut search_target = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "location" => location = Some(value.to_string()),
            "st" => search_target = Some(value.to_ascii_lowercase()),
            _ => {}
        }
    }
    if let Some(st) = search_target {
        let gateway = ["internetgatewaydevice", "wanipconnection", "wanpppconnection"].iter().any(|k| st.contains(k));
        if !gateway {
            return None;
        }
    }
    location.filter(|l| !l.is_empty() && l.len() <= MAX_URL)
}

/// Read the description and pick the service that maps ports.
fn gateway_from(location: (SocketAddrV4, String)) -> Result<Gateway> {
    let answer = http_request(location.0, "GET", &location.1, &[], b"", HTTP_TIMEOUT)?;
    if answer.status != 200 {
        return Err(Error::msg(format!(
            "The router's UPnP description could not be read (HTTP {}).",
            answer.status
        )));
    }
    let xml = String::from_utf8_lossy(&answer.body);
    let base = match elements(&xml, "URLBase").first().map(|b| unescape(b)) {
        Some(url_base) if !url_base.trim().is_empty() => {
            resolve(&location, &url_base).ok_or_else(|| Error::msg(OUTSIDE))?
        }
        _ => location,
    };
    let (service_type, control) = pick_service(&xml)
        .ok_or_else(|| Error::msg("The router answered, but it offers no port forwarding over UPnP."))?;
    let (router, control_path) = resolve(&base, &control).ok_or_else(|| Error::msg(OUTSIDE))?;
    if !local_network(*router.ip()) {
        return Err(Error::msg(OUTSIDE));
    }
    let local_ip = match answer.local {
        SocketAddr::V4(local) => *local.ip(),
        SocketAddr::V6(local) => local
            .ip()
            .to_ipv4_mapped()
            .ok_or_else(|| Error::msg("Sigil could not tell this PC's local address."))?,
    };
    Ok(Gateway { router, control_path, service_type, local_ip })
}

/// The best port-mapping service in a description: (service type, control URL).
fn pick_service(xml: &str) -> Option<(String, String)> {
    let services: Vec<(String, String)> = elements(xml, "service")
        .into_iter()
        .filter_map(|block| {
            let service_type = unescape(elements(block, "serviceType").first()?).trim().to_string();
            let control = unescape(elements(block, "controlURL").first()?).trim().to_string();
            Some((service_type, control))
        })
        .collect();
    SERVICES
        .iter()
        .find_map(|wanted| services.iter().find(|(t, c)| t == wanted && !c.is_empty()).cloned())
}

/// The inner text of every `<name>` element (any namespace prefix), in order. Elements of the
/// same name do not nest in the documents read here.
fn elements<'a>(xml: &'a str, local: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = xml[at..].find('<') {
        let start = at + offset;
        at = start + 1;
        let rest = &xml[start + 1..];
        let name_len = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len());
        let name = &rest[..name_len];
        if name.is_empty() || name.starts_with(['?', '!']) {
            continue;
        }
        if name.rsplit(':').next() != Some(local) {
            continue;
        }
        let Some(tag_end) = rest.find('>') else { break };
        let open_end = start + 1 + tag_end + 1;
        if rest[..tag_end].ends_with('/') {
            found.push("");
            at = open_end;
            continue;
        }
        if let Some(close) = closing_tag(&xml[open_end..], name) {
            found.push(&xml[open_end..open_end + close]);
            at = open_end + close;
        }
    }
    found
}

/// Offset of `</name>` (whitespace allowed before `>`), never a longer name like `</nameList>`.
fn closing_tag(text: &str, name: &str) -> Option<usize> {
    let needle = format!("</{name}");
    let mut from = 0;
    while let Some(offset) = text[from..].find(&needle) {
        let at = from + offset;
        let after = text[at + needle.len()..].trim_start();
        if after.starts_with('>') {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Printable ASCII, at most 64 characters: some routers refuse anything longer or stranger.
fn clean_description(text: &str) -> String {
    let cleaned: String = text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(64).collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() { "Sigil".to_string() } else { cleaned }
}

// ---------------------------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------------------------

struct Answer {
    status: u16,
    body: Vec<u8>,
    /// This PC's end of the connection.
    local: SocketAddr,
}

fn http_request(
    addr: SocketAddrV4,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> Result<Answer> {
    let unreachable = |e: std::io::Error| Error::msg(format!("The router at {} did not answer ({e}).", addr.ip()));
    let mut stream = TcpStream::connect_timeout(&SocketAddr::V4(addr), timeout).map_err(unreachable)?;
    stream.set_read_timeout(Some(timeout)).map_err(unreachable)?;
    stream.set_write_timeout(Some(timeout)).map_err(unreachable)?;
    let local = stream.local_addr().map_err(unreachable)?;
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nUser-Agent: Sigil UPnP/1.0\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if method != "GET" {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(body);
    stream.write_all(&bytes).map_err(unreachable)?;
    let (status, body) = read_response(&mut stream)?;
    Ok(Answer { status, body, local })
}

fn read_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>)> {
    let deadline = Instant::now() + HTTP_TOTAL;
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        if data.len() > MAX_RESPONSE {
            return Err(Error::msg("The router's answer was too large. Sigil stopped reading it."));
        }
        if let Some(done) = parse_response(&data, false)? {
            return Ok(done);
        }
        if Instant::now() > deadline {
            return Err(Error::msg("The router took too long to answer."));
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => data.extend_from_slice(&buffer[..read]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(Error::msg("The router stopped answering.")),
        }
    }
    if data.len() > MAX_RESPONSE {
        return Err(Error::msg("The router's answer was too large. Sigil stopped reading it."));
    }
    parse_response(&data, true)?.ok_or_else(|| Error::msg("The router sent an incomplete answer."))
}

/// Status and body when `data` holds a whole answer. At EOF a body without a length ends there.
fn parse_response(data: &[u8], eof: bool) -> Result<Option<(u16, Vec<u8>)>> {
    let Some(head_end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if eof { Err(Error::msg("The router sent a broken answer.")) } else { Ok(None) };
    };
    let head = String::from_utf8_lossy(&data[..head_end]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .strip_prefix("HTTP/1.")
        .and_then(|rest| rest.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| Error::msg("The router sent a broken answer."))?;
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse::<usize>().ok(),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    let body = &data[head_end + 4..];
    if chunked {
        return match dechunk(body) {
            Some(decoded) => Ok(Some((status, decoded))),
            None if eof => Err(Error::msg("The router sent an incomplete answer.")),
            None => Ok(None),
        };
    }
    match length {
        Some(length) if body.len() >= length => Ok(Some((status, body[..length].to_vec()))),
        Some(_) if eof => Err(Error::msg("The router sent an incomplete answer.")),
        Some(_) => Ok(None),
        None if eof => Ok(Some((status, body.to_vec()))),
        None => Ok(None),
    }
}

/// A chunked body, once its last chunk arrived.
fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&body[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        if body.len() < size + 2 || size > MAX_RESPONSE {
            return None;
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

// ---------------------------------------------------------------------------------------------
// SOAP
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
enum SoapError {
    /// A UPnP error the router reported.
    Fault { code: u32, description: String },
    Other(String),
}

impl SoapError {
    fn message(&self, port: u16) -> String {
        match self {
            Self::Fault { code: 718, .. } => {
                format!("Port {port} is already forwarded to another device on your router.")
            }
            Self::Fault { code: 606, .. } => format!(
                "The router does not let programs open port {port} (UPnP error 606). Allow UPnP port changes in its settings, or forward the port by hand."
            ),
            Self::Fault { code, description } if description.is_empty() => {
                format!("The router refused port {port} (UPnP error {code}).")
            }
            Self::Fault { code, description } => {
                format!("The router refused port {port} (UPnP error {code}: {description}).")
            }
            Self::Other(message) => message.clone(),
        }
    }
}

fn soap(gateway: &Gateway, action: &str, args: &[(&str, String)], timeout: Duration) -> std::result::Result<String, SoapError> {
    let mut arguments = String::new();
    for (name, value) in args {
        arguments.push_str(&format!("<{name}>{}</{name}>", escape(value)));
    }
    let envelope = format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{arguments}</u:{action}></s:Body></s:Envelope>\r\n",
        escape(&gateway.service_type)
    );
    let soap_action = format!("\"{}#{action}\"", gateway.service_type);
    let answer = http_request(
        gateway.router,
        "POST",
        &gateway.control_path,
        &[("Content-Type", "text/xml; charset=\"utf-8\""), ("SOAPAction", &soap_action)],
        envelope.as_bytes(),
        timeout,
    )
    .map_err(|e| SoapError::Other(e.to_string()))?;
    let body = String::from_utf8_lossy(&answer.body).into_owned();
    if answer.status == 200 {
        return Ok(body);
    }
    match elements(&body, "errorCode").first().and_then(|c| c.trim().parse::<u32>().ok()) {
        Some(code) => {
            let description: String = elements(&body, "errorDescription")
                .first()
                .map(|d| unescape(d))
                .unwrap_or_default()
                .chars()
                .filter(|c| c.is_ascii_graphic() || *c == ' ')
                .take(80)
                .collect();
            Err(SoapError::Fault { code, description: description.trim().to_string() })
        }
        None => Err(SoapError::Other(format!("The router answered HTTP {}.", answer.status))),
    }
}

fn add_mapping(gateway: &Gateway, request: PortRequest, description: &str, lease: u32) -> std::result::Result<(), SoapError> {
    soap(
        gateway,
        "AddPortMapping",
        &[
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", request.port.to_string()),
            ("NewProtocol", request.protocol.as_str().to_string()),
            ("NewInternalPort", request.port.to_string()),
            ("NewInternalClient", gateway.local_ip.to_string()),
            ("NewEnabled", "1".to_string()),
            ("NewPortMappingDescription", description.to_string()),
            ("NewLeaseDuration", lease.to_string()),
        ],
        HTTP_TIMEOUT,
    )
    .map(|_| ())
}

fn delete_mapping(gateway: &Gateway, request: PortRequest, timeout: Duration) -> std::result::Result<(), SoapError> {
    soap(
        gateway,
        "DeletePortMapping",
        &[
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", request.port.to_string()),
            ("NewProtocol", request.protocol.as_str().to_string()),
        ],
        timeout,
    )
    .map(|_| ())
}

/// The router's internet address, when it says. Any failure is None.
fn external_ip(gateway: &Gateway) -> Option<Ipv4Addr> {
    let body = soap(gateway, "GetExternalIPAddress", &[], HTTP_TIMEOUT).ok()?;
    let ip: Ipv4Addr = unescape(elements(&body, "NewExternalIPAddress").first()?).trim().parse().ok()?;
    (!ip.is_unspecified()).then_some(ip)
}

/// For callers that hold an [`IpAddr`]: true when it is on the local network.
pub fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => local_network(ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some_and(local_network),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    const IP1: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";
    const IP2: &str = "urn:schemas-upnp-org:service:WANIPConnection:2";
    const PPP1: &str = "urn:schemas-upnp-org:service:WANPPPConnection:1";

    #[derive(Clone, Default)]
    struct Igd {
        description: String,
        chunked: bool,
        huge: bool,
        /// (protocol, port, UPnP error code) to refuse.
        faults: Vec<(String, u16, u32)>,
        permanent_only: bool,
        external_ip: String,
    }

    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        path: String,
        action: String,
        body: String,
    }

    impl Seen {
        fn arg(&self, name: &str) -> String {
            elements(&self.body, name).first().map(|s| s.to_string()).unwrap_or_default()
        }
    }

    struct Fake {
        port: u16,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Fake {
        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        fn actions(&self, action: &str) -> Vec<Seen> {
            self.seen().into_iter().filter(|s| s.action == action).collect()
        }

        fn location(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }
    }

    fn description(services: &[(&str, &str)]) -> String {
        let mut list = String::new();
        for (kind, control) in services {
            list.push_str(&format!(
                "<service>\n  <serviceType>{kind}</serviceType>\n  <serviceId>urn:upnp-org:serviceId:x</serviceId>\n  <controlURL> {control} </controlURL>\n  <eventSubURL>/evt</eventSubURL>\n</service>\n"
            ));
        }
        format!(
            "<?xml version=\"1.0\"?>\n<root xmlns=\"urn:schemas-upnp-org:device-1-0\">\n<device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>\n<serviceList><service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/l3f</controlURL></service></serviceList>\n<deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType><deviceList><device><deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>\n<serviceList>\n{list}</serviceList></device></deviceList></device></deviceList></device></root>\n"
        )
    }

    fn fault(code: u32, text: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>{text}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"
        )
    }

    /// A fake router's HTTP side on 127.0.0.1. `make` gets the port, for absolute URLs.
    fn start_http(make: impl FnOnce(u16) -> Igd) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let igd = make(port);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                handle(stream, &igd, &record);
            }
        });
        Fake { port, seen }
    }

    fn handle(mut stream: TcpStream, igd: &Igd, seen: &Mutex<Vec<Seen>>) {
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(n) => data.extend_from_slice(&buffer[..n]),
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap().split(' ');
        let method = first.next().unwrap().to_string();
        let path = first.next().unwrap().to_string();
        let mut length = 0;
        let mut action = String::new();
        for line in lines {
            let (name, value) = line.split_once(':').unwrap();
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap(),
                "soapaction" => action = value.trim().to_string(),
                _ => {}
            }
        }
        while data.len() < head_end + 4 + length {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&buffer[..n]),
            }
        }
        let body = String::from_utf8_lossy(&data[head_end + 4..]).into_owned();
        let short = action.trim_matches('"').rsplit('#').next().unwrap_or_default().to_string();
        let record = Seen { method: method.clone(), path, action: short.clone(), body };
        seen.lock().unwrap().push(record.clone());
        if !action.is_empty() {
            assert!(action.starts_with('"') && action.ends_with('"'), "SOAPAction is quoted: {action}");
        }

        let (status, reply, chunked) = if method == "GET" {
            if igd.huge {
                (200, "a".repeat(70_000), false)
            } else {
                (200, igd.description.clone(), igd.chunked)
            }
        } else {
            match short.as_str() {
                "AddPortMapping" => {
                    let protocol = record.arg("NewProtocol");
                    let port: u16 = record.arg("NewExternalPort").parse().unwrap();
                    let lease = record.arg("NewLeaseDuration");
                    if let Some((_, _, code)) = igd.faults.iter().find(|(p, n, _)| *p == protocol && *n == port) {
                        (500, fault(*code, "Refused"), false)
                    } else if igd.permanent_only && lease != "0" {
                        (500, fault(725, "OnlyPermanentLeasesSupported"), false)
                    } else {
                        (200, "<s:Envelope><s:Body><u:AddPortMappingResponse/></s:Body></s:Envelope>".to_string(), false)
                    }
                }
                "GetExternalIPAddress" => (
                    200,
                    format!("<s:Envelope><s:Body><u:GetExternalIPAddressResponse><NewExternalIPAddress>{}</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>", igd.external_ip),
                    false,
                ),
                _ => (200, "<s:Envelope><s:Body/></s:Envelope>".to_string(), false),
            }
        };
        let mut out = format!("HTTP/1.1 {status} X\r\nContent-Type: text/xml\r\nConnection: close\r\n");
        if chunked {
            out.push_str("Transfer-Encoding: chunked\r\n\r\n");
            for piece in reply.as_bytes().chunks(100) {
                out.push_str(&format!("{:x};ext=1\r\n{}\r\n", piece.len(), String::from_utf8_lossy(piece)));
            }
            out.push_str("0\r\n\r\n");
        } else {
            out.push_str(&format!("Content-Length: {}\r\n\r\n{reply}", reply.len()));
        }
        let _ = stream.write_all(out.as_bytes());
    }

    /// A fake router's SSDP side: answers each M-SEARCH with `replies`, in order.
    fn start_ssdp(replies: Vec<Vec<u8>>) -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while let Ok((n, from)) = socket.recv_from(&mut buffer) {
                if !buffer[..n].starts_with(b"M-SEARCH * HTTP/1.1\r\n") {
                    continue;
                }
                for reply in &replies {
                    let _ = socket.send_to(reply, from);
                }
            }
        });
        addr
    }

    fn reply(location: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nst: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nUSN: uuid:x\r\nLocation: {location}\r\n\r\n"
        )
        .into_bytes()
    }

    fn gateway_for(fake: &Fake, path: &str) -> Gateway {
        let ssdp = start_ssdp(vec![reply(&fake.location(path))]);
        discover_at(ssdp, Duration::from_secs(2)).unwrap()
    }

    fn standard() -> Fake {
        start_http(|_| Igd {
            description: description(&[(IP1, "/ctl")]),
            external_ip: "203.0.113.7".into(),
            ..Igd::default()
        })
    }

    #[test]
    fn discovery_finds_the_router_and_its_best_service() {
        // WANPPPConnection is listed first but WANIPConnection wins; the control URL is relative
        // to the description's folder.
        let fake = start_http(|_| Igd { description: description(&[(PPP1, "/ppp"), (IP1, "ctl/ip")]), ..Igd::default() });
        let gateway = gateway_for(&fake, "/igd/desc.xml");
        assert_eq!(gateway.service_type, IP1);
        assert_eq!(gateway.control_path, "/igd/ctl/ip");
        assert_eq!(gateway.router, SocketAddrV4::new(Ipv4Addr::LOCALHOST, fake.port));
        assert_eq!(gateway.local_ip, Ipv4Addr::LOCALHOST);
        assert_eq!(fake.seen()[0].method, "GET");
        assert_eq!(fake.seen()[0].path, "/igd/desc.xml");

        // Version 2 beats version 1; an absolute control URL on the same host is taken as is.
        let fake = start_http(|port| Igd {
            description: description(&[(IP1, "/one"), (IP2, &format!("http://127.0.0.1:{port}/two?x=1&amp;y=2"))]),
            ..Igd::default()
        });
        let gateway = gateway_for(&fake, "/d.xml");
        assert_eq!(gateway.service_type, IP2);
        assert_eq!(gateway.control_path, "/two?x=1&y=2");

        // Only a PPP connection: that one.
        let fake = start_http(|_| Igd { description: description(&[(PPP1, "/ppp")]), ..Igd::default() });
        assert_eq!(gateway_for(&fake, "/d.xml").service_type, PPP1);
    }

    #[test]
    fn a_chunked_description_parses() {
        let fake = start_http(|_| Igd { description: description(&[(IP1, "/ctl")]), chunked: true, ..Igd::default() });
        assert_eq!(gateway_for(&fake, "/d.xml").control_path, "/ctl");
    }

    #[test]
    fn ports_are_mapped_with_the_expected_request_and_released_once() {
        let fake = standard();
        let gateway = gateway_for(&fake, "/d.xml");
        let ports = [PortRequest::tcp(27067), PortRequest::udp(27067), PortRequest::tcp(27068)];
        let mut lease = open_ports_on(gateway, &ports, "Sigil & <Deadworks>\n27067").unwrap();
        assert!(lease.results.iter().all(MappingResult::ok));
        assert_eq!(lease.mapped(), ports.to_vec());
        assert!(lease.any_mapped());
        assert_eq!(lease.external_ip, Some(Ipv4Addr::new(203, 0, 113, 7)));

        let adds = fake.actions("AddPortMapping");
        assert_eq!(adds.len(), 3);
        let first = &adds[0];
        assert_eq!(first.method, "POST");
        assert_eq!(first.path, "/ctl");
        assert_eq!(first.arg("NewRemoteHost"), "");
        assert_eq!(first.arg("NewExternalPort"), "27067");
        assert_eq!(first.arg("NewInternalPort"), "27067");
        assert_eq!(first.arg("NewProtocol"), "TCP");
        assert_eq!(first.arg("NewInternalClient"), "127.0.0.1");
        assert_eq!(first.arg("NewEnabled"), "1");
        assert_eq!(first.arg("NewLeaseDuration"), "43200");
        assert_eq!(first.arg("NewPortMappingDescription"), "Sigil &amp; &lt;Deadworks&gt;27067");
        assert!(first.body.contains(&format!("xmlns:u=\"{IP1}\"")));
        assert_eq!(adds[1].arg("NewProtocol"), "UDP");
        assert_eq!(adds[2].arg("NewExternalPort"), "27068");

        let released = lease.release();
        assert_eq!(released.len(), 3);
        assert!(released.iter().all(MappingResult::ok));
        assert!(!lease.any_mapped());
        assert!(lease.release().is_empty(), "a second release does nothing");
        drop(lease);
        let deletes = fake.actions("DeletePortMapping");
        assert_eq!(deletes.len(), 3, "each mapping removed once, drop adds none");
        assert_eq!(deletes[1].arg("NewProtocol"), "UDP");
        assert_eq!(deletes[1].arg("NewExternalPort"), "27067");

        // Dropping a lease removes its mappings.
        let gateway = gateway_for(&fake, "/d.xml");
        let lease = open_ports_on(gateway, &[PortRequest::tcp(27080)], "Sigil").unwrap();
        drop(lease);
        let deletes = fake.actions("DeletePortMapping");
        assert_eq!(deletes.len(), 4);
        assert_eq!(deletes[3].arg("NewExternalPort"), "27080");
    }

    #[test]
    fn a_conflict_is_explained_and_the_other_ports_still_map() {
        let fake = start_http(|_| Igd {
            description: description(&[(IP1, "/ctl")]),
            faults: vec![("UDP".into(), 27067, 718)],
            ..Igd::default()
        });
        let gateway = gateway_for(&fake, "/d.xml");
        let mut lease = open_ports_on(gateway, &[PortRequest::tcp(27067), PortRequest::udp(27067)], "Sigil").unwrap();
        assert!(lease.results[0].ok());
        assert_eq!(
            lease.results[1].error.as_deref(),
            Some("Port 27067 is already forwarded to another device on your router.")
        );
        assert_eq!(lease.mapped(), vec![PortRequest::tcp(27067)]);
        assert_eq!(lease.external_ip, None, "an empty answer is no address");
        lease.release();
        assert_eq!(fake.actions("DeletePortMapping").len(), 1, "only what was mapped is removed");
    }

    #[test]
    fn nothing_mapped_is_an_error_with_the_reason() {
        let fake = start_http(|_| Igd {
            description: description(&[(IP1, "/ctl")]),
            faults: vec![("TCP".into(), 27067, 606)],
            ..Igd::default()
        });
        let gateway = gateway_for(&fake, "/d.xml");
        let error = open_ports_on(gateway, &[PortRequest::tcp(27067)], "Sigil").unwrap_err().to_string();
        assert!(error.contains("UPnP error 606"), "{error}");
        assert!(fake.actions("DeletePortMapping").is_empty());
    }

    #[test]
    fn a_router_with_permanent_leases_only_gets_lease_zero() {
        let fake = start_http(|_| Igd { description: description(&[(IP1, "/ctl")]), permanent_only: true, ..Igd::default() });
        let gateway = gateway_for(&fake, "/d.xml");
        let lease = open_ports_on(gateway, &[PortRequest::udp(27067)], "Sigil").unwrap();
        assert!(lease.results[0].ok());
        let leases: Vec<String> = fake.actions("AddPortMapping").iter().map(|s| s.arg("NewLeaseDuration")).collect();
        assert_eq!(leases, vec!["43200".to_string(), "0".to_string()]);
    }

    #[test]
    fn replies_that_point_elsewhere_are_never_contacted() {
        let fake = standard();
        let ssdp = start_ssdp(vec![
            reply("http://10.0.0.1:5000/desc.xml"),
            reply("http://8.8.8.8/desc.xml"),
            reply("http://router.local/desc.xml"),
            reply(&format!("https://127.0.0.1:{}/desc.xml", fake.port)),
            reply(&format!("http://user@127.0.0.1:{}/desc.xml", fake.port)),
        ]);
        let started = Instant::now();
        let error = discover_at(ssdp, Duration::from_millis(800)).unwrap_err().to_string();
        assert_eq!(error, OUTSIDE);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(fake.seen().is_empty(), "no description was fetched");
    }

    #[test]
    fn a_control_url_on_another_host_is_refused() {
        for control in ["http://10.1.2.3/ctl", "//10.1.2.3/ctl", "https://127.0.0.1/ctl", "http://8.8.8.8/ctl"] {
            let fake = start_http(|_| Igd { description: description(&[(IP1, control)]), ..Igd::default() });
            let ssdp = start_ssdp(vec![reply(&fake.location("/d.xml"))]);
            let error = discover_at(ssdp, Duration::from_millis(800)).unwrap_err().to_string();
            assert_eq!(error, OUTSIDE, "{control}");
        }
    }

    #[test]
    fn no_reply_times_out_with_a_clear_error() {
        let silent = start_ssdp(Vec::new());
        let started = Instant::now();
        let error = discover_at(silent, Duration::from_millis(500)).unwrap_err().to_string();
        assert_eq!(error, NO_ROUTER);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn garbage_and_oversized_replies_are_skipped() {
        let fake = standard();
        let mut oversized = reply(&fake.location("/big.xml"));
        oversized.extend(std::iter::repeat_n(b'x', 3000));
        let ssdp = start_ssdp(vec![
            oversized,
            vec![0xff, 0xfe, 0x00, 0x41],
            b"NOTIFY * HTTP/1.1\r\nLOCATION: http://127.0.0.1/x\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nST: urn:dial-multiscreen-org:service:dial:1\r\nLOCATION: http://127.0.0.1:1/tv\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\n\r\n".to_vec(),
            reply(&fake.location("/d.xml")),
        ]);
        let gateway = discover_at(ssdp, Duration::from_secs(2)).unwrap();
        assert_eq!(gateway.control_path, "/ctl");
        let fetched: Vec<String> = fake.seen().iter().map(|s| s.path.clone()).collect();
        assert_eq!(fetched, vec!["/d.xml".to_string()], "only the good reply was followed");
    }

    #[test]
    fn an_oversized_description_is_refused() {
        let fake = start_http(|_| Igd { huge: true, ..Igd::default() });
        let ssdp = start_ssdp(vec![reply(&fake.location("/d.xml"))]);
        let error = discover_at(ssdp, Duration::from_millis(800)).unwrap_err().to_string();
        assert!(error.contains("too large"), "{error}");
    }

    #[test]
    fn approval_is_required() {
        let error = open_ports(&[PortRequest::tcp(27067)], "Sigil", false).unwrap_err().to_string();
        assert!(error.contains("not approved"), "{error}");
    }

    #[test]
    fn urls_and_addresses() {
        let lan = Ipv4Addr::new(192, 168, 1, 1);
        assert_eq!(
            location_target("http://192.168.1.1:5000/rootDesc.xml", lan),
            Some((SocketAddrV4::new(lan, 5000), "/rootDesc.xml".to_string()))
        );
        assert_eq!(location_target("HTTP://192.168.1.1", lan).map(|t| t.1), Some("/".to_string()));
        assert!(location_target("http://192.168.1.2/x", lan).is_none(), "another host");
        assert!(location_target("http://8.8.8.8/x", Ipv4Addr::new(8, 8, 8, 8)).is_none(), "public");
        assert!(location_target("http://[::1]:80/x", lan).is_none());
        assert!(location_target("http://192.168.1.1:0/x", lan).is_none());
        assert!(location_target("http://192.168.1.1/a b", lan).is_none());
        assert!(location_target("ftp://192.168.1.1/x", lan).is_none());
        assert!(local_network(Ipv4Addr::new(172, 20, 0, 1)));
        assert!(local_network(Ipv4Addr::new(169, 254, 3, 4)));
        assert!(!local_network(Ipv4Addr::new(172, 32, 0, 1)));
        assert!(is_local_network("::ffff:10.0.0.2".parse().unwrap()));
        assert!(!is_local_network("2001:db8::1".parse().unwrap()));

        let base = (SocketAddrV4::new(lan, 80), "/igd/root.xml".to_string());
        assert_eq!(resolve(&base, "ctl").unwrap().1, "/igd/ctl");
        assert_eq!(resolve(&base, "/ctl").unwrap().1, "/ctl");
        assert_eq!(resolve(&base, "http://192.168.1.1:49000/c").unwrap().0.port(), 49000);
        assert!(resolve(&base, "http://192.168.1.9/c").is_none());
        assert!(resolve(&base, "").is_none());
    }

    #[test]
    fn xml_and_answers() {
        let xml = "<a><s:errorCode> 718 </s:errorCode><errorCodeList>x</errorCodeList><b/><serviceType>t</serviceType ></a>";
        assert_eq!(elements(xml, "errorCode"), vec![" 718 "]);
        assert_eq!(elements(xml, "b"), vec![""]);
        assert_eq!(elements(xml, "serviceType"), vec!["t"]);
        assert!(elements("<a><b>unclosed", "b").is_empty());
        assert_eq!(unescape(&escape("a&b<c>\"'")), "a&b<c>\"'");
        assert_eq!(clean_description("\u{1F600}\n\t "), "Sigil");
        assert_eq!(clean_description(&"x".repeat(100)).len(), 64);

        assert_eq!(dechunk(b"3\r\nabc\r\n0\r\n\r\n"), Some(b"abc".to_vec()));
        assert_eq!(dechunk(b"3\r\nab"), None);
        assert_eq!(dechunk(b"zz\r\n"), None);
        assert!(parse_response(b"garbage", true).is_err());
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nab", false).unwrap(), Some((200, b"ab".to_vec())));
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nab", false).unwrap(), None);
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nab", true).is_err());
        assert_eq!(parse_response(b"HTTP/1.0 404 No\r\n\r\nrest", true).unwrap(), Some((404, b"rest".to_vec())));

        assert_eq!(
            SoapError::Fault { code: 501, description: "Action Failed".into() }.message(1),
            "The router refused port 1 (UPnP error 501: Action Failed)."
        );
    }
}
