use crate::torrent::TorrentInfo;
use crate::{
    error::{Result, TorrentError},
    peer_id,
};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use serde_bencode::de::from_bytes;
use tokio::net::{lookup_host, UdpSocket};
use tokio::time::{timeout, Duration};
use url::form_urlencoded::byte_serialize;
use url::Url;

pub use peers::Peers;

const UDP_CONNECT_ACTION: u32 = 0;
const UDP_ANNOUNCE_ACTION: u32 = 1;
const UDP_ERROR_ACTION: u32 = 3;
const UDP_CONNECT_MAGIC: u64 = 0x41727101980;
const UDP_MAX_ATTEMPTS: usize = 3;
const UDP_INITIAL_TIMEOUT: Duration = Duration::from_secs(5);
const UDP_CONNECTION_ID_TTL: Duration = Duration::from_secs(60);
const UDP_MAX_PACKET_SIZE: usize = 65_536;

#[derive(Debug, Clone)]
pub struct TrackerRequest {
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: usize,
    pub downloaded: usize,
    pub left: usize,
    pub compact: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackerResponse {
    #[serde(default)]
    pub interval: Option<usize>,
    pub peers: Peers,
}

#[derive(Debug, Deserialize)]
struct RawTrackerResponse {
    #[serde(default)]
    interval: Option<usize>,
    #[serde(rename = "failure reason")]
    failure_reason: Option<String>,
    peers: Option<Peers>,
}

#[derive(Debug, Clone, Copy)]
struct UdpConnectionId {
    value: u64,
    elapsed_since_connect: Duration,
}

impl TrackerResponse {
    pub async fn query(t: &TorrentInfo, info_hash: &[u8; 20]) -> Result<Self> {
        if t.trackers.is_empty() {
            return Err(TorrentError::Tracker("No trackers available".into()));
        }

        let request = TrackerRequest {
            peer_id: peer_id::generate_peer_id(),
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: t.length.max(0) as usize,
            compact: 1,
        };

        let client = Client::new();
        let mut last_error = None;
        let mut last_empty_response = None;

        for tracker in &t.trackers {
            match query_tracker(&client, tracker, info_hash, &request).await {
                Ok(response) if !response.peers.0.is_empty() => return Ok(response),
                Ok(response) => last_empty_response = Some(response),
                Err(err) => last_error = Some(err),
            }
        }

        if let Some(response) = last_empty_response {
            return Ok(response);
        }

        Err(last_error.unwrap_or(TorrentError::NoPeersAvailable))
    }
}

async fn query_tracker(
    client: &Client,
    tracker_url: &str,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
) -> Result<TrackerResponse> {
    let normalized = normalize_tracker_url(tracker_url)?;
    match normalized.scheme() {
        "http" | "https" => {
            query_http_tracker(client, normalized.as_str(), info_hash, request).await
        }
        "udp" => query_udp_tracker(&normalized, info_hash, request).await,
        scheme => Err(TorrentError::Tracker(format!(
            "Unsupported tracker scheme: {}",
            scheme
        ))),
    }
}

fn normalize_tracker_url(tracker_url: &str) -> Result<Url> {
    if tracker_url.is_empty() {
        return Err(TorrentError::Tracker("Tracker URL is empty".into()));
    }

    let normalized = if tracker_url.contains("://") {
        tracker_url.to_string()
    } else {
        format!("http://{}", tracker_url)
    };

    Url::parse(&normalized).map_err(TorrentError::from)
}

async fn query_http_tracker(
    client: &Client,
    tracker_url: &str,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
) -> Result<TrackerResponse> {
    let url = build_tracker_url(tracker_url, info_hash, request)?;
    let response = make_tracker_request(client, &url).await?;
    parse_tracker_response(&response)
}

async fn query_udp_tracker(
    tracker_url: &Url,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
) -> Result<TrackerResponse> {
    let host = tracker_url
        .host_str()
        .ok_or_else(|| TorrentError::Tracker("UDP tracker URL is missing a host".into()))?;
    let port = tracker_url
        .port()
        .ok_or_else(|| TorrentError::Tracker("UDP tracker URL is missing a port".into()))?;
    let remote = lookup_host((host, port))
        .await
        .map_err(|e| TorrentError::Tracker(format!("Failed to resolve UDP tracker: {}", e)))?
        .next()
        .ok_or_else(|| TorrentError::Tracker("UDP tracker host resolved to no addresses".into()))?;

    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| TorrentError::Tracker(format!("Failed to bind UDP socket: {}", e)))?;
    socket
        .connect(remote)
        .await
        .map_err(|e| TorrentError::Tracker(format!("Failed to connect UDP socket: {}", e)))?;

