use crate::{
    error::{Result, TorrentError},
    peer, peer_id,
    report::{self, ProgressEvent},
    source,
    storage::DownloadStorage,
    torrent,
};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};
use tokio::{fs, time::timeout};

const MAX_RETRIES: u32 = 5;
const WORKER_COUNT: usize = 10;
const RETRY_DELAY: Duration = Duration::from_secs(2);
const PIECE_REQUEUE_LIMIT: u8 = 1;
const COOLDOWN_ATTEMPTS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PieceTask {
    piece_index: usize,
    retries_left: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerFailureKind {
    Generic,
    Timeout,
    MissingPiece,
    Verification,
}

#[derive(Debug, Clone, Default)]
struct PeerHealth {
    successes: usize,
    failures: usize,
    timeout_streak: usize,
    cooldown_until: Option<Instant>,
    unavailable_pieces: HashSet<usize>,
    known_pieces: Option<Vec<bool>>,
    last_error_kind: Option<PeerFailureKind>,
}

pub async fn download_file(output_file: &str, source: &str) -> Result<()> {
    download_file_with_shutdown(output_file, source, std::future::pending()).await
}

pub async fn download_file_with_shutdown<F>(
    output_file: &str,
    source: &str,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()>,
{
    let source::ResolvedDownloadSource {
        info,
        info_hash,
        peers,
        is_magnet,
        selected_tracker,
        tracker_warnings,
    } = source::resolve_download_source(source).await?;

    let storage = DownloadStorage::open(output_file, &info).await?;
    report::emit_stdout(&ProgressEvent::ResumeLoaded {
        completed_pieces: storage.completed_piece_count(),
        total_pieces: storage.total_piece_count(),
    });
    if let Some(tracker) = selected_tracker.as_deref() {
        report::emit_stdout(&ProgressEvent::TrackerSelected { tracker });
    }
    for warning in &tracker_warnings {
        report::emit_stderr(&ProgressEvent::TrackerWarning { message: warning });
    }

    let peer_health = Arc::new(Mutex::new(initialize_peer_health(&peers)));
    let piece_availability = Arc::new(Mutex::new(
        collect_piece_availability(&peers, &info, &info_hash, is_magnet, &peer_health).await,
    ));
    let mut missing_pieces = storage.missing_piece_indices();
    {
        let availability = piece_availability.lock().await;
        sort_missing_pieces_by_availability(&mut missing_pieces, &availability);
    }
    if missing_pieces.is_empty() {
        let finalized_path = storage.finalize().await?;
        report::emit_stdout(&ProgressEvent::DownloadFinalized {
            output: finalized_path.to_string_lossy().as_ref(),
        });
        return Ok(());
    }

    let piece_tasks = missing_pieces
        .into_iter()
        .map(|piece_index| PieceTask {
            piece_index,
            retries_left: PIECE_REQUEUE_LIMIT,
        })
        .collect::<Vec<_>>();
    let piece_queue = Arc::new(Mutex::new(VecDeque::from(piece_tasks)));
    let storage = Arc::new(Mutex::new(storage));
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let mut join_set = JoinSet::new();
    let worker_count = piece_queue.lock().await.len().min(WORKER_COUNT);

    for _ in 0..worker_count {
        let piece_queue = Arc::clone(&piece_queue);
        let storage = Arc::clone(&storage);
        let peer_health = Arc::clone(&peer_health);
        let piece_availability = Arc::clone(&piece_availability);
        let shutdown_requested = Arc::clone(&shutdown_requested);
        let peers = peers.clone();
        let info = info.clone();

        join_set.spawn(async move {
            loop {
                let task = dequeue_piece_task(&piece_queue, &shutdown_requested).await;

                let Some(task) = task else {
                    break Ok::<(), TorrentError>(());
                };

                match download_piece_with_retry_from_peers(
                    &peers,
                    &info,
                    &info_hash,
                    task.piece_index,
                    is_magnet,
                    &peer_health,
                    &piece_availability,
                )
                .await
                {
                    Ok(piece_data) => {
                        let mut storage = storage.lock().await;
                        storage
                            .write_verified_piece(&info, task.piece_index, &piece_data)
                            .await?;
                        report::emit_stdout(&ProgressEvent::PieceStored {
                            piece_index: task.piece_index,
                            completed_pieces: storage.completed_piece_count(),
                            total_pieces: storage.total_piece_count(),
                        });
                    }
                    Err(err) if task.retries_left > 0 => {
                        if shutdown_requested.load(Ordering::Relaxed) {
                            break Err(err);
                        }
                        report::emit_stderr(&ProgressEvent::PieceDownloadFailed {
                            piece_index: task.piece_index,
                            peer: "<requeue>",
                            error: format!("requeued after failure: {err}"),
                        });
                        requeue_piece_task(
                            &piece_queue,
                            &piece_availability,
                            PieceTask {
                                piece_index: task.piece_index,
                                retries_left: task.retries_left - 1,
                            },
                        )
                        .await;
                    }
                    Err(err) => break Err(err),
                }
            }
        });
    }

    tokio::pin!(shutdown);
    let mut interrupted = false;
    let mut interruption_error: Option<TorrentError> = None;

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                interrupted = true;
                shutdown_requested.store(true, Ordering::Relaxed);
            }
            result = join_set.join_next() => {
                match result {
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(err))) => {
                        if interrupted {
                            interruption_error = Some(err);
                            continue;
                        }
                        return Err(err)
                    }
                    Some(Err(err)) => {
                        if interrupted {
                            interruption_error = Some(TorrentError::DownloadFailed(format!(
                                "A download task failed: {}",
                                err
                            )));
                            continue;
                        }
                        return Err(TorrentError::DownloadFailed(format!(
                            "A download task failed: {}",
                            err
                        )));
                    }
                    None => break,
                }
            }
        }
    }

    if interrupted {
        while join_set.join_next().await.is_some() {}
        report::emit_stderr(&ProgressEvent::DownloadInterrupted {
            output: output_file,
        });
        return Err(interruption_error.unwrap_or_else(|| {
            TorrentError::DownloadFailed("Interrupted; partial state preserved for resume".into())
        }));
    }

    let storage = Arc::try_unwrap(storage)
        .map_err(|_| TorrentError::DownloadFailed("Failed to unwrap download storage".into()))?
        .into_inner();

    if !storage.is_complete() {
        return Err(TorrentError::DownloadFailed(
            "Some pieces failed to download".into(),
        ));
    }

    let finalized_path = storage.finalize().await?;
    report::emit_stdout(&ProgressEvent::DownloadFinalized {
        output: finalized_path.to_string_lossy().as_ref(),
    });
    Ok(())
}

