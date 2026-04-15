use reqwest;
use serde_bencode;
use serde_json;
use std::fmt;
use std::io;
use tokio::sync::broadcast::error::SendError;
use url;
pub type Result<T> = std::result::Result<T, TorrentError>;

#[derive(Debug)]
pub enum TorrentError {
    Io(io::Error),
    Bencode(serde_bencode::Error),
    MissingKey(&'static str),
    UnexpectedType {
        expected: &'static str,
        found: &'static str,
    },
    Reqwest(reqwest::Error),
    InvalidResponseFormat(String),
    UrlParse(url::ParseError),
    Json(serde_json::Error),
    Tracker(String),
    InvalidInfoHash,
    InvalidPeerResponse,
    PieceVerificationFailed,
    ConnectionFailed(String),
    DecodeError(String),
    NoPeersAvailable,
    UnexpectedMessage(String),
    UnexpectedBlockData,
    DownloadFailed(String),
    InvalidMagnetLink,
    ChannelSendError(String),
    MetadataExtensionNotSupported,
    InvalidMetadataResponse,
    MetadataSizeNotFound,
    MetadataRejected,
    IncompleteMetadata,
    ConnectionClosed,
    ConnectionTimeout,
}

impl fmt::Display for TorrentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TorrentError::Io(e) => write!(f, "IO Error: {}", e),
            TorrentError::Bencode(e) => write!(f, "Bencode Error: {}", e),
            TorrentError::MissingKey(key) => write!(f, "Missing Key: {}", key),
            TorrentError::UnexpectedType { expected, found } => {
                write!(f, "Unexpected Type: expected {}, found {}", expected, found)
            }
            TorrentError::Reqwest(e) => write!(f, "HTTP Request Error: {}", e),
            TorrentError::InvalidResponseFormat(msg) => {
                write!(f, "Invalid Response Format: {}", msg)
            }
            TorrentError::UrlParse(e) => write!(f, "URL Parse Error: {}", e),
            TorrentError::Json(e) => write!(f, "JSON Error: {}", e),
            TorrentError::Tracker(e) => write!(f, "Tracker Error: {}", e),
            TorrentError::InvalidInfoHash => write!(f, "Invalid Info Hash"),
            TorrentError::InvalidPeerResponse => write!(f, "Invalid Peer Response"),
            TorrentError::PieceVerificationFailed => write!(f, "Piece Verification Failed"),
            TorrentError::ConnectionFailed(msg) => write!(f, "Connection Failed: {}", msg),
            TorrentError::DecodeError(msg) => write!(f, "Decode Error: {}", msg),
            TorrentError::NoPeersAvailable => write!(f, "No Peers Available"),
            TorrentError::UnexpectedMessage(msg) => write!(f, "Unexpected Message: {}", msg),
            TorrentError::UnexpectedBlockData => write!(f, "Unexpected Block Data"),
            TorrentError::DownloadFailed(msg) => write!(f, "Download Failed: {}", msg),
            TorrentError::InvalidMagnetLink => write!(f, "Invalid Magnet Link"),
            TorrentError::ChannelSendError(msg) => write!(f, "Channel Send Error: {}", msg),
            TorrentError::MetadataExtensionNotSupported => {
                write!(f, "Metadata Extension Not Supported")
            }
            TorrentError::InvalidMetadataResponse => write!(f, "Invalid Metadata Response"),
            TorrentError::MetadataSizeNotFound => write!(f, "Metadata Size Not Found"),
            TorrentError::MetadataRejected => write!(f, "Metadata Rejected"),
            TorrentError::IncompleteMetadata => write!(f, "Incomplete Metadata"),
            TorrentError::ConnectionClosed => write!(f, "Connection Closed"),
            TorrentError::ConnectionTimeout => write!(f, "Connection Timeout"),
        }
    }
}

impl std::error::Error for TorrentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TorrentError::Io(e) => Some(e),
            TorrentError::Bencode(e) => Some(e),
            TorrentError::Reqwest(e) => Some(e),
            TorrentError::UrlParse(e) => Some(e),
            TorrentError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SendError<usize>> for TorrentError {
    fn from(err: SendError<usize>) -> Self {
        TorrentError::ChannelSendError(format!("Failed to send piece index: {}", err))
    }
}

impl From<io::Error> for TorrentError {
    fn from(err: io::Error) -> Self {
        TorrentError::Io(err)
    }
}

impl From<serde_bencode::Error> for TorrentError {
    fn from(err: serde_bencode::Error) -> Self {
        TorrentError::Bencode(err)
    }
}

impl From<reqwest::Error> for TorrentError {
    fn from(err: reqwest::Error) -> Self {
        TorrentError::Reqwest(err)
    }
}

impl From<url::ParseError> for TorrentError {
    fn from(err: url::ParseError) -> Self {
        TorrentError::UrlParse(err)
    }
}

impl From<serde_json::Error> for TorrentError {
    fn from(err: serde_json::Error) -> Self {
        TorrentError::Json(err)
    }
}

impl From<std::array::TryFromSliceError> for TorrentError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        TorrentError::DecodeError(err.to_string())
    }
}