    let connection_id = udp_connect(&socket).await?;
    udp_announce(&socket, connection_id, info_hash, request).await
}

async fn udp_connect(socket: &UdpSocket) -> Result<UdpConnectionId> {
    let transaction_id = rand::thread_rng().gen::<u32>();
    let request = encode_udp_connect_request(transaction_id);
    let response = send_udp_with_retry(socket, &request).await?;
    Ok(UdpConnectionId {
        value: parse_udp_connect_response(&response, transaction_id)?,
        elapsed_since_connect: Duration::ZERO,
    })
}

async fn udp_announce(
    socket: &UdpSocket,
    mut connection_id: UdpConnectionId,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
) -> Result<TrackerResponse> {
    let mut timeout_duration = UDP_INITIAL_TIMEOUT;

    for attempt in 0..UDP_MAX_ATTEMPTS {
        if !udp_connection_id_is_fresh(connection_id.elapsed_since_connect) {
            connection_id = udp_connect(socket).await?;
            timeout_duration = UDP_INITIAL_TIMEOUT;
        }

        let transaction_id = rand::thread_rng().gen::<u32>();
        let key = rand::thread_rng().gen::<u32>();
        let packet = encode_udp_announce_request(
            connection_id.value,
            transaction_id,
            info_hash,
            request,
            key,
        );

        socket.send(&packet).await.map_err(|e| {
            TorrentError::Tracker(format!("Failed to send UDP tracker packet: {}", e))
        })?;

        let mut buffer = vec![0u8; UDP_MAX_PACKET_SIZE];
        match timeout(timeout_duration, socket.recv(&mut buffer)).await {
            Ok(Ok(size)) => return parse_udp_announce_response(&buffer[..size], transaction_id),
            Ok(Err(e)) => {
                return Err(TorrentError::Tracker(format!(
                    "Failed to receive UDP tracker packet: {}",
                    e
                )))
            }
            Err(_) if attempt + 1 == UDP_MAX_ATTEMPTS => {
                return Err(TorrentError::ConnectionTimeout)
            }
            Err(_) => {
                connection_id.elapsed_since_connect += timeout_duration;
                timeout_duration *= 2;
            }
        }
    }

    Err(TorrentError::ConnectionTimeout)
}

async fn send_udp_with_retry(socket: &UdpSocket, packet: &[u8]) -> Result<Vec<u8>> {
    let mut timeout_duration = UDP_INITIAL_TIMEOUT;
    let mut buffer = vec![0u8; UDP_MAX_PACKET_SIZE];

    for attempt in 0..UDP_MAX_ATTEMPTS {
        socket.send(packet).await.map_err(|e| {
            TorrentError::Tracker(format!("Failed to send UDP tracker packet: {}", e))
        })?;

        match timeout(timeout_duration, socket.recv(&mut buffer)).await {
            Ok(Ok(size)) => return Ok(buffer[..size].to_vec()),
            Ok(Err(e)) => {
                return Err(TorrentError::Tracker(format!(
                    "Failed to receive UDP tracker packet: {}",
                    e
                )))
            }
            Err(_) if attempt + 1 == UDP_MAX_ATTEMPTS => {
                return Err(TorrentError::ConnectionTimeout)
            }
            Err(_) => timeout_duration *= 2,
        }
    }

    Err(TorrentError::ConnectionTimeout)
}

fn build_tracker_url(
    tracker_url: &str,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
) -> Result<String> {
    let separator = if tracker_url.contains('?') { '&' } else { '?' };
    let query = [
        ("peer_id", percent_encode(&request.peer_id)),
        ("port", request.port.to_string()),
        ("uploaded", request.uploaded.to_string()),
        ("downloaded", request.downloaded.to_string()),
        ("left", request.left.to_string()),
        ("compact", request.compact.to_string()),
        ("info_hash", percent_encode(info_hash)),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key}={value}"))
    .collect::<Vec<_>>()
    .join("&");

    Ok(format!("{tracker_url}{separator}{query}"))
}