pub async fn download_piece(output_file: &str, source: &str, piece_index: usize) -> Result<()> {
    let source::ResolvedDownloadSource {
        info,
        info_hash,
        peers,
        is_magnet,
        selected_tracker,
        tracker_warnings,
    } = source::resolve_download_source(source).await?;

    if let Some(tracker) = selected_tracker.as_deref() {
        report::emit_stdout(&ProgressEvent::TrackerSelected { tracker });
    }
    for warning in &tracker_warnings {
        report::emit_stderr(&ProgressEvent::TrackerWarning { message: warning });
    }

    let peer_health = Arc::new(Mutex::new(initialize_peer_health(&peers)));
    let piece_availability = Arc::new(Mutex::new(
        collect_piece_availability(&peers, &info, &info_hash, is_magnet, &peer_health).await,
    ));
    let piece_data = download_piece_with_retry_from_peers(
        &peers,
        &info,
        &info_hash,
        piece_index,
        is_magnet,
        &peer_health,
        &piece_availability,
    )
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
    peer_health: &Arc<Mutex<HashMap<String, PeerHealth>>>,
    piece_availability: &Arc<Mutex<Vec<usize>>>,
) -> Result<Vec<u8>> {
    if peers.is_empty() {
        return Err(TorrentError::NoPeersAvailable);
    }

    let candidate_order = {
        let health = peer_health.lock().await;
        build_ranked_peer_list(peers, &health, piece_index, MAX_RETRIES as usize)
    };

    let mut last_error =
        TorrentError::DownloadFailed(format!("Failed to download piece {}", piece_index));
    let attempt_started_at = Instant::now();

    for peer_addr in candidate_order {
        let peer_addr = peer_addr.to_string();
        match try_download_piece(&peer_addr, info, info_hash, piece_index, is_magnet).await {
            Ok(piece_data) => {
                if torrent::verify_piece(info, piece_index, &piece_data) {
                    let mut health = peer_health.lock().await;
                    record_peer_success(&mut health, &peer_addr);
                    let mut availability = piece_availability.lock().await;
                    note_peer_piece_success(
                        &mut health,
                        &mut availability,
                        &peer_addr,
                        piece_index,
                        info.pieces.len(),
                    );
                    return Ok(piece_data);
                }
                last_error = TorrentError::PieceVerificationFailed;
                report::emit_stderr(&ProgressEvent::PieceVerificationFailed {
                    piece_index,
                    peer: &peer_addr,
                });
                let mut health = peer_health.lock().await;
                let mut availability = piece_availability.lock().await;
                record_peer_failure(
                    &mut health,
                    &mut availability,
                    &peer_addr,
                    piece_index,
                    PeerFailureKind::Verification,
                    attempt_started_at,
                );
            }
            Err(err) => {
                let error = err.to_string();
                let failure_kind = classify_failure(&err);
                last_error = err;
                report::emit_stderr(&ProgressEvent::PieceDownloadFailed {
                    piece_index,
                    peer: &peer_addr,
                    error,
                });
                let mut health = peer_health.lock().await;
                let mut availability = piece_availability.lock().await;
                record_peer_failure(
                    &mut health,
                    &mut availability,
                    &peer_addr,
                    piece_index,
                    failure_kind,
                    attempt_started_at,
                );
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
    peer_health: &Arc<Mutex<HashMap<String, PeerHealth>>>,
) -> Vec<usize> {
    let mut availability = vec![0; info.pieces.len()];

    for peer_addr in peers {
        match fetch_peer_bitfield(&peer_addr.to_string(), info, info_hash, is_magnet).await {
            Ok(bitfield) => {
                let mut health = peer_health.lock().await;
                let peer = peer_addr.to_string();
                note_peer_bitfield(&mut health, &mut availability, &peer, bitfield);
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
            return Err(TorrentError::PeerDoesNotAdvertisePiece(piece_index));
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

fn initialize_peer_health(peers: &[std::net::SocketAddrV4]) -> HashMap<String, PeerHealth> {
    peers
        .iter()
        .map(|peer| (peer.to_string(), PeerHealth::default()))
        .collect()
}

async fn dequeue_piece_task(
    piece_queue: &Arc<Mutex<VecDeque<PieceTask>>>,
    shutdown_requested: &Arc<AtomicBool>,
) -> Option<PieceTask> {
    let mut queue = piece_queue.lock().await;
    if shutdown_requested.load(Ordering::Relaxed) {
        None
    } else {
        queue.pop_front()
    }
}

fn build_ranked_peer_list(
    peers: &[std::net::SocketAddrV4],
    health: &HashMap<String, PeerHealth>,
    piece_index: usize,
    limit: usize,
) -> Vec<std::net::SocketAddrV4> {
    let mut peers = peers.to_vec();

    let now = Instant::now();
    peers.sort_by_key(|peer| {
        let key = health.get(&peer.to_string());
        let in_cooldown = key
            .and_then(|h| h.cooldown_until)
            .is_some_and(|until| until > now);
        let piece_state_rank = key
            .and_then(|h| piece_known_rank(h, piece_index))
            .unwrap_or(1u8);
        let failures = key.map_or(0, |h| h.failures);
        let timeouts = key.map_or(0, |h| h.timeout_streak);
        let successes = key.map_or(0, |h| h.successes);
        let known_piece_count = key.map_or(0, known_piece_count);
        let last_error_rank = key.and_then(error_rank).unwrap_or(0u8);
        (
            in_cooldown,
            piece_state_rank,
            failures,
            timeouts,
            Reverse(successes),
            Reverse(known_piece_count),
            last_error_rank,
            peer.to_string(),
        )
    });

    let take_count = limit.min(peers.len());
    peers.into_iter().take(take_count).collect()
}

fn piece_known_rank(health: &PeerHealth, piece_index: usize) -> Option<u8> {
    if health.unavailable_pieces.contains(&piece_index) {
        return Some(2);
    }

    health.known_pieces.as_ref().map(|pieces| {
        if pieces.get(piece_index).copied().unwrap_or(false) {
            0
        } else {
            2
        }
    })
}

fn known_piece_count(health: &PeerHealth) -> usize {
    health
        .known_pieces
        .as_ref()
        .map_or(0, |pieces| pieces.iter().filter(|known| **known).count())
}

fn error_rank(health: &PeerHealth) -> Option<u8> {
    health.last_error_kind.map(|kind| match kind {
        PeerFailureKind::Generic => 0,
        PeerFailureKind::Timeout => 1,
        PeerFailureKind::Verification => 2,
        PeerFailureKind::MissingPiece => 3,
    })
}

fn record_peer_success(health: &mut HashMap<String, PeerHealth>, peer: &str) {
    let entry = health.entry(peer.to_string()).or_default();
    entry.successes += 1;
    entry.timeout_streak = 0;
    entry.cooldown_until = None;
    entry.last_error_kind = None;
}

fn note_peer_piece_success(
    health: &mut HashMap<String, PeerHealth>,
    availability: &mut [usize],
    peer: &str,
    piece_index: usize,
    piece_count: usize,
) {
    let entry = health.entry(peer.to_string()).or_default();
    let previous = entry
        .known_pieces
        .as_ref()
        .and_then(|pieces| pieces.get(piece_index).copied());
    let pieces = entry
        .known_pieces
        .get_or_insert_with(|| vec![false; piece_count]);
    if piece_index < pieces.len() {
        pieces[piece_index] = true;
    }
    update_piece_availability_count(availability, piece_index, previous, Some(true));
    entry.unavailable_pieces.remove(&piece_index);
}

fn note_peer_bitfield(
    health: &mut HashMap<String, PeerHealth>,
    availability: &mut [usize],
    peer: &str,
    bitfield: Vec<bool>,
) {
    let entry = health.entry(peer.to_string()).or_default();
    let previous = entry.known_pieces.clone();
    entry.known_pieces = Some(bitfield);
    update_availability_from_bitfield(
        availability,
        previous.as_deref(),
        entry.known_pieces.as_deref(),
    );
}

fn record_peer_missing_piece(
    health: &mut HashMap<String, PeerHealth>,
    availability: &mut [usize],
    peer: &str,
    piece_index: usize,
    piece_count: usize,
) {
    let entry = health.entry(peer.to_string()).or_default();
    let previous = entry
        .known_pieces
        .as_ref()
        .and_then(|pieces| pieces.get(piece_index).copied());
    let pieces = entry
        .known_pieces
        .get_or_insert_with(|| vec![false; piece_count]);
    if piece_index < pieces.len() {
        pieces[piece_index] = false;
    }
    update_piece_availability_count(availability, piece_index, previous, Some(false));
    entry.unavailable_pieces.insert(piece_index);
}

fn record_peer_failure(
    health: &mut HashMap<String, PeerHealth>,
    availability: &mut [usize],
    peer: &str,
    piece_index: usize,
    failure_kind: PeerFailureKind,
    attempt_started_at: Instant,
) {
    let entry = health.entry(peer.to_string()).or_default();
    entry.failures += 1;
    entry.last_error_kind = Some(failure_kind);

    match failure_kind {
        PeerFailureKind::Timeout => {
            entry.timeout_streak += 1;
            entry.cooldown_until = Some(attempt_started_at + RETRY_DELAY * COOLDOWN_ATTEMPTS);
        }
        PeerFailureKind::MissingPiece => {
            let piece_count = entry
                .known_pieces
                .as_ref()
                .map_or(piece_index + 1, |pieces| pieces.len().max(piece_index + 1));
            record_peer_missing_piece(health, availability, peer, piece_index, piece_count);
            let entry = health.entry(peer.to_string()).or_default();
            entry.cooldown_until = Some(attempt_started_at + RETRY_DELAY);
        }
        PeerFailureKind::Verification => {
            entry.cooldown_until = Some(attempt_started_at + RETRY_DELAY * (COOLDOWN_ATTEMPTS + 1));
        }
        PeerFailureKind::Generic => {
            entry.timeout_streak = 0;
            entry.cooldown_until = Some(attempt_started_at + RETRY_DELAY);
        }
    }
}

fn update_piece_availability_count(
    availability: &mut [usize],
    piece_index: usize,
    previous: Option<bool>,
    next: Option<bool>,
) {
    if piece_index >= availability.len() || previous == next {
        return;
    }

    if previous == Some(true) && availability[piece_index] > 0 {
        availability[piece_index] -= 1;
    }
    if next == Some(true) {
        availability[piece_index] += 1;
    }
}

fn update_availability_from_bitfield(
    availability: &mut [usize],
    previous: Option<&[bool]>,
    next: Option<&[bool]>,
) {
    let max_len = previous
        .map_or(0, |bits| bits.len())
        .max(next.map_or(0, |bits| bits.len()))
        .min(availability.len());

    for piece_index in 0..max_len {
        let previous_value = previous.and_then(|bits| bits.get(piece_index).copied());
        let next_value = next.and_then(|bits| bits.get(piece_index).copied());
        update_piece_availability_count(availability, piece_index, previous_value, next_value);
    }
}

async fn requeue_piece_task(
    piece_queue: &Arc<Mutex<VecDeque<PieceTask>>>,
    piece_availability: &Arc<Mutex<Vec<usize>>>,
    task: PieceTask,
) {
    let availability = piece_availability.lock().await.clone();
    let mut queue = piece_queue.lock().await;
    queue.push_back(task);
    queue.make_contiguous().sort_by_key(|task| {
        let availability = availability.get(task.piece_index).copied().unwrap_or(0);
        (availability == 0, availability, task.piece_index)
    });
}

fn classify_failure(err: &TorrentError) -> PeerFailureKind {
    match err {
        TorrentError::ConnectionTimeout => PeerFailureKind::Timeout,
        TorrentError::PieceVerificationFailed => PeerFailureKind::Verification,
        TorrentError::PeerDoesNotAdvertisePiece(_) => PeerFailureKind::MissingPiece,
        _ => PeerFailureKind::Generic,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_ranked_peer_list, classify_failure, dequeue_piece_task, error_rank,
        initialize_peer_health, known_piece_count, note_peer_bitfield, record_peer_failure,
        record_peer_missing_piece, requeue_piece_task, sort_missing_pieces_by_availability,
        update_availability_from_bitfield, PeerFailureKind, PieceTask,
    };
    use crate::error::TorrentError;
    use std::collections::VecDeque;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::{atomic::AtomicBool, Arc};
    use std::time::Instant;
    use tokio::sync::Mutex;

    #[test]
    fn prefers_rarest_nonzero_pieces_first_and_unknown_last() {
        let mut missing = vec![0, 1, 2, 3];
        let availability = vec![4, 1, 0, 2];

        sort_missing_pieces_by_availability(&mut missing, &availability);

        assert_eq!(missing, vec![1, 3, 0, 2]);
    }

    #[test]
    fn shorter_availability_vectors_sort_unknown_pieces_last() {
        let mut missing = vec![0, 1, 2, 3];
        let availability = vec![3, 1];

        sort_missing_pieces_by_availability(&mut missing, &availability);

        assert_eq!(missing, vec![1, 0, 2, 3]);
    }

    #[test]
    fn availability_probe_counts_only_advertised_pieces() {
        let mut availability = vec![0; 4];
        update_availability_from_bitfield(
            &mut availability,
            None,
            Some(&[true, false, true, false]),
        );
        update_availability_from_bitfield(
            &mut availability,
            None,
            Some(&[false, true, true, false]),
        );

        assert_eq!(availability, vec![1, 1, 2, 0]);
    }

    #[test]
    fn cooldown_and_missing_piece_demote_peer() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
        ];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 4];
        record_peer_failure(
            &mut health,
            &mut availability,
            &peers[0].to_string(),
            3,
            PeerFailureKind::MissingPiece,
            Instant::now(),
        );

        let ranked = build_ranked_peer_list(&peers, &health, 3, peers.len());
        assert_eq!(ranked[0], peers[1]);
    }

    #[test]
    fn no_peers_error_is_classified_as_generic() {
        assert_eq!(
            classify_failure(&TorrentError::NoPeersAvailable),
            PeerFailureKind::Generic
        );
    }

    #[test]
    fn timeout_failures_increase_cooldown() {
        let peers = vec![SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881)];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 1];
        let peer = peers[0].to_string();
        let started_at = Instant::now();
        record_peer_failure(
            &mut health,
            &mut availability,
            &peer,
            0,
            PeerFailureKind::Timeout,
            started_at,
        );

        let peer_health = health.get(&peer).expect("peer health should exist");
        assert_eq!(peer_health.timeout_streak, 1);
        assert!(peer_health.cooldown_until.is_some());
        assert!(peer_health.cooldown_until.expect("cooldown should exist") > started_at);
    }

    #[test]
    fn structured_missing_piece_errors_are_classified_correctly() {
        assert_eq!(
            classify_failure(&TorrentError::PeerDoesNotAdvertisePiece(7)),
            PeerFailureKind::MissingPiece
        );
    }

    #[test]
    fn live_bitfield_update_improves_ranking_for_known_piece() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
        ];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 2];
        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peers[1].to_string(),
            vec![false, true],
        );

        let ranked_before = build_ranked_peer_list(&peers, &health, 1, peers.len());
        assert_eq!(ranked_before[0], peers[1]);

        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peers[0].to_string(),
            vec![false, true],
        );
        let ranked_after = build_ranked_peer_list(&peers, &health, 1, peers.len());
        assert!(ranked_after.contains(&peers[0]));
        assert!(ranked_after.contains(&peers[1]));
    }

    #[test]
    fn peer_with_more_known_good_pieces_wins_ties() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
        ];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 4];

        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peers[0].to_string(),
            vec![true, false, false, false],
        );
        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peers[1].to_string(),
            vec![true, true, true, false],
        );

        let ranked = build_ranked_peer_list(&peers, &health, 0, peers.len());
        assert_eq!(ranked[0], peers[1]);
    }

    #[test]
    fn last_error_rank_prefers_less_severe_recent_failures() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
        ];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 2];
        let now = Instant::now();

        record_peer_failure(
            &mut health,
            &mut availability,
            &peers[0].to_string(),
            0,
            PeerFailureKind::Timeout,
            now,
        );
        record_peer_failure(
            &mut health,
            &mut availability,
            &peers[1].to_string(),
            0,
            PeerFailureKind::MissingPiece,
            now,
        );

        let ranked = build_ranked_peer_list(&peers, &health, 1, peers.len());
        assert_eq!(ranked[0], peers[0]);
    }

    #[test]
    fn missing_piece_demotes_only_that_piece() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
        ];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 4];
        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peers[0].to_string(),
            vec![true, true, true, true],
        );
        record_peer_missing_piece(&mut health, &mut availability, &peers[0].to_string(), 2, 4);

        let ranked_for_missing = build_ranked_peer_list(&peers, &health, 2, peers.len());
        assert_eq!(ranked_for_missing[0], peers[1]);

        let ranked_for_other = build_ranked_peer_list(&peers, &health, 1, peers.len());
        assert_eq!(ranked_for_other[0], peers[0]);
    }

    #[test]
    fn runtime_negative_evidence_beats_earlier_positive_evidence() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
        ];
        let mut health = initialize_peer_health(&peers);
        let mut availability = vec![0; 3];
        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peers[0].to_string(),
            vec![false, false, true],
        );
        record_peer_missing_piece(&mut health, &mut availability, &peers[0].to_string(), 2, 3);

        let ranked = build_ranked_peer_list(&peers, &health, 2, peers.len());
        assert_eq!(ranked[0], peers[1]);
    }

    #[test]
    fn helper_counts_and_ranks_are_stable() {
        let peer = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881);
        let mut health = initialize_peer_health(&[peer]);
        let mut availability = vec![0; 3];

        note_peer_bitfield(
            &mut health,
            &mut availability,
            &peer.to_string(),
            vec![true, false, true],
        );
        let health = health.get(&peer.to_string()).expect("peer should exist");
        assert_eq!(known_piece_count(health), 2);
        assert_eq!(error_rank(health), None);
    }

    #[test]
    fn equal_weight_peers_have_deterministic_order() {
        let peers = vec![
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882),
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881),
        ];
        let health = initialize_peer_health(&peers);

        let ranked = build_ranked_peer_list(&peers, &health, 0, peers.len());

        assert_eq!(
            ranked[0],
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6881)
        );
        assert_eq!(
            ranked[1],
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 6882)
        );
    }

    #[test]
    fn live_bitfield_updates_adjust_piece_availability_counts() {
        let mut availability = vec![0; 4];

        update_availability_from_bitfield(
            &mut availability,
            None,
            Some(&[false, true, false, true]),
        );
        assert_eq!(availability, vec![0, 1, 0, 1]);

        update_availability_from_bitfield(
            &mut availability,
            Some(&[false, true, false, true]),
            Some(&[true, true, false, false]),
        );
        assert_eq!(availability, vec![1, 1, 0, 0]);
    }

    #[tokio::test]
    async fn requeued_pieces_are_resorted_by_updated_availability() {
        let queue = Arc::new(Mutex::new(VecDeque::from([
            PieceTask {
                piece_index: 3,
                retries_left: 0,
            },
            PieceTask {
                piece_index: 1,
                retries_left: 0,
            },
        ])));
        let piece_availability = Arc::new(Mutex::new(vec![0, 2, 0, 1]));

        requeue_piece_task(
            &queue,
            &piece_availability,
            PieceTask {
                piece_index: 2,
                retries_left: 1,
            },
        )
        .await;

        let queue = queue.lock().await;
        let order = queue
            .iter()
            .map(|task| task.piece_index)
            .collect::<Vec<_>>();
        assert_eq!(order, vec![3, 1, 2]);
    }

    #[tokio::test]
    async fn dequeue_respects_shutdown_before_starting_new_work() {
        let queue = Arc::new(Mutex::new(VecDeque::<PieceTask>::from([PieceTask {
            piece_index: 3,
            retries_left: 1,
        }])));
        let shutdown_requested = Arc::new(AtomicBool::new(true));

        let task = dequeue_piece_task(&queue, &shutdown_requested).await;
        assert!(task.is_none());
        assert_eq!(queue.lock().await.len(), 1);
    }
}
