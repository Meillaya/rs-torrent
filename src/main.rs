use bittorrent_starter_rust::{
    bencode,
    cli::{parse_cli, Command},
    download,
    error::{Result, TorrentError},
    magnet, peer, peer_id, source,
    torrent::{self, TorrentInfo},
    tracker::TrackerResponse,
};
use serde_bencode::value::Value as BencodeValue;
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_cli();

    match cli.command {
        Command::Decode { bencoded_value } => {
            let bencoded_value = bencoded_value.as_bytes();
            let decoded: BencodeValue = bencode::decode(bencoded_value)?;
            let json_value: Value = bencode_to_json(decoded);
            println!("{}", serde_json::to_string(&json_value)?);
        }
        Command::Info { torrent_file } => {
            let info = torrent::get_info(&torrent_file)?;
            print_torrent_info(&info);
        }
        Command::Peers { torrent_file } => {
            let info = torrent::get_info(&torrent_file)?;
            let info_hash = source::parse_info_hash(&info.info_hash)?;
            let tracker_response = TrackerResponse::query(&info, &info_hash).await?;
            for peer in tracker_response.peers.0 {
                println!("{}", peer);
            }
        }
        Command::Handshake { torrent_file, peer } => {
            let info = torrent::get_info(&torrent_file)?;
            let info_hash = source::parse_info_hash(&info.info_hash)?;
            let peer_id: [u8; 20] = peer_id::generate_peer_id();
            let mut peer = peer::Peer::new(&peer).await?;
            let received_peer_id = peer.handshake(&info_hash, &peer_id).await?;

            println!("Peer ID: {}", hex::encode(received_peer_id));
        }
        Command::DownloadPiece(args) => {
            download::download_piece(&args.output_file, &args.source, args.piece_index).await?;
        }
        Command::Download(args) => {
            download::download_file(&args.output_file, &args.source).await?;
        }
        Command::MagnetParse { magnet_link } => {
            let parsed_magnet = magnet::Magnet::parse(&magnet_link)?;
            for tracker_url in &parsed_magnet.trackers {
                println!("Tracker URL: {}", tracker_url);
            }
            println!("Info Hash: {}", parsed_magnet.info_hash);
        }
        Command::MagnetHandshake { magnet_link } => magnet_handshake(&magnet_link).await?,
        Command::MagnetInfo { magnet_link } => magnet_info(&magnet_link).await?,
        Command::MagnetDownloadPiece(args) => {
            download::download_piece(&args.output_file, &args.source, args.piece_index).await?;
        }
        Command::MagnetDownload(args) => {
            download::download_file(&args.output_file, &args.source).await?;
        }
    }

    Ok(())
}

fn print_torrent_info(info: &TorrentInfo) {
    println!(
        "Tracker URL: {}",
        info.trackers.first().cloned().unwrap_or_default()
    );
    println!("Length: {}", info.length);
    println!("Info Hash: {}", info.info_hash);
    println!("Piece Length: {}", info.piece_length);
    println!("Number of Pieces: {}", info.pieces.len());
    println!("Piece Hashes:");

    for (i, piece) in info.pieces.iter().enumerate() {
        println!("{}: {}", i, hex::encode(piece));
    }
}

async fn magnet_info(magnet_link: &str) -> Result<()> {
    let parsed_magnet = magnet::Magnet::parse(magnet_link)?;
    let info_hash = source::parse_info_hash(&parsed_magnet.info_hash)?;
    let peer_id: [u8; 20] = peer_id::generate_peer_id();

    let torrent_info = TorrentInfo::from_magnet(&parsed_magnet)?;
    let tracker_response = TrackerResponse::query(&torrent_info, &info_hash).await?;

    if tracker_response.peers.0.is_empty() {
        return Err(TorrentError::NoPeersAvailable);
    }

    let peer_addr = &tracker_response.peers.0[0].to_string();
    let mut peer = peer::Peer::new(peer_addr).await?;

    peer.handshake(&info_hash, &peer_id).await?;
    let metadata = peer.receive_metadata().await?;

    // Validate and display the received metadata
    let validated_info = TorrentInfo::validate_metadata(&metadata, &parsed_magnet.info_hash)?;

    println!(
        "Tracker URL: {}",
        parsed_magnet.trackers.first().cloned().unwrap_or_default()
    );
    println!("Length: {}", validated_info.length);
    println!("Info Hash: {}", validated_info.info_hash);
    println!("Piece Length: {}", validated_info.piece_length);
    println!("Piece Hashes:");
    for piece_hash in &validated_info.pieces {
        println!("{}", hex::encode(piece_hash));
    }

    Ok(())
}

async fn magnet_handshake(magnet_link: &str) -> Result<()> {
    let parsed_magnet = magnet::Magnet::parse(magnet_link)?;
    let info_hash = source::parse_info_hash(&parsed_magnet.info_hash)?;

    let torrent_info = TorrentInfo::from_magnet(&parsed_magnet)?;
    let tracker_response = TrackerResponse::query(&torrent_info, &info_hash).await?;

    if tracker_response.peers.0.is_empty() {
        return Err(TorrentError::NoPeersAvailable);
    }

    let peer_addr = &tracker_response.peers.0[0].to_string();
    let mut peer = peer::Peer::new(peer_addr).await?;
    let peer_id = peer_id::generate_peer_id();
    let received_peer_id = peer.handshake(&info_hash, &peer_id).await?;

    println!("Peer ID: {}", hex::encode(received_peer_id));

    Ok(())
}

fn bencode_to_json(value: BencodeValue) -> Value {
    match value {
        BencodeValue::Bytes(b) => match String::from_utf8(b.clone()) {
            Ok(s) => Value::String(s),
            Err(_) => Value::String(hex::encode(b)),
        },
        BencodeValue::Int(i) => Value::Number(i.into()),
        BencodeValue::List(l) => Value::Array(l.into_iter().map(bencode_to_json).collect()),
        BencodeValue::Dict(d) => {
            let mut map = serde_json::Map::new();
            for (k, v) in d {
                let key = String::from_utf8_lossy(&k).into_owned();
                map.insert(key, bencode_to_json(v));
            }
            Value::Object(map)
        }
    }
}
