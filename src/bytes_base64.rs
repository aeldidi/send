//! A module to use with `serde(with = "bytes_base64")` if you want to
//! serialize/deserialize a `Vec<u8>` with the representation of a
//! base64-encoded string.
use base64::prelude::*;
use serde::{de::Error, Deserialize, Deserializer, Serializer};

pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    let mut data = Vec::new();
    match BASE64_STANDARD.decode_vec(s, &mut data) {
        Ok(()) => Ok(data),
        Err(err) => Err(Error::custom(format!("invalid base64: {}", err))),
    }
}

pub fn serialize<S>(x: &Vec<u8>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Ok(s.serialize_str(&BASE64_STANDARD.encode(x))?)
}
