use crate::torrent::TorrentInfo;
use crate::{
    error::{Result, TorrentError},
    peer_id,
};
use reqwest::Client;
use serde::Deserialize;
use serde_bencode::de::from_bytes;
use url::form_urlencoded::byte_serialize;

pub use peers::Peers;

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

impl TrackerResponse {
    pub async fn query(t: &TorrentInfo, info_hash: &[u8; 20]) -> Result<Self> {
        let client = Client::new();
        let request = TrackerRequest {
            peer_id: peer_id::generate_peer_id(),
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: t.length as usize,
            compact: 1,
        };

        let tracker_url = build_tracker_url(&t.announce, info_hash, &request, false)?;

        let response = make_tracker_request(&client, &tracker_url).await?;
        parse_tracker_response(&response)
    }

    pub async fn query_with_url(t: &TorrentInfo, info_hash: &[u8; 20]) -> Result<Self> {
        let client = Client::new();
        let request = TrackerRequest {
            peer_id: peer_id::generate_peer_id(),
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: t.length.max(0) as usize,
            compact: 1,
        };

        let full_url = build_tracker_url(&t.announce, info_hash, &request, true)?;
        let response = make_tracker_request(&client, &full_url).await?;
        parse_tracker_response(&response)
    }
}

fn build_tracker_url(
    announce: &str,
    info_hash: &[u8; 20],
    request: &TrackerRequest,
    normalize_scheme: bool,
) -> Result<String> {
    if announce.is_empty() {
        return Err(TorrentError::Tracker("Tracker URL is empty".into()));
    }

    let announce = if normalize_scheme
        && !announce.starts_with("http://")
        && !announce.starts_with("https://")
    {
        format!("http://{}", announce)
    } else {
        announce.to_string()
    };

    let separator = if announce.contains('?') { '&' } else { '?' };
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

    Ok(format!("{announce}{separator}{query}"))
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
    use serde::de::{self, Visitor};
    use serde::ser::Serializer;
    use std::fmt;
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[derive(Debug, Clone)]
    pub struct Peers(pub Vec<SocketAddrV4>);

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
            if !v.len().is_multiple_of(6) {
                return Err(E::custom(format!(
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
    use super::{build_tracker_url, parse_tracker_response, percent_encode, TrackerRequest};

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

        let url = build_tracker_url("http://tracker.test/announce", &info_hash, &request, false)
            .expect("tracker url should build");

        assert!(url.starts_with("http://tracker.test/announce?"));
        assert!(url.contains("peer_id=-TR3000-123456789012"));
        assert!(url.contains("port=6881"));
        assert!(url.contains("info_hash=%00%00%00%00"));
    }

    #[test]
    fn normalizes_tracker_scheme_for_magnet_urls() {
        let request = TrackerRequest {
            peer_id: *b"-TR3000-123456789012",
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: 0,
            compact: 1,
        };
        let info_hash = [1u8; 20];

        let url = build_tracker_url("tracker.test/announce", &info_hash, &request, true)
            .expect("tracker url should build");

        assert!(url.starts_with("http://tracker.test/announce?"));
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
}