fn percent_encode(bytes: &[u8]) -> String {
    byte_serialize(bytes).collect()
}

fn parse_tracker_response(response: &[u8]) -> Result<TrackerResponse> {
    let tracker_info: RawTrackerResponse = from_bytes(response).map_err(|e| {
        TorrentError::InvalidResponseFormat(format!(
            "Failed to deserialize tracker response: {}",
            e
        ))
    })?;

    if let Some(failure_reason) = tracker_info.failure_reason {
        return Err(TorrentError::Tracker(failure_reason));
    }

    let peers = tracker_info.peers.ok_or_else(|| {
        TorrentError::InvalidResponseFormat("Tracker response missing peers".into())
    })?;

    Ok(TrackerResponse {
        interval: tracker_info.interval,
        peers,
    })
}

fn encode_udp_connect_request(transaction_id: u32) -> [u8; 16] {
    let mut packet = [0u8; 16];
    packet[0..8].copy_from_slice(&UDP_CONNECT_MAGIC.to_be_bytes());
    packet[8..12].copy_from_slice(&UDP_CONNECT_ACTION.to_be_bytes());
    packet[12..16].copy_from_slice(&transaction_id.to_be_bytes());
    packet
}

fn parse_udp_connect_response(response: &[u8], expected_transaction_id: u32) -> Result<u64> {
    if response.len() < 8 {
        return Err(TorrentError::InvalidResponseFormat(
            "UDP connect response too short".into(),
        ));
    }

    let action = u32::from_be_bytes(response[0..4].try_into().expect("slice length checked"));
    let transaction_id =
        u32::from_be_bytes(response[4..8].try_into().expect("slice length checked"));

    if transaction_id != expected_transaction_id {
        return Err(TorrentError::Tracker(
            "UDP tracker transaction ID mismatch".into(),
        ));
    }

    if action == UDP_ERROR_ACTION {
        let message = String::from_utf8_lossy(&response[8..]).into_owned();
        return Err(TorrentError::Tracker(message));
    }

    if action != UDP_CONNECT_ACTION || response.len() < 16 {
        return Err(TorrentError::InvalidResponseFormat(
            "Invalid UDP connect response".into(),
        ));
    }

    Ok(u64::from_be_bytes(
        response[8..16].try_into().expect("slice length checked"),
    ))
}

fn encode_udp_announce_request(
    connection_id: u64,
    transaction_id: u32,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
    key: u32,
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(98);
    packet.extend_from_slice(&connection_id.to_be_bytes());
    packet.extend_from_slice(&UDP_ANNOUNCE_ACTION.to_be_bytes());
    packet.extend_from_slice(&transaction_id.to_be_bytes());
    packet.extend_from_slice(info_hash);
    packet.extend_from_slice(&request.peer_id);
    packet.extend_from_slice(&(request.downloaded as u64).to_be_bytes());
    packet.extend_from_slice(&(request.left as u64).to_be_bytes());
    packet.extend_from_slice(&(request.uploaded as u64).to_be_bytes());
    packet.extend_from_slice(&0u32.to_be_bytes());
    packet.extend_from_slice(&0u32.to_be_bytes());
    packet.extend_from_slice(&key.to_be_bytes());
    packet.extend_from_slice(&u32::MAX.to_be_bytes());
    packet.extend_from_slice(&request.port.to_be_bytes());
    packet
}

fn parse_udp_announce_response(
    response: &[u8],
    expected_transaction_id: u32,
) -> Result<TrackerResponse> {
    if response.len() < 8 {
        return Err(TorrentError::InvalidResponseFormat(
            "UDP announce response too short".into(),
        ));
    }

    let action = u32::from_be_bytes(response[0..4].try_into().expect("slice length checked"));
    let transaction_id =
        u32::from_be_bytes(response[4..8].try_into().expect("slice length checked"));

    if transaction_id != expected_transaction_id {
        return Err(TorrentError::Tracker(
            "UDP tracker transaction ID mismatch".into(),
        ));
    }

    if action == UDP_ERROR_ACTION {
        let message = String::from_utf8_lossy(&response[8..]).into_owned();
        return Err(TorrentError::Tracker(message));
    }

    if action != UDP_ANNOUNCE_ACTION || response.len() < 20 {
        return Err(TorrentError::InvalidResponseFormat(
            "Invalid UDP announce response".into(),
        ));
    }

    let interval = u32::from_be_bytes(response[8..12].try_into().expect("slice length checked"));
    let peers = Peers::from_compact_bytes(&response[20..])?;

    Ok(TrackerResponse {
        interval: Some(interval as usize),
        peers,
    })
}

