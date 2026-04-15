use std::net::SocketAddrV4;

use crate::{
    error::{Result, TorrentError},
    magnet::{self, Magnet},
    peer, peer_id,
    torrent::{self, TorrentInfo},
    tracker::{self, TrackerResponse},
};

pub struct ResolvedDownloadSource {
    pub info: TorrentInfo,
    pub info_hash: [u8; 20],
    pub peers: Vec<SocketAddrV4>,
    pub is_magnet: bool,
}

pub fn is_magnet_link(source: &str) -> bool {
    source.starts_with("magnet:?")
}

pub fn parse_info_hash(info_hash: &str) -> Result<[u8; 20]> {
    let bytes = hex::decode(info_hash).map_err(|_| TorrentError::InvalidInfoHash)?;
    bytes.try_into().map_err(|_| TorrentError::InvalidInfoHash)
}

pub async fn resolve_download_source(source: &str) -> Result<ResolvedDownloadSource> {
    if is_magnet_link(source) {
        let parsed_magnet = Magnet::parse(source)?;
        let info_hash = parse_info_hash(&parsed_magnet.info_hash)?;
        let (info, peers) = retrieve_torrent_info_from_magnet(&parsed_magnet, &info_hash).await?;

        return Ok(ResolvedDownloadSource {
            info,
            info_hash,
            peers,
            is_magnet: true,
        });
    }

    let info = torrent::get_info(source)?;
    let info_hash = parse_info_hash(&info.info_hash)?;
    let tracker_response = tracker::TrackerResponse::query(&info, &info_hash).await?;

    if tracker_response.peers.0.is_empty() {
        return Err(TorrentError::NoPeersAvailable);
    }

    Ok(ResolvedDownloadSource {
        info,
        info_hash,
        peers: tracker_response.peers.0,
        is_magnet: false,
    })
}

async fn retrieve_torrent_info_from_magnet(
    parsed_magnet: &magnet::Magnet,
    info_hash: &[u8; 20],
) -> Result<(TorrentInfo, Vec<SocketAddrV4>)> {
    let torrent_info = TorrentInfo::from_magnet(parsed_magnet)?;
    let tracker_response = TrackerResponse::query_with_url(&torrent_info, info_hash).await?;

    if tracker_response.peers.0.is_empty() {
        return Err(TorrentError::NoPeersAvailable);
    }

    for peer_addr in &tracker_response.peers.0 {
        match get_metadata_from_peer(&peer_addr.to_string(), info_hash).await {
            Ok(validated_info) => return Ok((validated_info, tracker_response.peers.0.clone())),
            Err(e) => {
                eprintln!("Failed to get metadata from peer {}: {}", peer_addr, e);
            }
        }
    }

    Err(TorrentError::NoPeersAvailable)
}

async fn get_metadata_from_peer(peer_addr: &str, info_hash: &[u8; 20]) -> Result<TorrentInfo> {
    let peer_id = peer_id::generate_peer_id();
    let mut peer = peer::Peer::new(peer_addr).await?;

    peer.enable_tcp_nodelay().await?;
    peer.handshake(info_hash, &peer_id).await?;

    let metadata = peer.receive_metadata().await?;
    let info_hash_hex = hex::encode(info_hash);

    TorrentInfo::validate_metadata(&metadata, &info_hash_hex)
}

#[cfg(test)]
mod tests {
    use super::{is_magnet_link, parse_info_hash};

    #[test]
    fn detects_magnet_links() {
        assert!(is_magnet_link("magnet:?xt=urn:btih:abcdef"));
        assert!(!is_magnet_link("sample.torrent"));
    }

    #[test]
    fn parses_valid_info_hashes() {
        let hash = parse_info_hash("0123456789abcdef0123456789abcdef01234567")
            .expect("valid info hash should parse");

        assert_eq!(hash.len(), 20);
    }

    #[test]
    fn rejects_invalid_info_hashes() {
        assert!(parse_info_hash("not-hex").is_err());
        assert!(parse_info_hash("deadbeef").is_err());
    }
}
