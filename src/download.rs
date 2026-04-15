use crate::{
    error::{Result, TorrentError},
    peer, peer_id,
    report::{self, ProgressEvent},
    source,
    storage::DownloadStorage,
    torrent,
};
use rand::seq::SliceRandom;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};
use tokio::{fs, time::timeout};

const MAX_RETRIES: u32 = 5;
const WORKER_COUNT: usize = 10;
const RETRY_DELAY: Duration = Duration::from_secs(2);

pub async fn download_file(output_file: &str, source: &str) -> Result<()> {
    let source::ResolvedDownloadSource {
        info,
        info_hash,
        peers,
        is_magnet,
    } = source::resolve_download_source(source).await?;

    let storage = DownloadStorage::open(output_file, &info).await?;
    report::emit_stdout(&ProgressEvent::ResumeLoaded {
        completed_pieces: storage.completed_piece_count(),
        total_pieces: storage.total_piece_count(),
    });
    let piece_availability = collect_piece_availability(&peers, &info, &info_hash, is_magnet).await;
    let mut missing_pieces = storage.missing_piece_indices();
    sort_missing_pieces_by_availability(&mut missing_pieces, &piece_availability);
    if missing_pieces.is_empty() {
        storage.finalize().await?;
        report::emit_stdout(&ProgressEvent::DownloadFinalized {
            output: output_file,
        });
        return Ok(());
    }

    let piece_queue = Arc::new(Mutex::new(VecDeque::from(missing_pieces.clone())));
    let storage = Arc::new(Mutex::new(storage));
    let mut join_set = JoinSet::new();
    let worker_count = missing_pieces.len().min(WORKER_COUNT);

    for _ in 0..worker_count {
        let piece_queue = Arc::clone(&piece_queue);
        let storage = Arc::clone(&storage);
        let peers = peers.clone();
        let info = info.clone();

        join_set.spawn(async move {
            loop {
                let piece_index = {
                    let mut queue = piece_queue.lock().await;
                    queue.pop_front()
                };

                let Some(piece_index) = piece_index else {
                    break Ok::<(), TorrentError>(());
                };

                let piece_data = download_piece_with_retry_from_peers(
                    &peers,
                    &info,
                    &info_hash,
                    piece_index,
                    is_magnet,
                )
                .await?;

                let mut storage = storage.lock().await;
                storage.write_piece(&info, piece_index, &piece_data).await?;
                report::emit_stdout(&ProgressEvent::PieceStored {
                    piece_index,
                    completed_pieces: storage.completed_piece_count(),
                    total_pieces: storage.total_piece_count(),
                });
            }
        });
    }

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(TorrentError::DownloadFailed(format!(
                    "A download task failed: {}",
                    err
                )))
            }
        }
    }

    let storage = Arc::try_unwrap(storage)
        .map_err(|_| TorrentError::DownloadFailed("Failed to unwrap download storage".into()))?
        .into_inner();

    if !storage.is_complete() {
        return Err(TorrentError::DownloadFailed(
            "Some pieces failed to download".into(),
        ));
    }

    storage.finalize().await?;
    report::emit_stdout(&ProgressEvent::DownloadFinalized {
        output: output_file,
    });
    Ok(())
}

pub async fn download_piece(output_file: &str, source: &str, piece_index: usize) -> Result<()> {
    let source::ResolvedDownloadSource {
        info,
        info_hash,
        peers,
        is_magnet,
    } = source::resolve_download_source(source).await?;

    let piece_data =
        download_piece_with_retry_from_peers(&peers, &info, &info_hash, piece_index, is_magnet)
            .await?;

    fs::write(output_file, piece_data).await?;
    report::emit_stdout(&ProgressEvent::PieceWritten {
        piece_index,
        output: output_file,
    });
    Ok(())
}

async fn download_piece_with_retry_from_peers(
    peers: &[std::net::SocketAddrV4],
    info: &torrent::TorrentInfo,
    info_hash: &[u8; 20],
    piece_index: usize,
    is_magnet: bool,
) -> Result<Vec<u8>> {
    if peers.is_empty() {
        return Err(TorrentError::NoPeersAvailable);
    }

    let mut candidates = peers.to_vec();
    {
        let mut rng = rand::thread_rng();
        candidates.shuffle(&mut rng);
    }

    let max_attempts = (MAX_RETRIES as usize).max(candidates.len());
    let mut last_error =
        TorrentError::DownloadFailed(format!("Failed to download piece {}", piece_index));

    for peer_addr in candidates.iter().cycle().take(max_attempts) {
        let peer_addr = peer_addr.to_string();
        match try_download_piece(&peer_addr, info, info_hash, piece_index, is_magnet).await {
            Ok(piece_data) => {
                if torrent::verify_piece(info, piece_index, &piece_data) {
                    return Ok(piece_data);
                }
                last_error = TorrentError::PieceVerificationFailed;
                report::emit_stderr(&ProgressEvent::PieceVerificationFailed {
                    piece_index,
                    peer: &peer_addr,
                });
            }
            Err(err) => {
                let error = err.to_string();
                last_error = err;
                report::emit_stderr(&ProgressEvent::PieceDownloadFailed {
                    piece_index,
                    peer: &peer_addr,
                    error,
                });
            }
        }
        sleep(RETRY_DELAY).await;
    }

    Err(last_error)
}

