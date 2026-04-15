use crate::{
    error::{Result, TorrentError},
    torrent::{self, TorrentInfo, TorrentLayout, TorrentLayoutFile},
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct DownloadStorage {
    finalize_target: FinalizeTarget,
    part_path: PathBuf,
    state_path: PathBuf,
    file: File,
    state: ResumeState,
}

#[derive(Debug, Clone)]
enum FinalizeTarget {
    SingleFile {
        output_path: PathBuf,
    },
    MultiFile {
        root_dir: PathBuf,
        files: Vec<TorrentLayoutFile>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ResumeState {
    info_hash: String,
    piece_length: i64,
    total_length: i64,
    completed_pieces: Vec<bool>,
}

impl DownloadStorage {
    pub async fn open<P: AsRef<Path>>(output_path: P, info: &TorrentInfo) -> Result<Self> {
        if !self_describes_torrent(info) {
            return Err(TorrentError::DownloadFailed(
                "Torrent metadata is incomplete for storage initialization".into(),
            ));
        }

        let output_path = output_path.as_ref().to_path_buf();
        let finalize_target = build_finalize_target(&output_path, info)?;
        let (part_path, state_path) = working_paths(&finalize_target, &output_path, info)?;

        ensure_target_is_available(&finalize_target, &part_path, &state_path).await?;
        ensure_parent_directory(&part_path).await?;
        ensure_parent_directory(&state_path).await?;

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&part_path)
            .await?;
        file.set_len(info.length as u64).await?;

        let mut storage = Self {
            finalize_target,
            part_path,
            state_path,
            file,
            state: ResumeState::new(info),
        };

        storage.load_or_initialize_state(info).await?;
        storage.reconcile_completed_pieces(info).await?;
        storage.persist_state().await?;

        Ok(storage)
    }

    pub fn missing_piece_indices(&self) -> Vec<usize> {
        self.state
            .completed_pieces
            .iter()
            .enumerate()
            .filter_map(|(index, completed)| (!completed).then_some(index))
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.state
            .completed_pieces
            .iter()
            .all(|completed| *completed)
    }

    pub fn completed_piece_count(&self) -> usize {
        self.state
            .completed_pieces
            .iter()
            .filter(|completed| **completed)
            .count()
    }

    pub fn total_piece_count(&self) -> usize {
        self.state.completed_pieces.len()
    }

    pub async fn write_piece(
        &mut self,
        info: &TorrentInfo,
        piece_index: usize,
        data: &[u8],
    ) -> Result<()> {
        if !torrent::verify_piece(info, piece_index, data) {
            return Err(TorrentError::PieceVerificationFailed);
        }

        let expected_len = piece_length_for(info, piece_index)?;
        if data.len() != expected_len {
            return Err(TorrentError::UnexpectedBlockData);
        }

        self.seek_to_piece(info, piece_index).await?;
        self.file.write_all(data).await?;
        self.file.flush().await?;
        self.file.sync_data().await?;

        self.state.completed_pieces[piece_index] = true;
        self.persist_state().await?;
        Ok(())
    }

    pub async fn finalize(mut self) -> Result<PathBuf> {
        if !self.is_complete() {
            return Err(TorrentError::DownloadFailed(
                "Cannot finalize incomplete download".into(),
            ));
        }

        self.file.flush().await?;
        drop(self.file);
        let finalized_path: PathBuf;

        match &self.finalize_target {
            FinalizeTarget::SingleFile { output_path } => {
                if fs::try_exists(output_path).await? {
                    return Err(TorrentError::DownloadFailed(format!(
                        "Refusing to overwrite existing file: {}",
                        output_path.display()
                    )));
                }
                ensure_parent_directory(output_path).await?;
                fs::rename(&self.part_path, output_path).await?;
                finalized_path = output_path.clone();
            }
            FinalizeTarget::MultiFile { root_dir, files } => {
                if fs::try_exists(root_dir).await? {
                    return Err(TorrentError::DownloadFailed(format!(
                        "Refusing to overwrite existing directory: {}",
                        root_dir.display()
                    )));
                }

                ensure_parent_directory(root_dir).await?;
                let staging_dir = append_suffix(root_dir, ".staging");
                if fs::try_exists(&staging_dir).await? {
                    fs::remove_dir_all(&staging_dir).await?;
                }
                fs::create_dir_all(&staging_dir).await?;

                if let Err(err) =
                    materialize_multi_file_tree(&self.part_path, &staging_dir, files).await
                {
                    let _ = fs::remove_dir_all(&staging_dir).await;
                    return Err(err);
                }

                fs::rename(&staging_dir, root_dir).await?;
                fs::remove_file(&self.part_path).await?;
                finalized_path = root_dir.clone();
            }
        }

        if fs::try_exists(&self.state_path).await? {
            fs::remove_file(&self.state_path).await?;
        }
        Ok(finalized_path)
    }

    async fn load_or_initialize_state(&mut self, info: &TorrentInfo) -> Result<()> {
        if !fs::try_exists(&self.state_path).await? {
            self.state = ResumeState::new(info);
            return Ok(());
        }

        let raw_state = fs::read(&self.state_path).await?;
        let persisted: ResumeState = serde_json::from_slice(&raw_state)?;
        if persisted.matches(info) {
            self.state = persisted;
        } else {
            self.state = ResumeState::new(info);
            self.file.set_len(info.length as u64).await?;
            self.file.seek(std::io::SeekFrom::Start(0)).await?;
        }

        Ok(())
    }

    async fn reconcile_completed_pieces(&mut self, info: &TorrentInfo) -> Result<()> {
        for piece_index in 0..self.state.completed_pieces.len() {
            if !self.state.completed_pieces[piece_index] {
                continue;
            }

            let piece = self.read_piece(info, piece_index).await?;
            if !torrent::verify_piece(info, piece_index, &piece) {
                self.state.completed_pieces[piece_index] = false;
                self.zero_piece(info, piece_index).await?;
            }
        }

        Ok(())
    }

    async fn read_piece(&mut self, info: &TorrentInfo, piece_index: usize) -> Result<Vec<u8>> {
        let length = piece_length_for(info, piece_index)?;
        self.seek_to_piece(info, piece_index).await?;
        let mut buffer = vec![0u8; length];
        self.file.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    async fn zero_piece(&mut self, info: &TorrentInfo, piece_index: usize) -> Result<()> {
        let length = piece_length_for(info, piece_index)?;
        self.seek_to_piece(info, piece_index).await?;
        self.file.write_all(&vec![0u8; length]).await?;
        self.file.flush().await?;
        self.file.sync_data().await?;
        Ok(())
    }

    async fn seek_to_piece(&mut self, info: &TorrentInfo, piece_index: usize) -> Result<()> {
        let offset = piece_offset(info, piece_index)?;
        self.file
            .seek(std::io::SeekFrom::Start(offset as u64))
            .await?;
        Ok(())
    }

    async fn persist_state(&self) -> Result<()> {
        let encoded = serde_json::to_vec_pretty(&self.state)?;
        let tmp_path = append_suffix(&self.state_path, ".tmp");
        let mut temp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .await?;
        temp.write_all(&encoded).await?;
        temp.flush().await?;
        temp.sync_data().await?;
        drop(temp);
        fs::rename(&tmp_path, &self.state_path).await?;
        Ok(())
    }
}

impl ResumeState {
    fn new(info: &TorrentInfo) -> Self {
        Self {
            info_hash: info.info_hash.clone(),
            piece_length: info.piece_length,
            total_length: info.length,
            completed_pieces: vec![false; info.pieces.len()],
        }
    }

    fn matches(&self, info: &TorrentInfo) -> bool {
        self.info_hash == info.info_hash
            && self.piece_length == info.piece_length
            && self.total_length == info.length
            && self.completed_pieces.len() == info.pieces.len()
    }
}

fn self_describes_torrent(info: &TorrentInfo) -> bool {
    info.length > 0 && info.piece_length > 0 && !info.pieces.is_empty() && info.layout.is_some()
}

fn piece_offset(info: &TorrentInfo, piece_index: usize) -> Result<usize> {
    if piece_index >= info.pieces.len() {
        return Err(TorrentError::InvalidResponseFormat(
            "Invalid piece index".into(),
        ));
    }

    Ok(piece_index * info.piece_length as usize)
}

fn piece_length_for(info: &TorrentInfo, piece_index: usize) -> Result<usize> {
    if piece_index >= info.pieces.len() {
        return Err(TorrentError::InvalidResponseFormat(
            "Invalid piece index".into(),
        ));
    }

    if piece_index == info.pieces.len() - 1 {
        Ok(info.length as usize - (info.pieces.len() - 1) * info.piece_length as usize)
    } else {
        Ok(info.piece_length as usize)
    }
}

fn build_finalize_target(output_path: &Path, info: &TorrentInfo) -> Result<FinalizeTarget> {
    match info
        .layout
        .clone()
        .ok_or_else(|| TorrentError::DownloadFailed("Torrent layout is missing".into()))?
    {
        TorrentLayout::SingleFile { .. } => Ok(FinalizeTarget::SingleFile {
            output_path: output_path.to_path_buf(),
        }),
        TorrentLayout::MultiFile { root_name, files } => Ok(FinalizeTarget::MultiFile {
            root_dir: output_path.join(root_name),
            files,
        }),
    }
}

fn working_paths(
    finalize_target: &FinalizeTarget,
    output_path: &Path,
    info: &TorrentInfo,
) -> Result<(PathBuf, PathBuf)> {
    match finalize_target {
        FinalizeTarget::SingleFile { output_path } => Ok((
            append_suffix(output_path, ".part"),
            append_suffix(output_path, ".resume.json"),
        )),
        FinalizeTarget::MultiFile {
            root_dir: _,
            files: _,
        } => {
            let stem = info.name.clone();
            Ok((
                output_path.join(format!("{stem}.part")),
                output_path.join(format!("{stem}.resume.json")),
            ))
        }
    }
}

async fn ensure_target_is_available(
    finalize_target: &FinalizeTarget,
    part_path: &Path,
    state_path: &Path,
) -> Result<()> {
    match finalize_target {
        FinalizeTarget::SingleFile { output_path } => {
            if fs::try_exists(output_path).await?
                && !fs::try_exists(part_path).await?
                && !fs::try_exists(state_path).await?
            {
                return Err(TorrentError::DownloadFailed(format!(
                    "Output path already exists: {}",
                    output_path.display()
                )));
            }
        }
        FinalizeTarget::MultiFile { root_dir, .. } => {
            if fs::try_exists(root_dir).await?
                && !fs::try_exists(part_path).await?
                && !fs::try_exists(state_path).await?
            {
                return Err(TorrentError::DownloadFailed(format!(
                    "Output root already exists: {}",
                    root_dir.display()
                )));
            }
        }
    }

    Ok(())
}

async fn ensure_parent_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();
    path.push(OsString::from(suffix));
    PathBuf::from(path)
}

fn is_safe_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }

    path.components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
}

