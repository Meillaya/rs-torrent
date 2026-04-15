use crate::error::{Result, TorrentError};
use rand::Rng;
use serde_bencode::value::Value as BencodeValue;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::sleep;
use tokio::{net::TcpStream, time::timeout};

const METADATA_BLOCK_SIZE: usize = 16 * 1024;
const PROTOCOL: &str = "BitTorrent protocol";
const MAX_HANDSHAKE_ATTEMPTS: usize = 5;

pub struct Peer {
    stream: TcpStream,
    pending_messages: VecDeque<PeerMessage>,
    piece_availability: Vec<bool>,
    supports_extensions: bool,
    extension_handshake: Option<ExtensionHandshake>,
}

pub struct PeerMessage {
    pub payload: Vec<u8>,
    pub id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExtensionHandshake {
    metadata_extension_id: Option<u8>,
    metadata_size: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
struct MetadataMessage {
    msg_type: i64,
    piece: usize,
    total_size: Option<usize>,
    data: Vec<u8>,
}

impl Peer {
    pub async fn new(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| TorrentError::ConnectionFailed(e.to_string()))?;
        Ok(Self {
            stream,
            pending_messages: VecDeque::new(),
            piece_availability: Vec::new(),
            supports_extensions: true,
            extension_handshake: None,
        })
    }

    pub async fn enable_tcp_nodelay(&mut self) -> Result<()> {
        self.stream.set_nodelay(true).map_err(|e| {
            TorrentError::ConnectionFailed(format!("Failed to set TCP_NODELAY: {}", e))
        })?;
        Ok(())
    }

    pub async fn handshake(
        &mut self,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
    ) -> Result<[u8; 20]> {
        let mut handshake = vec![19];
        handshake.extend_from_slice(PROTOCOL.as_bytes());

        let mut reserved_bytes = [0u8; 8];
        reserved_bytes[5] = 0x10;
        handshake.extend_from_slice(&reserved_bytes);
        handshake.extend_from_slice(info_hash);
        handshake.extend_from_slice(peer_id);

        let mut attempts = 0;
        while attempts < MAX_HANDSHAKE_ATTEMPTS {
            match self.try_handshake(&handshake).await {
                Ok(peer_id_received) => {
                    if self.supports_extensions {
                        self.send_extension_handshake().await?;
                        self.receive_extension_handshake().await?;
                    }
                    return Ok(peer_id_received);
                }
                Err(e) => {
                    eprintln!(
                        "Handshake attempt {} failed: {}. Retrying...",
                        attempts + 1,
                        e
                    );
                    attempts += 1;
                    let jitter = rand::thread_rng().gen_range(0..100);
                    sleep(Duration::from_secs(2) + Duration::from_millis(jitter)).await;
                }
            }
        }

        Err(TorrentError::ConnectionFailed(
            "Max handshake attempts reached".into(),
        ))
    }

    async fn try_handshake(&mut self, handshake: &[u8]) -> Result<[u8; 20]> {
        assert_eq!(handshake.len(), 68, "Handshake must be 68 bytes long");

        eprintln!("Sending handshake...");
        self.stream.write_all(handshake).await.map_err(|e| {
            TorrentError::ConnectionFailed(format!("Failed to send handshake: {}", e))
        })?;
        eprintln!("Handshake sent. Awaiting response...");

        let mut response = [0u8; 68];
        self.stream.read_exact(&mut response).await.map_err(|e| {
            TorrentError::ConnectionFailed(format!("Failed to receive handshake: {}", e))
        })?;
        eprintln!("Handshake response received.");

        if response[0] != 19 || &response[1..20] != PROTOCOL.as_bytes() {
            return Err(TorrentError::InvalidPeerResponse);
        }

        if response[28..48] != handshake[28..48] {
            return Err(TorrentError::InvalidPeerResponse);
        }

        self.supports_extensions = (response[25] & 0x10) != 0;

        let mut received_peer_id = [0u8; 20];
        received_peer_id.copy_from_slice(&response[48..68]);
        Ok(received_peer_id)
    }

    pub async fn receive_bitfield(&mut self, piece_count: usize) -> Result<Vec<bool>> {
        let message = self.receive_message().await?;
        if message.id != 5 {
            return Err(TorrentError::UnexpectedMessage(
                "Expected bitfield message".to_string(),
            ));
        }
        let availability = parse_bitfield(&message.payload, piece_count);
        self.piece_availability = availability.clone();
        Ok(availability)
    }

    pub fn has_piece(&self, piece_index: usize) -> Option<bool> {
        self.piece_availability.get(piece_index).copied()
    }

    pub async fn send_interested(&mut self) -> Result<()> {
        self.send_message(2, &[]).await?;
        println!("Sent 'interested' message.");
        Ok(())
    }

    pub async fn receive_unchoke(&mut self, piece_count: Option<usize>) -> Result<()> {
        loop {
            let message = self.receive_message().await?;
            if self.handle_auxiliary_message(&message, piece_count)? {
                continue;
            }

            if message.id != 1 {
                return Err(TorrentError::UnexpectedMessage(
                    "Expected unchoke message".to_string(),
                ));
            }
            println!("Received 'unchoke' message.");
            return Ok(());
        }
    }

    pub async fn request_block(&mut self, index: usize, begin: usize, length: usize) -> Result<()> {
        let payload = [
            (index as u32).to_be_bytes(),
            (begin as u32).to_be_bytes(),
            (length as u32).to_be_bytes(),
        ]
        .concat();

        self.send_message(6, &payload).await?;
        println!(
            "Requested block: piece {}, begin {}, length {}",
            index, begin, length
        );
        Ok(())
    }

    pub async fn receive_block(
        &mut self,
        expected_index: usize,
        expected_begin: usize,
        piece_count: Option<usize>,
    ) -> Result<Vec<u8>> {
        loop {
            let message = self.receive_message().await?;
            if self.handle_auxiliary_message(&message, piece_count)? {
                continue;
            }

            if message.id != 7 {
                return Err(TorrentError::UnexpectedMessage(
                    "Expected block message".to_string(),
                ));
            }

            if message.payload.len() < 8 {
                return Err(TorrentError::UnexpectedBlockData);
            }

            let index = u32::from_be_bytes(message.payload[0..4].try_into()?);
            let begin = u32::from_be_bytes(message.payload[4..8].try_into()?);
            if index as usize != expected_index || begin as usize != expected_begin {
                return Err(TorrentError::UnexpectedBlockData);
            }
            println!("Received block: piece {}, begin {}", index, begin);
            return Ok(message.payload[8..].to_vec());
        }
    }

    async fn send_message(&mut self, id: u8, payload: &[u8]) -> Result<()> {
        let length = (payload.len() + 1) as u32;
        let mut message = length.to_be_bytes().to_vec();
        message.push(id);
        message.extend_from_slice(payload);
        self.stream.write_all(&message).await.map_err(|e| {
            TorrentError::ConnectionFailed(format!("Failed to send message: {}", e))
        })?;
        Ok(())
    }

    async fn receive_message(&mut self) -> Result<PeerMessage> {
        if let Some(message) = self.pending_messages.pop_front() {
            return Ok(message);
        }

        self.read_message_from_stream().await
    }

    async fn read_message_from_stream(&mut self) -> Result<PeerMessage> {
        const TIMEOUT_DURATION: Duration = Duration::from_secs(30);

        let mut length_bytes = [0u8; 4];
        timeout(TIMEOUT_DURATION, self.stream.read_exact(&mut length_bytes))
            .await
            .map_err(|_| TorrentError::ConnectionTimeout)??;

        let length = u32::from_be_bytes(length_bytes) as usize;
        if length == 0 {
            return Ok(PeerMessage {
                id: 0,
                payload: Vec::new(),
            });
        }

        let mut buffer = vec![0u8; length];
        let mut total_read = 0;

        while total_read < length {
            match timeout(
                TIMEOUT_DURATION,
                self.stream.read(&mut buffer[total_read..]),
            )
            .await
            {
                Ok(Ok(0)) => return Err(TorrentError::ConnectionClosed),
                Ok(Ok(n)) => total_read += n,
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Ok(Err(e)) => {
                    return Err(TorrentError::ConnectionFailed(format!(
                        "Failed to read message: {}",
                        e
                    )))
                }
                Err(_) => return Err(TorrentError::ConnectionTimeout),
            }
        }

        let id = buffer[0];
        let payload = buffer[1..].to_vec();
        Ok(PeerMessage { id, payload })
    }

    fn handle_auxiliary_message(
        &mut self,
        message: &PeerMessage,
        piece_count: Option<usize>,
    ) -> Result<bool> {
        match message.id {
            0 => Ok(true),
            4 => {
                if let Some(piece_count) = piece_count {
                    let piece_index = parse_have_message(&message.payload)?;
                    self.record_have(piece_count, piece_index);
                }
                Ok(true)
            }
            5 => {
                if let Some(piece_count) = piece_count {
                    self.piece_availability = parse_bitfield(&message.payload, piece_count);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn record_have(&mut self, piece_count: usize, piece_index: usize) {
        if self.piece_availability.len() != piece_count {
            self.piece_availability.resize(piece_count, false);
        }

        if piece_index < self.piece_availability.len() {
            self.piece_availability[piece_index] = true;
        }
    }

    pub async fn receive_metadata(&mut self) -> Result<Vec<u8>> {
        let extension_id = self.get_metadata_extension_id()?;
        let metadata_size = self.get_metadata_size()?;
        let piece_count = metadata_size.div_ceil(METADATA_BLOCK_SIZE);
        let mut metadata = Vec::with_capacity(metadata_size);

        for expected_piece in 0..piece_count {
            self.send_metadata_request(expected_piece).await?;

            loop {
                let message = self.receive_metadata_message(extension_id).await?;
                if message.piece != expected_piece {
                    continue;
                }

                match message.msg_type {
                    1 => {
                        if message.total_size != Some(metadata_size) {
                            return Err(TorrentError::InvalidMetadataResponse);
                        }

                        let expected_piece_len =
                            std::cmp::min(METADATA_BLOCK_SIZE, metadata_size - metadata.len());
                        if message.data.len() != expected_piece_len {
                            return Err(TorrentError::InvalidMetadataResponse);
                        }

                        metadata.extend_from_slice(&message.data);
                        break;
                    }
                    2 => return Err(TorrentError::MetadataRejected),
                    _ => continue,
                }
            }
        }

        if metadata.len() != metadata_size {
            return Err(TorrentError::IncompleteMetadata);
        }

        println!("Received metadata: length={}", metadata.len());
        Ok(metadata)
    }

    pub async fn send_metadata_request(&mut self, piece: usize) -> Result<()> {
        let extension_id = self.get_metadata_extension_id()?;
        let dict = BencodeValue::Dict(
            vec![
                (b"msg_type".to_vec(), BencodeValue::Int(0)),
                (b"piece".to_vec(), BencodeValue::Int(piece as i64)),
            ]
            .into_iter()
            .collect(),
        );
        let bencoded_dict = serde_bencode::to_bytes(&dict)?;

        let mut payload = vec![extension_id];
        payload.extend_from_slice(&bencoded_dict);
        self.send_message(20, &payload).await?;

        println!("Sent metadata request for piece {}", piece);
        Ok(())
    }

    fn get_metadata_extension_id(&self) -> Result<u8> {
        self.extension_handshake
            .and_then(|handshake| handshake.metadata_extension_id)
            .ok_or(TorrentError::MetadataExtensionNotSupported)
    }

    fn get_metadata_size(&self) -> Result<usize> {
        self.extension_handshake
            .and_then(|handshake| handshake.metadata_size)
            .ok_or(TorrentError::MetadataSizeNotFound)
    }

    pub async fn receive_extension_handshake(&mut self) -> Result<()> {
        loop {
            let message = self.receive_message().await?;
            match message.id {
                20 => {
                    if message.payload.is_empty() {
                        continue;
                    }
                    let extended_id = message.payload[0];
                    if extended_id == 0 {
                        let handshake_data: BencodeValue =
                            serde_bencode::from_bytes(&message.payload[1..])?;
                        let parsed_handshake = parse_extension_handshake(&handshake_data)?;
                        self.extension_handshake = Some(parsed_handshake);

                        if let Some(id) = parsed_handshake.metadata_extension_id {
                            println!("Peer Metadata Extension ID: {}", id);
                        }
                        return Ok(());
                    }

                    self.pending_messages.push_back(message);
                }
                _ => self.pending_messages.push_back(message),
            }
        }
    }

    pub async fn send_extension_handshake(&mut self) -> Result<()> {
        let extension_handshake = serde_bencode::to_bytes(&BencodeValue::Dict(
            vec![(
                b"m".to_vec(),
                BencodeValue::Dict(
                    vec![(b"ut_metadata".to_vec(), BencodeValue::Int(1))]
                        .into_iter()
                        .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        ))?;

        let payload = [0]
            .into_iter()
            .chain(extension_handshake)
            .collect::<Vec<u8>>();
        self.send_message(20, &payload).await?;
        println!("Sent extension handshake");
        Ok(())
    }

    async fn receive_metadata_message(
        &mut self,
        expected_extension_id: u8,
    ) -> Result<MetadataMessage> {
        loop {
            let message = self.receive_message().await?;
            if message.id != 20 || message.payload.is_empty() {
                continue;
            }

            if message.payload[0] != expected_extension_id {
                continue;
            }

            return parse_metadata_message(&message.payload[1..]);
        }
    }
}

fn parse_extension_handshake(value: &BencodeValue) -> Result<ExtensionHandshake> {
    let BencodeValue::Dict(dict) = value else {
        return Err(TorrentError::InvalidMetadataResponse);
    };

    let metadata_extension_id = dict
        .get(b"m".as_slice())
        .and_then(|m| match m {
            BencodeValue::Dict(m_dict) => m_dict.get(b"ut_metadata".as_slice()),
            _ => None,
        })
        .and_then(|id| match id {
            BencodeValue::Int(id) if *id >= 0 && *id <= u8::MAX as i64 => Some(*id as u8),
            _ => None,
        });

    let metadata_size = dict
        .get(b"metadata_size".as_slice())
        .and_then(|size| match size {
            BencodeValue::Int(size) if *size >= 0 => Some(*size as usize),
            _ => None,
        });

    Ok(ExtensionHandshake {
        metadata_extension_id,
        metadata_size,
    })
}

fn parse_metadata_message(payload: &[u8]) -> Result<MetadataMessage> {
    let dict: BencodeValue = serde_bencode::from_bytes(payload)?;
    let BencodeValue::Dict(dict) = dict else {
        return Err(TorrentError::InvalidMetadataResponse);
    };

    let msg_type = dict
        .get(b"msg_type".as_slice())
        .and_then(|v| match v {
            BencodeValue::Int(value) => Some(*value),
            _ => None,
        })
        .ok_or(TorrentError::InvalidMetadataResponse)?;
    let piece = dict
        .get(b"piece".as_slice())
        .and_then(|v| match v {
            BencodeValue::Int(value) if *value >= 0 => Some(*value as usize),
            _ => None,
        })
        .ok_or(TorrentError::InvalidMetadataResponse)?;
    let total_size = dict.get(b"total_size".as_slice()).and_then(|v| match v {
        BencodeValue::Int(value) if *value >= 0 => Some(*value as usize),
        _ => None,
    });

    let dict_len = serde_bencode::to_bytes(&BencodeValue::Dict(dict.clone()))?.len();
    let data = if payload.len() > dict_len {
        payload[dict_len..].to_vec()
    } else {
        Vec::new()
    };

    Ok(MetadataMessage {
        msg_type,
        piece,
        total_size,
        data,
    })
}

fn parse_bitfield(payload: &[u8], piece_count: usize) -> Vec<bool> {
    let mut availability = Vec::with_capacity(piece_count);

    for byte in payload {
        for bit_index in 0..8 {
            if availability.len() == piece_count {
                return availability;
            }

            let mask = 1 << (7 - bit_index);
            availability.push(byte & mask != 0);
        }
    }

    availability.resize(piece_count, false);
    availability
}

fn parse_have_message(payload: &[u8]) -> Result<usize> {
    if payload.len() != 4 {
        return Err(TorrentError::UnexpectedMessage(
            "Expected have payload length of 4".to_string(),
        ));
    }

    Ok(u32::from_be_bytes(payload.try_into()?) as usize)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_bitfield, parse_extension_handshake, parse_have_message, parse_metadata_message,
        ExtensionHandshake, MetadataMessage,
    };
    use serde_bencode::value::Value;

    #[test]
    fn parses_extension_handshake_metadata_fields() {
        let handshake = Value::Dict(
            vec![
                (
                    b"m".to_vec(),
                    Value::Dict(
                        vec![(b"ut_metadata".to_vec(), Value::Int(3))]
                            .into_iter()
                            .collect(),
                    ),
                ),
                (b"metadata_size".to_vec(), Value::Int(32769)),
            ]
            .into_iter()
            .collect(),
        );

        let parsed = parse_extension_handshake(&handshake).expect("handshake should parse");
        assert_eq!(
            parsed,
            ExtensionHandshake {
                metadata_extension_id: Some(3),
                metadata_size: Some(32769),
            }
        );
    }

    #[test]
    fn parses_metadata_data_messages_with_payload_suffix() {
        let dict = Value::Dict(
            vec![
                (b"msg_type".to_vec(), Value::Int(1)),
                (b"piece".to_vec(), Value::Int(1)),
                (b"total_size".to_vec(), Value::Int(20000)),
            ]
            .into_iter()
            .collect(),
        );
        let mut payload = serde_bencode::to_bytes(&dict).expect("dict should encode");
        payload.extend_from_slice(b"chunk");

        let parsed = parse_metadata_message(&payload).expect("metadata message should parse");
        assert_eq!(
            parsed,
            MetadataMessage {
                msg_type: 1,
                piece: 1,
                total_size: Some(20000),
                data: b"chunk".to_vec(),
            }
        );
    }

    #[test]
    fn parses_metadata_reject_messages_without_suffix() {
        let dict = Value::Dict(
            vec![
                (b"msg_type".to_vec(), Value::Int(2)),
                (b"piece".to_vec(), Value::Int(0)),
            ]
            .into_iter()
            .collect(),
        );
        let payload = serde_bencode::to_bytes(&dict).expect("dict should encode");

        let parsed = parse_metadata_message(&payload).expect("reject message should parse");
        assert_eq!(parsed.msg_type, 2);
        assert_eq!(parsed.piece, 0);
        assert_eq!(parsed.total_size, None);
        assert!(parsed.data.is_empty());
    }

    #[test]
    fn parses_bitfields_into_piece_availability() {
        let availability = parse_bitfield(&[0b1010_0000], 4);
        assert_eq!(availability, vec![true, false, true, false]);
    }

    #[test]
    fn parses_have_messages() {
        let piece_index = parse_have_message(&3u32.to_be_bytes()).expect("have should parse");
        assert_eq!(piece_index, 3);
    }
}