fn udp_connection_id_is_fresh(elapsed_since_connect: Duration) -> bool {
    elapsed_since_connect < UDP_CONNECTION_ID_TTL
}

async fn make_tracker_request(client: &Client, url: &str) -> Result<Vec<u8>> {
    println!("Tracker URL: {}", url);
    let response =
        client.get(url).send().await.map_err(|e| {
            TorrentError::Tracker(format!("Failed to send request to tracker: {}", e))
        })?;

    if !response.status().is_success() {
        return Err(TorrentError::Tracker(format!(
            "Tracker returned error status: {}",
            response.status()
        )));
    }

    let bytes = response.bytes().await.map_err(|e| {
        TorrentError::Tracker(format!("Failed to read tracker response bytes: {}", e))
    })?;
    Ok(bytes.to_vec())
}

mod peers {
    use crate::error::{Result, TorrentError};
    use serde::de::{self, Visitor};
    use serde::ser::Serializer;
    use std::fmt;
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[derive(Debug, Clone)]
    pub struct Peers(pub Vec<SocketAddrV4>);

    impl Peers {
        pub(crate) fn from_compact_bytes(v: &[u8]) -> Result<Self> {
            if !v.len().is_multiple_of(6) {
                return Err(TorrentError::InvalidResponseFormat(format!(
                    "Peers byte string length {} is not a multiple of 6",
                    v.len()
                )));
            }

            let peers = v
                .chunks_exact(6)
                .map(|chunk| {
                    let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                    let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                    SocketAddrV4::new(ip, port)
                })
                .collect();

            Ok(Peers(peers))
        }
    }

    struct PeersVisitor;

    impl<'de> Visitor<'de> for PeersVisitor {
        type Value = Peers;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a byte string with each peer represented by 6 bytes")
        }