async fn materialize_multi_file_tree(
    part_path: &Path,
    root_dir: &Path,
    files: &[TorrentLayoutFile],
) -> Result<()> {
    let mut source = File::open(part_path).await?;
    let mut buffer = vec![0u8; 16 * 1024];

    for file in files {
        if !is_safe_relative_path(&file.relative_path) {
            return Err(TorrentError::DownloadFailed(
                "Refusing to materialize unsafe relative path".into(),
            ));
        }
        source
            .seek(std::io::SeekFrom::Start(file.offset as u64))
            .await?;
        let destination = root_dir.join(&file.relative_path);
        ensure_parent_directory(&destination).await?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
            .await?;

        let mut remaining = file.length;
        while remaining > 0 {
            let chunk_len = remaining.min(buffer.len());
            source.read_exact(&mut buffer[..chunk_len]).await?;
            output.write_all(&buffer[..chunk_len]).await?;
            remaining -= chunk_len;
        }
        output.flush().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::DownloadStorage;
    use crate::torrent::{TorrentInfo, TorrentLayout, TorrentLayoutFile};
    use sha1::{Digest, Sha1};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn fake_info() -> TorrentInfo {
        let piece_a = b"abcd";
        let piece_b = b"efgh";

        TorrentInfo {
            trackers: Vec::new(),
            info_hash: "fake-info-hash".into(),
            length: 8,
            name: "fixture.bin".into(),
            piece_length: 4,
            pieces: vec![sha1_bytes(piece_a), sha1_bytes(piece_b)],
            layout: Some(TorrentLayout::SingleFile {
                suggested_name: "fixture.bin".into(),
                length: 8,
            }),
        }
    }

    fn fake_multi_file_info() -> TorrentInfo {
        let piece_a = b"abcd";
        let piece_b = b"efgh";

        TorrentInfo {
            trackers: Vec::new(),
            info_hash: "fake-multi".into(),
            length: 8,
            name: "bundle".into(),
            piece_length: 4,
            pieces: vec![sha1_bytes(piece_a), sha1_bytes(piece_b)],
            layout: Some(TorrentLayout::MultiFile {
                root_name: "bundle".into(),
                files: vec![
                    TorrentLayoutFile {
                        relative_path: PathBuf::from("a.txt"),
                        length: 3,
                        offset: 0,
                    },
                    TorrentLayoutFile {
                        relative_path: PathBuf::from("nested").join("b.txt"),
                        length: 5,
                        offset: 3,
                    },
                ],
            }),
        }
    }

    fn fake_unsafe_multi_file_info() -> TorrentInfo {
        let mut info = fake_multi_file_info();
        info.layout = Some(TorrentLayout::MultiFile {
            root_name: "bundle".into(),
            files: vec![TorrentLayoutFile {
                relative_path: PathBuf::from("..").join("evil.txt"),
                length: 8,
                offset: 0,
            }],
        });
        info
    }

    fn sha1_bytes(bytes: &[u8]) -> [u8; 20] {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        hasher.finalize().into()
    }

    #[tokio::test]
    async fn resumes_completed_pieces_and_finalizes() {
        let dir = tempdir().expect("tempdir should exist");
        let output = dir.path().join("download.bin");
        let info = fake_info();

        let mut storage = DownloadStorage::open(&output, &info)
            .await
            .expect("storage should open");
        assert_eq!(storage.missing_piece_indices(), vec![0, 1]);

        storage
            .write_piece(&info, 0, b"abcd")
            .await
            .expect("piece should write");
        assert_eq!(storage.missing_piece_indices(), vec![1]);
        drop(storage);

        let mut resumed = DownloadStorage::open(&output, &info)
            .await
            .expect("storage should reopen");
        assert_eq!(resumed.missing_piece_indices(), vec![1]);
        resumed
            .write_piece(&info, 1, b"efgh")
            .await
            .expect("second piece should write");
        resumed.finalize().await.expect("download should finalize");

        let final_bytes = tokio::fs::read(&output)
            .await
            .expect("final output should exist");
        assert_eq!(&final_bytes, b"abcdefgh");
        assert!(!output.with_extension("bin.part").exists());
        assert!(!output.with_extension("bin.resume.json").exists());
    }

    #[tokio::test]
    async fn invalidates_corrupted_completed_pieces_on_resume() {
        let dir = tempdir().expect("tempdir should exist");
        let output = dir.path().join("download.bin");
        let info = fake_info();

        let mut storage = DownloadStorage::open(&output, &info)
            .await
            .expect("storage should open");
        storage
            .write_piece(&info, 0, b"abcd")
            .await
            .expect("piece should write");
        drop(storage);

        let part_path = std::path::PathBuf::from(format!("{}.part", output.display()));
        let mut bytes = tokio::fs::read(&part_path)
            .await
            .expect("part file should exist");
        bytes[0] = b'Z';
        tokio::fs::write(&part_path, bytes)
            .await
            .expect("part file should rewrite");

        let resumed = DownloadStorage::open(&output, &info)
            .await
            .expect("storage should reopen");
        assert_eq!(resumed.missing_piece_indices(), vec![0, 1]);
    }

    #[tokio::test]
    async fn finalizes_multi_file_layout_under_destination_root() {
        let dir = tempdir().expect("tempdir should exist");
        let output_root = dir.path().join("downloads");
        let info = fake_multi_file_info();

        let mut storage = DownloadStorage::open(&output_root, &info)
            .await
            .expect("storage should open");
        storage
            .write_piece(&info, 0, b"abcd")
            .await
            .expect("first piece should write");
        storage
            .write_piece(&info, 1, b"efgh")
            .await
            .expect("second piece should write");
        let finalized = storage.finalize().await.expect("finalize should succeed");
        assert_eq!(finalized, output_root.join("bundle"));

        let first = tokio::fs::read(output_root.join("bundle").join("a.txt"))
            .await
            .expect("first file should exist");
        let second = tokio::fs::read(output_root.join("bundle").join("nested").join("b.txt"))
            .await
            .expect("second file should exist");

        assert_eq!(&first, b"abc");
        assert_eq!(&second, b"defgh");
        assert!(!output_root.join("bundle.part").exists());
        assert!(!output_root.join("bundle.resume.json").exists());
    }

    #[tokio::test]
    async fn resumes_multi_file_downloads() {
        let dir = tempdir().expect("tempdir should exist");
        let output_root = dir.path().join("downloads");
        let info = fake_multi_file_info();

        let mut storage = DownloadStorage::open(&output_root, &info)
            .await
            .expect("storage should open");
        storage
            .write_piece(&info, 0, b"abcd")
            .await
            .expect("first piece should write");
        drop(storage);

        let mut resumed = DownloadStorage::open(&output_root, &info)
            .await
            .expect("storage should reopen");
        assert_eq!(resumed.missing_piece_indices(), vec![1]);
        resumed
            .write_piece(&info, 1, b"efgh")
            .await
            .expect("second piece should write");
        let finalized = resumed.finalize().await.expect("finalize should succeed");
        assert_eq!(finalized, output_root.join("bundle"));

        let second = tokio::fs::read(output_root.join("bundle").join("nested").join("b.txt"))
            .await
            .expect("second file should exist");
        assert_eq!(&second, b"defgh");
    }

    #[tokio::test]
    async fn rejects_unsafe_multi_file_layouts() {
        let dir = tempdir().expect("tempdir should exist");
        let output_root = dir.path().join("downloads");
        let info = fake_unsafe_multi_file_info();

        let mut storage = DownloadStorage::open(&output_root, &info)
            .await
            .expect("storage should open");
        storage
            .write_piece(&info, 0, b"abcd")
            .await
            .expect("first piece should write");
        storage
            .write_piece(&info, 1, b"efgh")
            .await
            .expect("second piece should write");

        let err = storage
            .finalize()
            .await
            .expect_err("unsafe path should fail");
        assert!(err.to_string().contains("unsafe"));
        assert!(!output_root.join("evil.txt").exists());
    }
}
