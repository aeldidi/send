//! A module to use with `serde(with = "duration_seconds")` if you want to
//! serialize/deserialize a [chrono::Duration] with the representation of an
//! integer number of seconds.
use chrono::Duration;
use serde::{
    de::{Error, Unexpected},
    Deserialize, Deserializer, Serializer,
};

pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let seconds: u64 = Deserialize::deserialize(deserializer)?;
    if seconds > i64::MAX as u64 / 1000 {
        return Err(Error::invalid_value(
            Unexpected::Unsigned(seconds),
            &"a value between 0 and 9223372036854775",
        ));
    }

    match Duration::try_seconds(
        seconds.try_into().expect("seconds was > i64::MAX / 1000"),
    ) {
        Some(x) => Ok(x),
        None => Err(Error::invalid_value(
            Unexpected::Unsigned(seconds),
            &"a value between 0 and 9223372036854775",
        )),
    }
}

pub fn serialize<S>(x: &Duration, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Ok(s.serialize_i64(x.num_seconds())?)
}
