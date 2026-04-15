// torrent.rs

use crate::error::{Result, TorrentError};
use crate::magnet;
use serde::{Deserialize, Serialize};
use serde_bencode::de::from_bytes;
use sha1::{Digest, Sha1};
use std::fs;
use std::path::{Component, Path, PathBuf};
// use std::sync::Arc;

pub use hashes::Hashes;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Torrent {
    pub announce: String,
    #[serde(rename = "announce-list", default)]
    pub announce_list: Vec<Vec<String>>,
    pub info: Info,
}

impl Torrent {
    pub fn info_hash(&self) -> [u8; 20] {
        let info_encoded = serde_bencode::ser::to_bytes(&self.info)
            .expect("Re-encode info section should be fine");
        let mut hasher = Sha1::new();
        hasher.update(&info_encoded);
        hasher.finalize().into()
    }

    pub fn trackers(&self) -> Vec<String> {
        let mut trackers = Vec::new();
        push_tracker(&mut trackers, self.announce.clone());
        for tier in &self.announce_list {
            for tracker in tier {
                push_tracker(&mut trackers, tracker.clone());
            }
        }
        trackers
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Info {
    pub name: String,
    #[serde(rename = "piece length")]
    pub piece_length: usize,
    pub pieces: Hashes,
    #[serde(flatten)]
    pub keys: Keys,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Keys {
    SingleFile { length: usize },
    MultiFile { files: Vec<File> },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct File {
    pub length: usize,
    pub path: Vec<String>,
}

mod hashes {
    // use serde::{Deserialize, Serialize};
    use serde::de::{self, Visitor};
    use serde::ser::Serializer;
    use std::fmt;

    #[derive(Debug, Clone)]
    pub struct Hashes(pub Vec<[u8; 20]>);

    struct HashesVisitor;

    impl<'de> Visitor<'de> for HashesVisitor {
        type Value = Hashes;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a byte string whose length is a multiple of 20")
        }

        fn visit_bytes<E>(self, v: &[u8]) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            if !v.len().is_multiple_of(20) {
                return Err(E::custom(format!("length is {}", v.len())));
            }

            Ok(Hashes(
                v.chunks_exact(20)
                    .map(|slice_20| slice_20.try_into().expect("Slice is exactly 20 bytes"))
                    .collect(),
            ))
        }
    }

    impl<'de> serde::Deserialize<'de> for Hashes {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_bytes(HashesVisitor)
        }
    }

    impl serde::Serialize for Hashes {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let single_slice = self.0.concat();
            serializer.serialize_bytes(&single_slice)
        }
    }
}

