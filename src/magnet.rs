// magnet.rs

use crate::error::{Result, TorrentError};
use url::Url;

#[derive(Debug)]
pub struct Magnet {
    pub info_hash: String,
    pub tracker_url: Option<String>,
    pub display_name: Option<String>,
}

impl Magnet {
    pub fn parse(link: &str) -> Result<Self> {
        let url = Url::parse(link).map_err(|_| TorrentError::InvalidMagnetLink)?;

        if url.scheme() != "magnet" {
            return Err(TorrentError::InvalidMagnetLink);
        }

        let mut info_hash = None;
        let mut tracker_url = None;
        let mut display_name = None;

        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "xt" => {
                    if let Some(stripped) = value.strip_prefix("urn:btih:") {
                        info_hash = Some(stripped.to_string());
                    }
                }
                "tr" => tracker_url = Some(value.to_string()),
                "dn" => display_name = Some(value.to_string()),
                _ => {}
            }
        }

        let info_hash = info_hash.ok_or(TorrentError::InvalidMagnetLink)?;

        Ok(Magnet {
            info_hash,
            tracker_url,
            display_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Magnet;
    use crate::error::TorrentError;

    #[test]
    fn parses_common_magnet_fields() {
        let magnet = Magnet::parse(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=example&tr=http://tracker.test/announce",
        )
        .expect("magnet should parse");

        assert_eq!(magnet.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(magnet.display_name.as_deref(), Some("example"));
        assert_eq!(
            magnet.tracker_url.as_deref(),
            Some("http://tracker.test/announce")
        );
    }

    #[test]
    fn rejects_non_magnet_urls() {
        let err = Magnet::parse("https://example.com").expect_err("non-magnet should fail");
        assert!(matches!(err, TorrentError::InvalidMagnetLink));
    }
}
