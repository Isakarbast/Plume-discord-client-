use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Discord ID. The top 42 bits are a timestamp, so IDs sort by creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Snowflake(pub u64);

impl Snowflake {
    /// Discord's epoch (2015-01-01) in milliseconds.
    const DISCORD_EPOCH: u64 = 1_420_070_400_000;

    /// When this object was created, in Unix milliseconds.
    pub fn timestamp_ms(self) -> u64 {
        (self.0 >> 22) + Self::DISCORD_EPOCH
    }

    /// The smallest ID that could have been created at this moment. Handy as
    /// a message `nonce` or as a "before/after this time" cursor.
    pub fn from_timestamp_ms(unix_ms: u64) -> Self {
        Snowflake(unix_ms.saturating_sub(Self::DISCORD_EPOCH) << 22)
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Snowflake {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Snowflake)
    }
}

// Discord's JSON sends IDs as strings ("1234…") because JavaScript can't hold
// a full 64-bit integer. We accept strings *and* numbers, and always write strings.
impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SnowflakeVisitor;

        impl Visitor<'_> for SnowflakeVisitor {
            type Value = Snowflake;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a Discord ID as a string or integer")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Snowflake, E> {
                v.parse().map_err(E::custom)
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Snowflake, E> {
                Ok(Snowflake(v))
            }
        }

        deserializer.deserialize_any(SnowflakeVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_from_known_id() {
        // Example ID from Discord's API docs
        let id = Snowflake(175928847299117063);
        assert_eq!(id.timestamp_ms(), 1462015105796);
    }

    #[test]
    fn round_trips_a_timestamp() {
        let id = Snowflake::from_timestamp_ms(1462015105796);
        assert_eq!(id.timestamp_ms(), 1462015105796);
    }

    #[test]
    fn deserializes_from_string_and_number() {
        let from_str: Snowflake = serde_json::from_str(r#""175928847299117063""#).unwrap();
        let from_num: Snowflake = serde_json::from_str("175928847299117063").unwrap();
        assert_eq!(from_str, Snowflake(175928847299117063));
        assert_eq!(from_num, from_str);
    }

    #[test]
    fn serializes_as_string() {
        let json = serde_json::to_string(&Snowflake(42)).unwrap();
        assert_eq!(json, r#""42""#);
    }
}
