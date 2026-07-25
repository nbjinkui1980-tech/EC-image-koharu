//! Compile-first, test-only schema scaffold for the Revision 46 D0 contracts.

use serde::Deserialize;

const LEDGER_SCHEMA_VERSION: u8 = 1;
const BYTE_CEILING: u64 = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceLedger {
    version: u8,
    visual_input: String,
    visual_input_sha256: String,
    visual_manifest: String,
    visual_manifest_sha256: String,
    source_gate_fixture_manifest_sha256: String,
    evidence_root: String,
}

impl EvidenceLedger {
    fn validate(&self) -> Result<(), &'static str> {
        if self.version != LEDGER_SCHEMA_VERSION {
            return Err("ledger schema version drift");
        }
        if [
            self.visual_input.as_str(),
            self.visual_manifest.as_str(),
            self.evidence_root.as_str(),
        ]
        .contains(&"")
        {
            return Err("empty path");
        }
        if [
            self.visual_input_sha256.as_str(),
            self.visual_manifest_sha256.as_str(),
            self.source_gate_fixture_manifest_sha256.as_str(),
        ]
        .into_iter()
        .any(|hash| !is_sha256(hash))
        {
            return Err("invalid sha256");
        }
        Ok(())
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ContractId {
    ImageInputContractV1,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum EncodedFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum FormatDetection {
    ByteSniffed,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum DecodeStep {
    DecoderReportedOrientation,
    CanonicalizeRgba8,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum AlphaPolicy {
    WhiteOnlyAtModelRgbConversion,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum DecodedReservation {
    SharedInFlightPlusCache,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ImportDecode {
    SerialPerImportRequest,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum MaskWorkspaceSizing {
    CheckedPagePixelsTimesPeakLiveOneByteMasks,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MaskWorkspacePreflight {
    bytes_per_pixel: u8,
    sizing: MaskWorkspaceSizing,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageInputContract {
    contract: ContractId,
    formats: [EncodedFormat; 3],
    format_detection: FormatDetection,
    decode_order: [DecodeStep; 2],
    alpha_policy: AlphaPolicy,
    decoded_byte_ceiling: u64,
    decoded_reservation: DecodedReservation,
    aggregate_encoded_byte_ceiling: u64,
    import_decode: ImportDecode,
    mask_workspace_preflight: MaskWorkspacePreflight,
}

impl ImageInputContract {
    fn validate(&self) -> Result<(), &'static str> {
        if self.contract != ContractId::ImageInputContractV1
            || self.formats != [EncodedFormat::Png, EncodedFormat::Jpeg, EncodedFormat::Webp]
            || self.format_detection != FormatDetection::ByteSniffed
            || self.decode_order
                != [
                    DecodeStep::DecoderReportedOrientation,
                    DecodeStep::CanonicalizeRgba8,
                ]
            || self.alpha_policy != AlphaPolicy::WhiteOnlyAtModelRgbConversion
            || self.decoded_byte_ceiling != BYTE_CEILING
            || self.decoded_reservation != DecodedReservation::SharedInFlightPlusCache
            || self.aggregate_encoded_byte_ceiling != BYTE_CEILING
            || self.import_decode != ImportDecode::SerialPerImportRequest
            || self.mask_workspace_preflight.bytes_per_pixel != 1
            || self.mask_workspace_preflight.sizing
                != MaskWorkspaceSizing::CheckedPagePixelsTimesPeakLiveOneByteMasks
        {
            return Err("image input contract drift");
        }
        Ok(())
    }

    fn checked_rgba8_bytes(&self, width: u64, height: u64) -> Option<u64> {
        width
            .checked_mul(height)?
            .checked_mul(4)
            .filter(|bytes| *bytes <= self.decoded_byte_ceiling)
    }
}

const LEDGER_JSON: &str = r#"{
    "version": 1,
    "visual_input": "/external/input.webp",
    "visual_input_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "visual_manifest": "/external/manifest.json",
    "visual_manifest_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "source_gate_fixture_manifest_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "evidence_root": "/external/evidence/run"
}"#;

const IMAGE_INPUT_CONTRACT_JSON: &str = r#"{
    "contract": "image-input-contract-v1",
    "formats": ["png", "jpeg", "webp"],
    "format_detection": "byte-sniffed",
    "decode_order": ["decoder-reported-orientation", "canonicalize-rgba8"],
    "alpha_policy": "white-only-at-model-rgb-conversion",
    "decoded_byte_ceiling": 536870912,
    "decoded_reservation": "shared-in-flight-plus-cache",
    "aggregate_encoded_byte_ceiling": 536870912,
    "import_decode": "serial-per-import-request",
    "mask_workspace_preflight": {
        "bytes_per_pixel": 1,
        "sizing": "checked-page-pixels-times-peak-live-one-byte-masks"
    }
}"#;

#[test]
fn d0_revision_46_ledger_schema_is_closed() {
    let ledger: EvidenceLedger = serde_json::from_str(LEDGER_JSON).unwrap();
    assert_eq!(ledger.validate(), Ok(()));

    let mut value: serde_json::Value = serde_json::from_str(LEDGER_JSON).unwrap();
    value["unexpected"] = true.into();
    assert!(serde_json::from_value::<EvidenceLedger>(value).is_err());

    let mut value: serde_json::Value = serde_json::from_str(LEDGER_JSON).unwrap();
    value.as_object_mut().unwrap().remove("evidence_root");
    assert!(serde_json::from_value::<EvidenceLedger>(value).is_err());

    let mut value: serde_json::Value = serde_json::from_str(LEDGER_JSON).unwrap();
    value["version"] = 2.into();
    assert_eq!(
        serde_json::from_value::<EvidenceLedger>(value)
            .unwrap()
            .validate(),
        Err("ledger schema version drift")
    );

    let mut value: serde_json::Value = serde_json::from_str(LEDGER_JSON).unwrap();
    value["version"] = "1".into();
    assert!(serde_json::from_value::<EvidenceLedger>(value).is_err());

    for (field, replacement, expected) in [
        ("visual_input", serde_json::json!(""), Err("empty path")),
        (
            "visual_input_sha256",
            serde_json::json!("not-a-sha256"),
            Err("invalid sha256"),
        ),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(LEDGER_JSON).unwrap();
        value[field] = replacement;
        assert_eq!(
            serde_json::from_value::<EvidenceLedger>(value)
                .unwrap()
                .validate(),
            expected,
            "{field}"
        );
    }
}

#[test]
fn d0_image_input_contract_v1_schema_is_closed() {
    let contract: ImageInputContract = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    assert_eq!(contract.validate(), Ok(()));

    let mut value: serde_json::Value = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    value["unexpected"] = true.into();
    assert!(serde_json::from_value::<ImageInputContract>(value).is_err());

    let mut value: serde_json::Value = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    value.as_object_mut().unwrap().remove("import_decode");
    assert!(serde_json::from_value::<ImageInputContract>(value).is_err());

    let mut value: serde_json::Value = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    value["contract"] = "image-input-contract-v2".into();
    assert!(serde_json::from_value::<ImageInputContract>(value).is_err());

    for (field, replacement) in [
        ("decoded_byte_ceiling", serde_json::json!(BYTE_CEILING - 1)),
        (
            "aggregate_encoded_byte_ceiling",
            serde_json::json!(BYTE_CEILING - 1),
        ),
        ("formats", serde_json::json!(["jpeg", "png", "webp"])),
        (
            "decode_order",
            serde_json::json!(["canonicalize-rgba8", "decoder-reported-orientation"]),
        ),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
        value[field] = replacement;
        assert_eq!(
            serde_json::from_value::<ImageInputContract>(value)
                .unwrap()
                .validate(),
            Err("image input contract drift"),
            "{field}"
        );
    }

    let mut value: serde_json::Value = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    value["mask_workspace_preflight"]["bytes_per_pixel"] = 2.into();
    assert_eq!(
        serde_json::from_value::<ImageInputContract>(value)
            .unwrap()
            .validate(),
        Err("image input contract drift")
    );

    let mut value: serde_json::Value = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    value["mask_workspace_preflight"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<ImageInputContract>(value).is_err());
}

#[test]
fn d0_image_input_contract_v1_checked_rgba8_ceiling_is_frozen() {
    let contract: ImageInputContract = serde_json::from_str(IMAGE_INPUT_CONTRACT_JSON).unwrap();
    assert_eq!(
        contract.checked_rgba8_bytes(BYTE_CEILING / 4, 1),
        Some(BYTE_CEILING)
    );
    assert_eq!(contract.checked_rgba8_bytes(BYTE_CEILING / 4 + 1, 1), None);
    assert_eq!(contract.checked_rgba8_bytes(u64::MAX, 2), None);
}