async fn collect_piece_availability(
    peers: &[std::net::SocketAddrV4],
    info: &torrent::TorrentInfo,
    info_hash: &[u8; 20],
    is_magnet: bool,
) -> Vec<usize> {
    let mut availability = vec![0; info.pieces.len()];

    for peer_addr in peers {
        match fetch_peer_bitfield(&peer_addr.to_string(), info, info_hash, is_magnet).await {
            Ok(bitfield) => {
                for (piece_index, has_piece) in bitfield.into_iter().enumerate() {
                    if has_piece {
                        availability[piece_index] += 1;
                    }
                }
            }
            Err(err) => {
                let peer = peer_addr.to_string();
                report::emit_stderr(&ProgressEvent::BitfieldProbeFailed {
                    peer: &peer,
                    error: err.to_string(),
                });
            }
        }
    }

    availability
}

async fn fetch_peer_bitfield(
    peer_addr: &str,
    info: &torrent::TorrentInfo,
    info_hash: &[u8; 20],
    is_magnet: bool,
) -> Result<Vec<bool>> {
    let peer_id: [u8; 20] = peer_id::generate_peer_id();
    let mut peer = peer::Peer::new(peer_addr).await?;
    peer.enable_tcp_nodelay().await?;
    peer.handshake(info_hash, &peer_id).await?;

    if is_magnet {
        return Ok(vec![true; info.pieces.len()]);
    }

    peer.receive_bitfield(info.pieces.len()).await
}

fn sort_missing_pieces_by_availability(missing_pieces: &mut [usize], piece_availability: &[usize]) {
    missing_pieces.sort_by_key(|&piece_index| {
        let availability = piece_availability.get(piece_index).copied().unwrap_or(0);
        (availability == 0, availability, piece_index)
    });
}

async fn try_download_piece(
    peer_addr: &str,
    info: &torrent::TorrentInfo,
    info_hash: &[u8; 20],
    piece_index: usize,
    is_magnet: bool,
) -> Result<Vec<u8>> {
    let peer_id: [u8; 20] = peer_id::generate_peer_id();
    let mut peer = peer::Peer::new(peer_addr).await?;

    peer.enable_tcp_nodelay().await?;
    peer.handshake(info_hash, &peer_id).await?;

    download_piece_from_peer(&mut peer, info, piece_index, is_magnet).await
}

async fn download_piece_from_peer(
    peer: &mut peer::Peer,
    info: &torrent::TorrentInfo,
    piece_index: usize,
    is_magnet: bool,
) -> Result<Vec<u8>> {
    const BLOCK_SIZE: usize = 1 << 14;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
    const MAX_BLOCK_RETRIES: usize = 3;

    let piece_length = if piece_index == info.pieces.len() - 1 {
        info.length as usize - (info.pieces.len() - 1) * (info.piece_length as usize)
    } else {
        info.piece_length as usize
    };

    let mut piece_data = Vec::with_capacity(piece_length);
    let num_blocks = piece_length.div_ceil(BLOCK_SIZE);

    if !is_magnet {
        let _ = peer.receive_bitfield(info.pieces.len()).await?;
        if matches!(peer.has_piece(piece_index), Some(false)) {
            return Err(TorrentError::DownloadFailed(format!(
                "Peer does not advertise piece {}",
                piece_index
            )));
        }
    }
    peer.send_interested().await?;
    peer.receive_unchoke(Some(info.pieces.len())).await?;

    for block_index in 0..num_blocks {
        let begin = block_index * BLOCK_SIZE;
        let length = std::cmp::min(BLOCK_SIZE, piece_length - begin);

        for retry in 0..MAX_BLOCK_RETRIES {
            peer.request_block(piece_index, begin, length).await?;

            match timeout(
                REQUEST_TIMEOUT,
                peer.receive_block(piece_index, begin, Some(info.pieces.len())),
            )
            .await
            {
                Ok(Ok(block)) => {
                    if block.len() != length {
                        return Err(TorrentError::UnexpectedBlockData);
                    }
                    piece_data.extend_from_slice(&block);
                    break;
                }
                Ok(Err(TorrentError::UnexpectedMessage(msg))) => {
                    eprintln!(
                        "Unexpected message while downloading piece {} block {}: {:?}",
                        piece_index, block_index, msg
                    );
                    if retry == MAX_BLOCK_RETRIES - 1 {
                        return Err(TorrentError::UnexpectedMessage(msg));
                    }
                }
                Ok(Err(err)) => {
                    if retry == MAX_BLOCK_RETRIES - 1 {
                        return Err(err);
                    }
                }
                Err(_) => {
                    if retry == MAX_BLOCK_RETRIES - 1 {
                        return Err(TorrentError::ConnectionTimeout);
                    }
                }
            }
        }
    }

    if piece_data.len() != piece_length {
        return Err(TorrentError::UnexpectedBlockData);
    }

    Ok(piece_data)
}

#[cfg(test)]
mod tests {
    use super::sort_missing_pieces_by_availability;

    #[test]
    fn prefers_rarest_nonzero_pieces_first_and_unknown_last() {
        let mut missing = vec![0, 1, 2, 3];
        let availability = vec![4, 1, 0, 2];

        sort_missing_pieces_by_availability(&mut missing, &availability);

        assert_eq!(missing, vec![1, 3, 0, 2]);
    }
}