        fn visit_bytes<E>(self, v: &[u8]) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Peers::from_compact_bytes(v).map_err(E::custom)
        }
    }

    impl<'de> serde::Deserialize<'de> for Peers {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_bytes(PeersVisitor)
        }
    }

    impl serde::Serialize for Peers {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut bytes = Vec::with_capacity(6 * self.0.len());
            for peer in &self.0 {
                bytes.extend_from_slice(&peer.ip().octets());
                bytes.extend_from_slice(&peer.port().to_be_bytes());
            }
            serializer.serialize_bytes(&bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_tracker_url, encode_udp_announce_request, encode_udp_connect_request,
        parse_tracker_response, parse_udp_announce_response, parse_udp_connect_response,
        percent_encode, udp_connection_id_is_fresh, TrackerRequest,
    };
    use std::time::Duration;

    #[test]
    fn percent_encodes_raw_tracker_bytes() {
        assert_eq!(percent_encode(&[0, b'A', 255]), "%00A%FF");
    }

    #[test]
    fn builds_tracker_url_with_binary_fields() {
        let request = TrackerRequest {
            peer_id: *b"-TR3000-123456789012",
            port: 6881,
            uploaded: 1,
            downloaded: 2,
            left: 3,
            compact: 1,
        };
        let info_hash = [0u8; 20];

        let url = build_tracker_url("http://tracker.test/announce", &info_hash, &request)
            .expect("tracker url should build");

        assert!(url.starts_with("http://tracker.test/announce?"));
        assert!(url.contains("peer_id=-TR3000-123456789012"));
        assert!(url.contains("port=6881"));
        assert!(url.contains("info_hash=%00%00%00%00"));
    }

    #[test]
    fn parses_tracker_failure_reason() {
        let encoded = serde_bencode::to_bytes(&serde_bencode::value::Value::Dict(
            vec![(
                b"failure reason".to_vec(),
                serde_bencode::value::Value::Bytes(b"bad tracker".to_vec()),
            )]
            .into_iter()
            .collect(),
        ))
        .expect("failure response should encode");
        let err = parse_tracker_response(&encoded).expect_err("failure reason should be surfaced");

        assert!(err.to_string().contains("bad tracker"));
    }

    #[test]
    fn parses_compact_peer_lists() {
        let encoded = b"d8:intervali1800e5:peers6:\x7f\x00\x00\x01\x1a\xe1e".to_vec();
        let response = parse_tracker_response(&encoded).expect("compact peer list should parse");

        assert_eq!(response.interval, Some(1800));
        assert_eq!(response.peers.0.len(), 1);
        assert_eq!(response.peers.0[0].to_string(), "127.0.0.1:6881");
    }

    #[test]
    fn encodes_udp_connect_request() {
        let packet = encode_udp_connect_request(0x11223344);
        assert_eq!(&packet[0..8], &0x41727101980u64.to_be_bytes());
        assert_eq!(&packet[8..12], &0u32.to_be_bytes());
        assert_eq!(&packet[12..16], &0x11223344u32.to_be_bytes());
    }

    #[test]
    fn parses_udp_connect_response_and_rejects_mismatched_transaction_ids() {
        let mut response = Vec::new();
        response.extend_from_slice(&0u32.to_be_bytes());
        response.extend_from_slice(&0x11223344u32.to_be_bytes());
        response.extend_from_slice(&0x0102030405060708u64.to_be_bytes());

        let connection_id = parse_udp_connect_response(&response, 0x11223344)
            .expect("connect response should parse");
        assert_eq!(connection_id, 0x0102030405060708);
        assert!(parse_udp_connect_response(&response, 0x55667788).is_err());
    }

    #[test]
    fn parses_udp_error_packets() {
        let mut response = Vec::new();
        response.extend_from_slice(&3u32.to_be_bytes());
        response.extend_from_slice(&0x11223344u32.to_be_bytes());
        response.extend_from_slice(b"bad udp tracker");

        let err = parse_udp_connect_response(&response, 0x11223344)
            .expect_err("udp error packet should be surfaced");
        assert!(err.to_string().contains("bad udp tracker"));
    }

    #[test]
    fn udp_connection_ids_expire_after_ttl() {
        assert!(udp_connection_id_is_fresh(Duration::from_secs(59)));
        assert!(!udp_connection_id_is_fresh(Duration::from_secs(60)));
    }

    #[test]
    fn encodes_udp_announce_request() {
        let request = TrackerRequest {
            peer_id: *b"-TR3000-123456789012",
            port: 6881,
            uploaded: 1,
            downloaded: 2,
            left: 3,
            compact: 1,
        };
        let packet = encode_udp_announce_request(
            0x0102030405060708,
            0x11223344,
            &[0u8; 20],
            &request,
            0x55667788,
        );

        assert_eq!(&packet[0..8], &0x0102030405060708u64.to_be_bytes());
        assert_eq!(&packet[8..12], &1u32.to_be_bytes());
        assert_eq!(&packet[12..16], &0x11223344u32.to_be_bytes());
        assert_eq!(packet.len(), 98);
        assert_eq!(&packet[92..96], &u32::MAX.to_be_bytes());
        assert_eq!(&packet[96..98], &6881u16.to_be_bytes());
    }

    #[test]
    fn parses_udp_announce_response_and_rejects_bad_peer_tails() {
        let mut response = Vec::new();
        response.extend_from_slice(&1u32.to_be_bytes());
        response.extend_from_slice(&0x11223344u32.to_be_bytes());
        response.extend_from_slice(&1800u32.to_be_bytes());
        response.extend_from_slice(&2u32.to_be_bytes());
        response.extend_from_slice(&4u32.to_be_bytes());
        response.extend_from_slice(&[127, 0, 0, 1, 0x1A, 0xE1]);

        let parsed = parse_udp_announce_response(&response, 0x11223344)
            .expect("announce response should parse");
        assert_eq!(parsed.interval, Some(1800));
        assert_eq!(parsed.peers.0[0].to_string(), "127.0.0.1:6881");

        let mut bad = response.clone();
        bad.push(0);
        assert!(parse_udp_announce_response(&bad, 0x11223344).is_err());
    }
}
