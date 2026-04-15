use rand::Rng;

const PEER_ID_PREFIX: &[u8; 8] = b"-TR3000-";
const PEER_ID_CHARSET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub fn generate_peer_id() -> [u8; 20] {
    let mut peer_id = [0u8; 20];
    peer_id[..PEER_ID_PREFIX.len()].copy_from_slice(PEER_ID_PREFIX);

    let mut rng = rand::thread_rng();
    for byte in &mut peer_id[PEER_ID_PREFIX.len()..] {
        let idx = rng.gen_range(0..PEER_ID_CHARSET.len());
        *byte = PEER_ID_CHARSET[idx];
    }

    peer_id
}

#[cfg(test)]
mod tests {
    use super::{generate_peer_id, PEER_ID_CHARSET, PEER_ID_PREFIX};

    #[test]
    fn generated_peer_ids_have_expected_prefix_and_charset() {
        let peer_id = generate_peer_id();

        assert_eq!(&peer_id[..PEER_ID_PREFIX.len()], PEER_ID_PREFIX);
        assert_eq!(peer_id.len(), 20);

        for byte in &peer_id[PEER_ID_PREFIX.len()..] {
            assert!(PEER_ID_CHARSET.contains(byte));
        }
    }
}
