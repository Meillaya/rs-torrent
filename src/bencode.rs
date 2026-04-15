// bencode.rs

use crate::error::{Result, TorrentError};
use serde_bencode::value::Value;
use std::collections::HashMap;

pub fn decode(data: &[u8]) -> Result<Value> {
    serde_bencode::de::from_bytes(data).map_err(TorrentError::from)
}

pub fn encode(value: &Value) -> Result<Vec<u8>> {
    serde_bencode::ser::to_bytes(value).map_err(TorrentError::from)
}

// Decoding functions for specific types
pub fn decode_bytes(data: &[u8]) -> Result<Vec<u8>> {
    match decode(data)? {
        Value::Bytes(b) => Ok(b),
        _ => Err(TorrentError::UnexpectedType {
            expected: "bytes",
            found: "non-bytes",
        }),
    }
}

pub fn decode_string(data: &[u8]) -> Result<String> {
    let bytes = decode_bytes(data)?;
    String::from_utf8(bytes).map_err(|e| TorrentError::InvalidResponseFormat(e.to_string()))
}

pub fn decode_integer(data: &[u8]) -> Result<i64> {
    match decode(data)? {
        Value::Int(i) => Ok(i),
        _ => Err(TorrentError::UnexpectedType {
            expected: "integer",
            found: "non-integer",
        }),
    }
}

pub fn decode_list(data: &[u8]) -> Result<Vec<Value>> {
    match decode(data)? {
        Value::List(l) => Ok(l),
        _ => Err(TorrentError::UnexpectedType {
            expected: "list",
            found: "non-list",
        }),
    }
}

pub fn decode_dict(data: &[u8]) -> Result<HashMap<Vec<u8>, Value>> {
    match decode(data)? {
        Value::Dict(d) => Ok(d),
        _ => Err(TorrentError::UnexpectedType {
            expected: "dictionary",
            found: "non-dictionary",
        }),
    }
}

// Encoding functions for specific types
pub fn encode_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    encode(&Value::Bytes(bytes.to_vec()))
}

pub fn encode_string(s: &str) -> Result<Vec<u8>> {
    encode_bytes(s.as_bytes())
}

pub fn encode_integer(i: i64) -> Result<Vec<u8>> {
    encode(&Value::Int(i))
}

pub fn encode_list(list: &[Value]) -> Result<Vec<u8>> {
    encode(&Value::List(list.to_vec()))
}

pub fn encode_dict(dict: &HashMap<Vec<u8>, Value>) -> Result<Vec<u8>> {
    encode(&Value::Dict(dict.clone()))
}

#[cfg(test)]
mod tests {
    use super::{decode_dict, decode_integer, decode_string, encode_integer, encode_string};
    use crate::error::TorrentError;

    #[test]
    fn round_trips_strings_and_integers() {
        let encoded_string = encode_string("hello").expect("string should encode");
        let encoded_int = encode_integer(42).expect("integer should encode");

        assert_eq!(
            decode_string(&encoded_string).expect("string should decode"),
            "hello"
        );
        assert_eq!(decode_integer(&encoded_int).expect("int should decode"), 42);
    }

    #[test]
    fn reports_type_mismatches() {
        let err = decode_integer(b"5:hello").expect_err("string is not an integer");

        assert!(matches!(
            err,
            TorrentError::UnexpectedType {
                expected: "integer",
                found: "non-integer"
            }
        ));
    }

    #[test]
    fn decodes_dictionaries() {
        let dict = decode_dict(b"d3:cow3:moo4:spam4:eggse").expect("dict should decode");

        assert_eq!(dict.len(), 2);
        assert_eq!(
            dict.get(b"cow".as_slice()).expect("cow key should exist"),
            &serde_bencode::value::Value::Bytes(b"moo".to_vec())
        );
    }
}

// Helper functions for working with dictionaries
// pub fn get_bytes_from_dict(dict: &HashMap<Vec<u8>, Value>, key: &[u8]) -> Result<Vec<u8>> {
//     match dict.get(key) {
//         Some(Value::Bytes(b)) => Ok(b.clone()),
//         Some(_) => Err(TorrentError::UnexpectedType {
//             expected: "bytes",
//             found: "non-bytes",
//         }),
//         None => Err(TorrentError::MissingKey(&String::from_utf8_lossy(key).into_owned())),
//     }
// }

// pub fn get_string_from_dict(dict: &HashMap<Vec<u8>, Value>, key: &[u8]) -> Result<String> {
//     let bytes = get_bytes_from_dict(dict, key)?;
//     String::from_utf8(bytes).map_err(|e| TorrentError::InvalidResponseFormat(e.to_string()))
// }

// pub fn get_integer_from_dict(dict: &HashMap<Vec<u8>, Value>, key: &[u8]) -> Result<i64> {
//     match dict.get(key) {
//         Some(Value::Int(i)) => Ok(*i),
//         Some(_) => Err(TorrentError::UnexpectedType {
//             expected: "integer",
//             found: "non-integer",
//         }),
//         None => Err(TorrentError::MissingKey(String::from_utf8_lossy(key).into_owned())),
//     }
// }

// pub fn get_list_from_dict(dict: &HashMap<Vec<u8>, Value>, key: &[u8]) -> Result<Vec<Value>> {
//     match dict.get(key) {
//         Some(Value::List(l)) => Ok(l.clone()),
//         Some(_) => Err(TorrentError::UnexpectedType {
//             expected: "list",
//             found: "non-list",
//         }),
//         None => Err(TorrentError::MissingKey(String::from_utf8_lossy(key).into_owned())),
//     }
// }

// pub fn get_dict_from_dict(dict: &HashMap<Vec<u8>, Value>, key: &[u8]) -> Result<HashMap<Vec<u8>, Value>> {
//     match dict.get(key) {
//         Some(Value::Dict(d)) => Ok(d.clone()),
//         Some(_) => Err(TorrentError::UnexpectedType {
//             expected: "dictionary",
//             found: "non-dictionary",
//         }),
//         None => Err(TorrentError::MissingKey(String::from_utf8_lossy(key).into_owned())),
//     }
// }
