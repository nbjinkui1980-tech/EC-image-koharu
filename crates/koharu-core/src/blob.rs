//! Content-addressed blob references.
//!
//! `BlobRef` is just the hash. The actual `BlobStore` (filesystem + LRU
//! decode cache) lives in `koharu-app`.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;

/// Hex-encoded blake3 hash of an immutable blob.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct BlobRef(String);

impl utoipa::PartialSchema for BlobRef {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some("Hex-encoded blake3 hash of an immutable blob."))
            .min_length(Some(64))
            .max_length(Some(64))
            .pattern(Some("^[0-9a-f]{64}$"))
            .into()
    }
}

impl ToSchema for BlobRef {}

impl BlobRef {
    pub fn parse(hash: impl Into<String>) -> anyhow::Result<Self> {
        let hash = hash.into();
        anyhow::ensure!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "invalid blob ref: expected 64 lowercase hexadecimal characters"
        );
        Ok(Self(hash))
    }
    pub fn hash(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BlobRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for BlobRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn blob_ref_parse_accepts_only_lowercase_blake3_hex() {
        assert_eq!(BlobRef::parse(VALID).unwrap().hash(), VALID);

        for invalid in [
            String::new(),
            VALID[..63].to_owned(),
            format!("{VALID}0"),
            format!("A{}", &VALID[1..]),
            format!("{}F{}", &VALID[..15], &VALID[16..]),
            format!("{}é", "e".repeat(62)),
            format!("/{}", &VALID[1..]),
            format!("\\{}", &VALID[1..]),
            format!("../{}", &VALID[3..]),
            format!("./{}", &VALID[2..]),
            format!("C:\\{}", &VALID[3..]),
        ] {
            assert!(BlobRef::parse(&invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn blob_ref_deserialization_reuses_parser() {
        let valid: BlobRef = serde_json::from_str(&format!("\"{VALID}\"")).unwrap();
        assert_eq!(valid.hash(), VALID);

        for invalid in ["", "ABCDEF", "../outside", "é"] {
            assert!(
                serde_json::from_str::<BlobRef>(&format!("\"{invalid}\"")).is_err(),
                "deserialized {invalid:?}"
            );
        }
    }
}