pub fn decode_file(file_path: &str) -> Result<Torrent> {
    let content = fs::read(file_path)?;
    let torrent = from_bytes::<Torrent>(&content)
        .map_err(|_| TorrentError::DecodeError("Failed to deserialize Torrent".into()))?;
    Ok(torrent)
}
#[derive(Clone)]
pub struct TorrentInfo {
    pub trackers: Vec<String>,
    pub info_hash: String,
    pub length: i64,
    pub name: String,
    pub piece_length: i64,
    pub pieces: Vec<[u8; 20]>,
    pub layout: Option<TorrentLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentLayout {
    SingleFile {
        suggested_name: String,
        length: usize,
    },
    MultiFile {
        root_name: String,
        files: Vec<TorrentLayoutFile>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentLayoutFile {
    pub relative_path: PathBuf,
    pub length: usize,
    pub offset: usize,
}

impl TorrentInfo {
    pub fn calculate_info_hash(info: &Info) -> [u8; 20] {
        let info_encoded = serde_bencode::to_bytes(info).expect("Failed to encode info dictionary");
        let mut hasher = Sha1::new();
        hasher.update(&info_encoded);
        hasher.finalize().into()
    }

    pub fn calculate_length(info: &Info) -> i64 {
        match &info.keys {
            Keys::SingleFile { length } => *length as i64,
            Keys::MultiFile { files } => files.iter().map(|f| f.length as i64).sum(),
        }
    }

    pub fn build_layout(info: &Info) -> Result<TorrentLayout> {
        match &info.keys {
            Keys::SingleFile { length } => Ok(TorrentLayout::SingleFile {
                suggested_name: info.name.clone(),
                length: *length,
            }),
            Keys::MultiFile { files } => {
                let root_name = validate_path_component(&info.name)?;
                let mut offset = 0usize;
                let mut layout_files = Vec::with_capacity(files.len());

                for file in files {
                    let relative_path = validate_relative_path(&file.path)?;
                    layout_files.push(TorrentLayoutFile {
                        relative_path,
                        length: file.length,
                        offset,
                    });
                    offset += file.length;
                }

                Ok(TorrentLayout::MultiFile {
                    root_name,
                    files: layout_files,
                })
            }
        }
    }

    pub fn validate_metadata(metadata: &[u8], expected_info_hash: &str) -> Result<Self> {
        let info: Info = serde_bencode::from_bytes(metadata)?;
        let calculated_info_hash = hex::encode(Self::calculate_info_hash(&info));

        if calculated_info_hash != expected_info_hash {
            return Err(TorrentError::InvalidInfoHash);
        }

        let layout = Self::build_layout(&info)?;
        let name = info.name.clone();
        Ok(TorrentInfo {
            trackers: Vec::new(),
            info_hash: calculated_info_hash,
            length: Self::calculate_length(&info),
            name,
            piece_length: info.piece_length as i64,
            pieces: info.pieces.0,
            layout: Some(layout),
        })
    }
    pub fn from_magnet(magnet: &magnet::Magnet) -> Result<Self> {
        Ok(TorrentInfo {
            trackers: magnet.trackers.clone(),
            info_hash: magnet.info_hash.clone(),
            length: 0, // Length is unknown from magnet link
            name: magnet.display_name.clone().unwrap_or_default(),
            piece_length: 0,    // Unknown from magnet link
            pieces: Vec::new(), // Unknown from magnet link
            layout: None,
        })
    }
}

pub fn get_info(file_path: &str) -> Result<TorrentInfo> {
    let torrent = decode_file(file_path)?;

    let info_hash = torrent.info_hash();
    let length = TorrentInfo::calculate_length(&torrent.info);
    let layout = TorrentInfo::build_layout(&torrent.info)?;
    let name = torrent.info.name.clone();

    Ok(TorrentInfo {
        trackers: torrent.trackers(),
        info_hash: hex::encode(info_hash),
        length,
        name,
        piece_length: torrent.info.piece_length as i64,
        pieces: torrent.info.pieces.0,
        layout: Some(layout),
    })
}

pub fn get_piece_hash(info: &TorrentInfo, piece_index: usize) -> Result<[u8; 20]> {
    info.pieces
        .get(piece_index)
        .cloned()
        .ok_or_else(|| TorrentError::InvalidResponseFormat("Invalid piece index".into()))
}

pub fn verify_piece(info: &TorrentInfo, piece_index: usize, piece_data: &[u8]) -> bool {
    if let Ok(expected_hash) = get_piece_hash(info, piece_index) {
        let mut hasher = Sha1::new();
        hasher.update(piece_data);
        let actual_hash = hasher.finalize();
        expected_hash == actual_hash.as_slice()
    } else {
        false
    }
}

fn validate_path_component(component: &str) -> Result<String> {
    if component.is_empty() || component == "." || component == ".." {
        return Err(TorrentError::InvalidResponseFormat(
            "Unsafe torrent path component".into(),
        ));
    }

    if component.contains('/') || component.contains('\\') {
        return Err(TorrentError::InvalidResponseFormat(
            "Torrent path component contains a separator".into(),
        ));
    }

    let path = Path::new(component);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(component.to_string()),
        _ => Err(TorrentError::InvalidResponseFormat(
            "Unsafe torrent path component".into(),
        )),
    }
}

fn validate_relative_path(segments: &[String]) -> Result<PathBuf> {
    if segments.is_empty() {
        return Err(TorrentError::InvalidResponseFormat(
            "Multi-file path is empty".into(),
        ));
    }

    let mut path = PathBuf::new();
    for segment in segments {
        path.push(validate_path_component(segment)?);
    }
    Ok(path)
}

fn push_tracker(trackers: &mut Vec<String>, tracker: String) {
    if tracker.is_empty() || trackers.contains(&tracker) {
        return;
    }
    trackers.push(tracker);
}

#[cfg(test)]
mod tests {
    use super::{
        decode_file, get_info, validate_relative_path, verify_piece, Keys, Torrent, TorrentInfo,
        TorrentLayout,
    };
    use std::path::PathBuf;

    fn sample_torrent_path() -> String {
        format!("{}/sample.torrent", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn decodes_sample_torrent_fixture() {
        let torrent = decode_file(&sample_torrent_path()).expect("sample torrent should decode");

        assert_eq!(
            torrent.announce,
            "http://bittorrent-test-tracker.codecrafters.io/announce"
        );
        assert_eq!(torrent.info.name, "sample.txt");
        assert_eq!(torrent.info.pieces.0.len(), 3);
        assert_eq!(
            torrent.trackers(),
            vec!["http://bittorrent-test-tracker.codecrafters.io/announce"]
        );
        assert!(matches!(
            torrent.info.keys,
            Keys::SingleFile { length: 92063 }
        ));
    }

    #[test]
    fn derives_sample_torrent_info() {
        let info = get_info(&sample_torrent_path()).expect("sample torrent info should load");

        assert_eq!(info.info_hash, "d69f91e6b2ae4c542468d1073a71d4ea13879a7f");
        assert_eq!(info.length, 92063);
        assert_eq!(info.piece_length, 32768);
        assert_eq!(info.pieces.len(), 3);
        assert_eq!(
            info.trackers,
            vec!["http://bittorrent-test-tracker.codecrafters.io/announce"]
        );
        assert!(matches!(
            info.layout,
            Some(TorrentLayout::SingleFile { .. })
        ));
    }

    #[test]
    fn rejects_invalid_piece_data() {
        let info = get_info(&sample_torrent_path()).expect("sample torrent info should load");

        assert!(!verify_piece(&info, 0, b"not the correct piece"));
    }

    #[test]
    fn calculates_lengths_for_multi_file_metadata() {
        let info = super::Info {
            name: "bundle".into(),
            piece_length: 16384,
            pieces: super::Hashes(vec![[0u8; 20]]),
            keys: Keys::MultiFile {
                files: vec![
                    super::File {
                        length: 10,
                        path: vec!["a".into()],
                    },
                    super::File {
                        length: 15,
                        path: vec!["b".into()],
                    },
                ],
            },
        };

        assert_eq!(TorrentInfo::calculate_length(&info), 25);
        let layout = TorrentInfo::build_layout(&info).expect("multi-file layout should build");
        match layout {
            TorrentLayout::MultiFile { root_name, files } => {
                assert_eq!(root_name, "bundle");
                assert_eq!(files.len(), 2);
                assert_eq!(files[0].relative_path, PathBuf::from("a"));
                assert_eq!(files[0].offset, 0);
                assert_eq!(files[1].relative_path, PathBuf::from("b"));
                assert_eq!(files[1].offset, 10);
            }
            _ => panic!("expected multi-file layout"),
        }
    }

    #[test]
    fn rejects_unsafe_multi_file_paths() {
        assert!(validate_relative_path(&["..".into(), "evil".into()]).is_err());
        assert!(validate_relative_path(&["nested/file".into()]).is_err());
    }

    #[test]
    fn preserves_multiple_trackers_from_torrent_metadata() {
        let torrent = Torrent {
            announce: "http://tracker.one/announce".into(),
            announce_list: vec![
                vec!["http://tracker.two/announce".into()],
                vec!["udp://tracker.three:6969/announce".into()],
                vec!["http://tracker.one/announce".into()],
            ],
            info: super::Info {
                name: "file.bin".into(),
                piece_length: 4,
                pieces: super::Hashes(vec![[0u8; 20]]),
                keys: Keys::SingleFile { length: 4 },
            },
        };

        assert_eq!(
            torrent.trackers(),
            vec![
                "http://tracker.one/announce",
                "http://tracker.two/announce",
                "udp://tracker.three:6969/announce",
            ]
        );
    }
}
