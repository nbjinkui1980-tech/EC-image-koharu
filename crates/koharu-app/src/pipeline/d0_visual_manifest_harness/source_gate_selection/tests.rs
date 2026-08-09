use super::*;

type RasterBounds = (u32, u32, u32, u32);

fn candidates(
    bbox: (f64, f64, f64, f64),
    page: (u32, u32),
) -> [(&'static str, RasterBounds); 4] {
    const RATIOS: [(&str, u32, u32); 4] = [
        ("S25L4", 1, 25),
        ("S25L5", 1, 20),
        ("S25L6", 3, 50),
        ("S25L7", 7, 100),
    ];
    let short_side = (bbox.2 - bbox.0).min(bbox.3 - bbox.1);
    let long_side = (bbox.2 - bbox.0).max(bbox.3 - bbox.1);
    RATIOS.map(|(name, numerator, denominator)| {
        let padding = (short_side / 4.0)
            .max(long_side * f64::from(numerator) / f64::from(denominator))
            .ceil()
            .max(1.0);
        (
            name,
            (
                (bbox.0 - padding).floor().clamp(0.0, f64::from(page.0)) as u32,
                (bbox.1 - padding).floor().clamp(0.0, f64::from(page.1)) as u32,
                (bbox.2 + padding).ceil().clamp(0.0, f64::from(page.0)) as u32,
                (bbox.3 + padding).ceil().clamp(0.0, f64::from(page.1)) as u32,
            ),
        )
    })
}

#[test]
fn source_gate_selection_candidates_quantize_outward_and_clip() {
    assert_eq!(
        candidates((10.2, 20.2, 110.8, 30.8), (200, 100)),
        [
            ("S25L4", (5, 15, 116, 36)),
            ("S25L5", (4, 14, 117, 37)),
            ("S25L6", (3, 13, 118, 38)),
            ("S25L7", (2, 12, 119, 39)),
        ]
    );
    assert_eq!(
        candidates((0.2, 0.2, 9.8, 9.8), (10, 10))[3].1,
        (0, 0, 10, 10)
    );
}

#[test]
fn source_gate_native_log_parser_derives_cpu_and_metal_buffers() {
    let cpu_load = br#"
load_tensors: offloaded 0/19 layers to GPU
load_tensors: CPU_Mapped model buffer size = 890.14 MiB
clip_ctx: CLIP using CPU backend
"#;
    assert_eq!(
        parse_native_load_log(cpu_load).unwrap(),
        ParsedLoadLog {
            offloaded_layers: 0,
            offloadable_layers: 19,
            model_buffer_bytes_by_backend: BTreeMap::from([(
                "CPU".into(),
                (890.14_f64 * 1024.0 * 1024.0).round() as u64,
            )]),
            mtmd_backend: "CPU".into(),
        }
    );

    let metal_load = br#"
load_tensors: offloaded 18/19 layers to GPU
load_tensors: MTL0 model buffer size = 840.00 MiB
load_tensors: CPU_Mapped model buffer size = 50.14 MiB
clip_ctx: CLIP using MTL0 backend
"#;
    let parsed = parse_native_load_log(metal_load).unwrap();
    assert_eq!(
        (parsed.offloaded_layers, parsed.offloadable_layers),
        (18, 19)
    );
    assert!(parsed.model_buffer_bytes_by_backend["Metal"] > 0);
    assert!(parsed.model_buffer_bytes_by_backend["CPU"] > 0);
    assert_eq!(parsed.mtmd_backend, "Metal");

    let inference = br#"
llama_context: CPU output buffer size = 0.39 MiB
llama_context: CPU output buffer size pending allocation
llama_kv_cache: MTL0 KV buffer size = 9.00 MiB
sched_reserve: MTL0 compute buffer size = 63.75 MiB
sched_reserve: CPU compute buffer size = 1.57 MiB
"#;
    let parsed = parse_native_inference_log(inference).unwrap();
    assert!(parsed.context_buffer_bytes_by_backend["CPU"] > 0);
    assert!(parsed.context_buffer_bytes_by_backend["Metal"] > 0);
    assert!(parsed.compute_buffer_bytes_by_backend["CPU"] > 0);
    assert!(parsed.compute_buffer_bytes_by_backend["Metal"] > 0);
}

#[test]
fn source_gate_native_log_parser_fails_closed_on_missing_actual_evidence() {
    assert!(parse_native_load_log(b"requested metal").is_err());
    assert!(parse_native_inference_log(b"inference completed").is_err());
    assert!(parse_native_inference_log(b"Vulkan compute buffer size = 1.00 MiB").is_err());
}

#[test]
fn source_gate_manifest_roi_matching_is_half_open_and_overlap_is_strict() {
    let roi = ValidatedHalfOpenRect {
        left: 10,
        top: 20,
        right: 30,
        bottom: 40,
    };
    assert!(rect_contains(roi, (10.0, 20.0)));
    assert!(rect_contains(roi, (29.999, 39.999)));
    assert!(!rect_contains(roi, (30.0, 40.0)));
    assert!(rect_intersects(roi, [29.0, 39.0, 31.0, 41.0]));
    assert!(!rect_intersects(roi, [30.0, 40.0, 31.0, 41.0]));
}

#[test]
fn source_ink_coverage_requires_every_ink_pixel() {
    let edit_roi = ValidatedHalfOpenRect {
        left: 2,
        top: 2,
        right: 5,
        bottom: 5,
    };
    let source_ink = [0, 1, 0, 0, 0, 0, 0, 1, 0];
    let mask = SourceInkMask::edit_roi(&source_ink, edit_roi);
    let mut support = [0; 64];

    support[2 * 8 + 3] = 1;
    support[4 * 8 + 3] = 1;
    assert!(source_ink_is_covered(8, edit_roi, mask, &support));

    support.fill(0);
    assert!(!source_ink_is_covered(8, edit_roi, mask, &support));

    support[2 * 8 + 3] = 1;
    assert!(!source_ink_is_covered(8, edit_roi, mask, &support));

    let mut page_source_ink = [0; 64];
    page_source_ink[2 * 8 + 3] = 1;
    page_source_ink[4 * 8 + 3] = 1;
    support[4 * 8 + 3] = 1;
    assert!(source_ink_is_covered(
        8,
        edit_roi,
        SourceInkMask::page(&page_source_ink, 8, 8),
        &support,
    ));
}

#[test]
fn source_gate_loaded_devices_come_from_enumerated_buffer_backends() {
    let enumerated = vec![
        EnumeratedDevice {
            index: 0,
            name: "CPU".into(),
            description: "Host CPU".into(),
            backend: "CPU".into(),
            device_type: "cpu".into(),
        },
        EnumeratedDevice {
            index: 1,
            name: "MTL0".into(),
            description: "Apple GPU".into(),
            backend: "MTL".into(),
            device_type: "integrated_gpu".into(),
        },
    ];
    let loaded = loaded_model_devices(
        &enumerated,
        &BTreeMap::from([("CPU".into(), 1), ("Metal".into(), 2)]),
    )
    .unwrap();
    assert_eq!(
        loaded
            .iter()
            .map(|device| device.backend.as_str())
            .collect::<Vec<_>>(),
        ["CPU", "Metal"]
    );
    assert!(loaded_model_devices(&enumerated, &BTreeMap::from([("CUDA".into(), 1)])).is_err());
}

#[test]
fn source_gate_selection_reports_each_failed_candidate_cell() {
    let mut evidence = calibration_evidence();
    for candidate in candidates_schema() {
        let failed = evidence
            .results
            .iter_mut()
            .find(|result| {
                result.entry_id == "r59-c01"
                    && result.process_evidence_id == "calibration-cpu"
                    && result.candidate_id == candidate.id
            })
            .unwrap();
        failed.derived.target_recall = 0.0;
        failed.derived.passed = false;
    }
    let error =
        select_smallest_all_pass(&evidence.results, &synthetic_entry_ids("calibration"))
            .unwrap_err()
            .to_string();
    for candidate in candidates_schema() {
        assert!(error.contains(&format!(
            "{}: r59-c01/cpu recall=0.000 protected=0 unmatched=0 rotation_excluded=true",
            candidate.id
        )));
    }
}

fn valid_environment(root: &Path) -> HashMap<&'static str, String> {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let manifest = root.join("visual-manifest.json");
    let manifest_bytes = serde_json::to_vec(&serde_json::json!({
        "entries": [
            {"id": "regression", "role": "regression"},
            {"id": "r59-c01", "role": "calibration"},
            {"id": "r59-c02", "role": "calibration"},
            {"id": "r59-c03", "role": "calibration"},
            {"id": "r59-c04", "role": "calibration"},
            {"id": "r59-h01", "role": "holdout"},
            {"id": "r59-h02", "role": "holdout"},
            {"id": "r59-h03", "role": "holdout"},
            {"id": "r59-h04", "role": "holdout"}
        ]
    }))
    .unwrap();
    fs::write(&manifest, &manifest_bytes).unwrap();
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let b0_sha = "a".repeat(40);
    let fixture_sha256 = "2".repeat(64);
    let required_check = write_required_check(
        root,
        Phase::CalibrationFreeze,
        &b0_sha,
        &manifest_sha256,
        &fixture_sha256,
    );
    HashMap::from([
        (PHASE_ENV, "calibration-freeze".into()),
        (B0_SHA_ENV, b0_sha),
        (
            VISUAL_INPUT_ENV,
            root.join("regression.png").to_string_lossy().into_owned(),
        ),
        (VISUAL_INPUT_SHA256_ENV, "0".repeat(64)),
        (
            VISUAL_EVIDENCE_ROOT_ENV,
            root.to_string_lossy().into_owned(),
        ),
        (VISUAL_MANIFEST_ENV, manifest.to_string_lossy().into_owned()),
        (VISUAL_MANIFEST_SHA256_ENV, manifest_sha256),
        (SOURCE_GATE_FIXTURE_SHA256_ENV, fixture_sha256),
        (
            ARTIFACT_ENV,
            root.join("selection.json").to_string_lossy().into_owned(),
        ),
        (
            REPORT_DIR_ENV,
            root.join("reports").to_string_lossy().into_owned(),
        ),
        (
            REQUIRED_CHECK_ENV,
            required_check.to_string_lossy().into_owned(),
        ),
    ])
}

fn parse_test_environment(
    values: &HashMap<&'static str, String>,
) -> io::Result<SelectionEnvironment> {
    SelectionEnvironment::parse_with_formal_paths(
        |name| values.get(name).cloned(),
        Some(FormalPublicPaths::frozen()),
    )
}

#[test]
fn formal_protocol_selection_retires_historical_custody_before_access() {
    assert_eq!(select_formal_revision(None, None).unwrap(), None);
    assert_eq!(select_formal_revision(None, Some("0")).unwrap(), None);
    assert_eq!(select_formal_revision(Some("0"), None).unwrap(), None);
    assert_eq!(
        select_formal_revision(None, Some("1"))
            .unwrap_err()
            .to_string(),
        HISTORICAL_CUSTODY_COMMAND_RETIRED
    );
    assert_eq!(
        select_formal_revision(Some("1"), None)
            .unwrap_err()
            .to_string(),
        HISTORICAL_CUSTODY_COMMAND_RETIRED
    );
    assert!(select_formal_revision(None, Some("true")).is_err());
}

#[test]
fn r52_bridge_retires_before_request_access() {
    assert_eq!(
        run_r52_evidence_bridge().unwrap_err().to_string(),
        HISTORICAL_CUSTODY_COMMAND_RETIRED
    );
}

#[test]
fn formal_protocol_environment_retires_before_model_access() {
    let root = tempfile::tempdir().unwrap();
    let mut values = valid_environment(root.path());
    values.insert(R60_FORMAL_CUSTODY_ENV, "1".into());

    let error = parse_test_environment(&values).err().expect("must reject");
    assert_eq!(error.to_string(), HISTORICAL_CUSTODY_COMMAND_RETIRED);
}

#[test]
fn r60_receipts_are_strict_and_dispatch_to_the_r60_bundle_validator() {
    let layout = R60LayoutReceipt {
        schema: "hanonly.r60.layout-receipt.v1".into(),
        plan_revision: 60,
        manifest_sha256: synthetic_hash(4),
        private_manifest_commitment_sha256: synthetic_hash(4),
        member_name_digest_sha256: synthetic_hash(5),
        ciphertext_sha256: synthetic_hash(1),
        layout_validator_sha256: synthetic_hash(3),
        entry_ids: FormalRevision::R60.entry_ids(),
        required_root_present: true,
        wrapper_absent: true,
        canonical_ustar_pass: true,
        manifest_binding_pass: true,
        same_archive_object_pass: true,
        layout_pass: true,
        restricted_values_disclosed: false,
    };
    let layout_bytes = canonical_json(&layout).unwrap();
    let layout_sha256 = sha256_hex(&layout_bytes);
    let public = R60PublicCommitment {
        schema: "hanonly.r60.public-commitment.v1".into(),
        plan_revision: 60,
        source_b0_sha: R60_SOURCE_B0_SHA.into(),
        ciphertext_sha256: synthetic_hash(1),
        layout_receipt_sha256: layout_sha256.clone(),
        layout_validator_sha256: synthetic_hash(3),
        manifest_sha256: synthetic_hash(4),
        member_name_digest_sha256: synthetic_hash(5),
        private_manifest_commitment_sha256: synthetic_hash(4),
        entry_ids: FormalRevision::R60.entry_ids(),
        cleanup_pass: true,
        restricted_values_disclosed: false,
        start_marker_absent: true,
    };
    let public_bytes = canonical_json(&public).unwrap();
    let public_sha256 = sha256_hex(&public_bytes);
    let successor = R60SuccessorCommitment {
        schema: "hanonly.r60.successor-commitment.v1".into(),
        plan_revision: 60,
        public_commitment_sha256: public_sha256.clone(),
        source_b0_sha: R60_SOURCE_B0_SHA.into(),
        successor_b0_sha: "b".repeat(40),
        contract_sha256: R60_CONTRACT_SHA256.into(),
        test_spec_sha256: R60_TEST_SPEC_SHA256.into(),
        calibration_artifact_sha256: R59_CALIBRATION_ARTIFACT_SHA256.into(),
        selected_candidate_id: "S25L4".into(),
        ciphertext_sha256: synthetic_hash(1),
        layout_receipt_sha256: layout_sha256.clone(),
        layout_validator_sha256: synthetic_hash(3),
        manifest_sha256: synthetic_hash(4),
        member_name_digest_sha256: synthetic_hash(5),
        private_manifest_commitment_sha256: synthetic_hash(4),
        entry_ids: FormalRevision::R60.entry_ids(),
        package_unchanged: true,
        start_marker_absent: true,
    };
    let successor_bytes = canonical_json(&successor).unwrap();
    let freeze = validate_r60_successor_commitments(
        &layout_bytes,
        &public_bytes,
        &public_sha256,
        &successor_bytes,
        &sha256_hex(&successor_bytes),
        &successor.successor_b0_sha,
        R60_CONTRACT_SHA256,
        R60_TEST_SPEC_SHA256,
    )
    .unwrap();

    let start = R60OpenMarker {
        schema: "hanonly.r60.holdout-start.v1".into(),
        plan_revision: 60,
        b0_sha: successor.successor_b0_sha.clone(),
        public_commitment_sha256: public_sha256,
        successor_commitment_sha256: freeze.receipt_sha256.clone(),
        calibration_artifact_sha256: R59_CALIBRATION_ARTIFACT_SHA256.into(),
        selected_candidate_id: "S25L4".into(),
        entry_ids: FormalRevision::R60.entry_ids(),
        pre_holdout_attestation_sha256: synthetic_hash(6),
        nonce_hex: synthetic_hash(7),
        state: "started".into(),
    };
    let start_bytes = canonical_json(&start).unwrap();
    validate_r60_start_receipt(
        &start_bytes,
        &successor.successor_b0_sha,
        "S25L4",
        &freeze,
        &start.pre_holdout_attestation_sha256,
    )
    .unwrap();

    let runtime = R60RuntimeCommitment {
        schema: "hanonly.r60.runtime-commitment.v1".into(),
        plan_revision: 60,
        b0_sha: successor.successor_b0_sha.clone(),
        start_marker_sha256: sha256_hex(&start_bytes),
        successor_commitment_sha256: freeze.receipt_sha256.clone(),
        ciphertext_sha256: synthetic_hash(1),
        layout_receipt_sha256: layout_sha256,
        layout_validator_sha256: synthetic_hash(3),
        member_name_digest_sha256: synthetic_hash(5),
        private_manifest_commitment_sha256: synthetic_hash(4),
        calibration_artifact_sha256: R59_CALIBRATION_ARTIFACT_SHA256.into(),
        selected_candidate_id: "S25L4".into(),
        plaintext_archive_sha256: synthetic_hash(8),
        manifest_sha256: synthetic_hash(4),
        oracle_sha256: synthetic_hash(9),
        hashes_sha256: synthetic_hash(10),
        entry_ids: FormalRevision::R60.entry_ids(),
        decrypt_pass: true,
        package_unchanged: true,
        restricted_values_disclosed: false,
        state: "runtime_committed".into(),
    };
    let runtime_bytes = canonical_json(&runtime).unwrap();
    let validated = validate_r60_runtime_receipt(
        &runtime_bytes,
        &successor.successor_b0_sha,
        &runtime.start_marker_sha256,
        &freeze,
    )
    .unwrap();
    assert_eq!(validated.manifest_sha256, runtime.manifest_sha256);
    assert_eq!(validated.oracle_sha256, runtime.oracle_sha256);
    assert_eq!(validated.hashes_sha256, runtime.hashes_sha256);

    let mut invalid_archive_hash: serde_json::Value =
        serde_json::from_slice(&runtime_bytes).unwrap();
    invalid_archive_hash["plaintext_archive_sha256"] = serde_json::json!("not-a-sha");
    assert!(
        validate_r60_runtime_receipt(
            &canonical_json(&invalid_archive_hash).unwrap(),
            &successor.successor_b0_sha,
            &runtime.start_marker_sha256,
            &freeze,
        )
        .is_err()
    );

    let mut open_runtime: serde_json::Value = serde_json::from_slice(&runtime_bytes).unwrap();
    open_runtime["runtime_manifest_sha256"] = serde_json::json!(synthetic_hash(12));
    assert!(
        validate_r60_runtime_receipt(
            &canonical_json(&open_runtime).unwrap(),
            &successor.successor_b0_sha,
            &runtime.start_marker_sha256,
            &freeze,
        )
        .is_err()
    );
}

#[test]
fn r60_cells_use_actual_metal_and_execution_only_generations() {
    let expected = vec![
        "r60-h01/cpu",
        "r60-h01/actual-metal",
        "r60-h02/cpu",
        "r60-h02/actual-metal",
        "r60-h03/cpu",
        "r60-h03/actual-metal",
        "r60-h04/cpu",
        "r60-h04/actual-metal",
    ];
    assert_eq!(FormalRevision::R60.formal_cell_keys(), expected);
    assert_eq!(FormalRevision::R60.diagnostic_generation_bounds(), (0, 16));
    assert_eq!(FormalRevision::R60.artifact_namespace(), "r60");
    assert_eq!(
        FormalRevision::R60.completion_summary_contract(),
        "hanonly-r60-b0-completion-summary-v1"
    );
    assert_eq!(
        FormalRevision::R60.completion_summary_stdout_prefix(),
        R60_COMPLETION_SUMMARY_STDOUT_PREFIX
    );
    assert_eq!(
        formal_external_evidence_suffix(
            Some(FormalRevision::R60),
            &format!(
                "source-gate/holdout/{}/load.log",
                FormalRevision::R60.external_device("metal")
            ),
        ),
        "r60/source-gate/holdout/actual-metal/load.log"
    );
    assert_eq!(
        formal_artifact_suffix(Some(FormalRevision::R60), "r59/diagnostic-index.json"),
        "r60/diagnostic-index.json"
    );

    let pass = synthetic_formal_run(
        expected
            .iter()
            .map(|key| {
                let (entry, device) = key.split_once('/').unwrap();
                synthetic_formal_cell(entry, device, true)
            })
            .collect(),
    );
    assert_eq!(
        validate_formal_terminal_closure(FormalRevision::R60, "S25L4", &pass).unwrap(),
        expected
    );

    let mut cells = pass.cells[..2].to_vec();
    cells[1] = synthetic_formal_cell("r60-h01", "actual-metal", false);
    assert!(
        validate_formal_terminal_closure(
            FormalRevision::R60,
            "S25L4",
            &synthetic_formal_run(cells.clone()),
        )
        .is_ok()
    );
    cells.push(synthetic_formal_cell("r60-h02", "cpu", true));
    assert!(
        validate_formal_terminal_closure(
            FormalRevision::R60,
            "S25L4",
            &synthetic_formal_run(cells),
        )
        .is_err()
    );
}

#[test]
fn formal_successor_accepts_only_exact_calibration_artifact_binding() {
    let original_b0_sha = "a".repeat(40);
    let successor_b0_sha = "b".repeat(40);
    let artifact_sha256 = synthetic_hash(5);
    let freeze = FreezeCommitments {
        receipt_sha256: synthetic_hash(1),
        original_public_commitment_sha256: synthetic_hash(2),
        original_b0_sha: original_b0_sha.clone(),
        successor_b0_sha: successor_b0_sha.clone(),
        calibration_artifact_sha256: artifact_sha256.clone(),
        ciphertext_sha256: synthetic_hash(3),
        private_manifest_commitment_sha256: synthetic_hash(4),
        r60_layout: None,
    };

    assert!(freeze.accepts_calibration_artifact(
        &original_b0_sha,
        &successor_b0_sha,
        &artifact_sha256,
    ));
    assert!(!freeze.accepts_calibration_artifact(
        &"c".repeat(40),
        &successor_b0_sha,
        &artifact_sha256
    ));
    assert!(!freeze.accepts_calibration_artifact(
        &original_b0_sha,
        &"d".repeat(40),
        &artifact_sha256
    ));
    assert!(!freeze.accepts_calibration_artifact(
        &original_b0_sha,
        &successor_b0_sha,
        &synthetic_hash(6),
    ));
}

#[test]
fn formal_successor_holds_exact_artifact_and_uses_phase_b0_attestations() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let mut values = valid_environment(&root);
    run_with(
        |name| values.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| Ok(calibration_evidence()),
    )
    .unwrap();

    let artifact_path = root.join("selection.json");
    let input = HeldInput::open(&artifact_path).unwrap();
    let mut artifact: FrozenArtifact = serde_json::from_slice(input.bytes()).unwrap();
    let original_b0_sha = artifact.b0_sha.clone();
    let successor_b0_sha = "b".repeat(40);
    let mut environment = parse_test_environment(&values).unwrap();
    environment.phase = Phase::Holdout;
    environment.b0_sha.clone_from(&successor_b0_sha);
    environment.formal_custody = Some(FormalCustody {
        revision: FormalRevision::R60,
        contract_sha256: synthetic_hash(7),
        holdout: Some(HoldoutCustody {
            directory: root.join("custody"),
            plaintext_directory: root.join("plaintext"),
            plaintext_archive: root.join("plaintext/bundle.tar"),
            freeze: FreezeCommitments {
                receipt_sha256: synthetic_hash(8),
                original_public_commitment_sha256: synthetic_hash(9),
                original_b0_sha: original_b0_sha.clone(),
                successor_b0_sha: successor_b0_sha.clone(),
                calibration_artifact_sha256: hex_sha256(input.sha256()),
                ciphertext_sha256: synthetic_hash(10),
                private_manifest_commitment_sha256: synthetic_hash(11),
                r60_layout: None,
            },
            expected_start_marker_sha256: synthetic_hash(12),
            open_marker: OnceCell::new(),
            runtime_commitment: OnceCell::new(),
        }),
    });

    hold_calibration_artifact(&environment, &input, &artifact).unwrap();
    validate_artifact(&artifact, Phase::CalibrationFreeze, &environment).unwrap();
    input.with_revalidated_path(|_| Ok(())).unwrap();

    values.insert(PHASE_ENV, "holdout".into());
    values.insert(B0_SHA_ENV, successor_b0_sha);
    set_required_check(&mut values, &root, Phase::Holdout);
    let holdout_environment = parse_test_environment(&values).unwrap();
    artifact
        .required_checks
        .push(holdout_environment.required_check.clone());
    validate_required_checks(&artifact, Phase::Holdout, &holdout_environment).unwrap();

    fs::write(&artifact_path, b"drifted calibration").unwrap();
    assert!(input.with_revalidated_path(|_| Ok(())).is_err());
}

#[test]
fn formal_terminal_pass_stays_non_authorizing_until_cleanup_receipt() {
    assert_eq!(
        pre_cleanup_completion_state(true),
        (
            "incomplete_non_authorizing",
            "terminal_pass_cleanup_pending"
        )
    );
    assert_eq!(
        pre_cleanup_completion_state(false),
        ("incomplete_non_authorizing", "completed_fail")
    );
}

fn write_required_check(
    root: &Path,
    phase: Phase,
    b0_sha: &str,
    manifest_sha256: &str,
    fixture_sha256: &str,
) -> PathBuf {
    let directory = root.join("source-gate-selection/checks");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .unwrap();
    fs::set_permissions(
        root.join("source-gate-selection"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let attestation = RequiredCheckAttestation {
        version: 1,
        mode: "b0-source-gate-anti-fixture".into(),
        phase: required_check_phase(phase).into(),
        b0_sha: b0_sha.into(),
        manifest_sha256: manifest_sha256.into(),
        source_gate_fixture_manifest_sha256: fixture_sha256.into(),
        checker_endpoint_sha256: sha256_file(
            &repository_root().unwrap().join(CHECKER_ENDPOINT),
        )
        .unwrap(),
        scanned_roots: ANTI_FIXTURE_SCANNED_ROOTS
            .iter()
            .map(|value| (*value).into())
            .collect(),
        allowed_descriptor_roots: ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS
            .iter()
            .map(|value| (*value).into())
            .collect(),
        policy_scan_sha256: "3".repeat(64),
        result: "pass".into(),
    };
    let path = root.join(required_check_relpath(phase));
    fs::write(&path, canonical_json(&attestation).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn set_required_check(values: &mut HashMap<&'static str, String>, root: &Path, phase: Phase) {
    let path = write_required_check(
        root,
        phase,
        values.get(B0_SHA_ENV).unwrap(),
        values.get(VISUAL_MANIFEST_SHA256_ENV).unwrap(),
        values.get(SOURCE_GATE_FIXTURE_SHA256_ENV).unwrap(),
    );
    values.insert(REQUIRED_CHECK_ENV, path.to_string_lossy().into_owned());
}

fn test_repository() -> PathBuf {
    repository_root().unwrap()
}

fn synthetic_hash(value: u8) -> String {
    format!("{value:064x}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn synthetic_process(phase: &str, device: &str) -> ProcessEvidence {
    let metal = device == "metal";
    ProcessEvidence {
        id: format!("{phase}-{device}"),
        phase: phase.into(),
        requested_device: device.into(),
        paddle_instance_id: if metal {
            "2".repeat(32)
        } else {
            "1".repeat(32)
        },
        executable_sha256: synthetic_hash(1),
        model_artifact_sha256: ModelArtifactHashes {
            pp_detection: synthetic_hash(2),
            pp_recognition: synthetic_hash(3),
            pp_recognition_config: synthetic_hash(4),
            vl_model: synthetic_hash(5),
            vl_mmproj: synthetic_hash(6),
        },
        runtime_library_sha256: BTreeMap::from([(
            "/usr/lib/libsynthetic.dylib".into(),
            synthetic_hash(7),
        )]),
        load_evidence: LoadEvidence {
            cpu_forced: !metal,
            gpu_offload_supported: metal,
            n_gpu_layers: if metal { B0_DEFAULT_GPU_LAYERS } else { 0 },
            mtmd_use_gpu: metal,
            word_boxes_backend: "rten_cpu".into(),
            raw_load_log_relpath: format!("source-gate/{phase}/{device}/load.log"),
            raw_load_log_sha256: synthetic_hash(8),
            enumerated_devices: Vec::new(),
            loaded_model_devices: vec![LoadedModelDevice {
                model_device_ordinal: 0,
                name: if metal { "Apple GPU" } else { "CPU" }.into(),
                backend: if metal { "Metal" } else { "CPU" }.into(),
                device_type: if metal { "integrated_gpu" } else { "cpu" }.into(),
            }],
            offloaded_layers: if metal { 32 } else { 0 },
            offloadable_layers: 39,
            model_buffer_bytes_by_backend: BTreeMap::from([
                ("CPU".into(), 1),
                ("Metal".into(), u64::from(metal)),
            ]),
            mtmd_backend: if metal { "Metal" } else { "CPU" }.into(),
        },
    }
}

fn synthetic_result(
    phase: &str,
    entry_id: &str,
    device: &str,
    candidate_id: &str,
) -> SelectionResult {
    let metal = device == "metal";
    SelectionResult {
        entry_id: entry_id.into(),
        process_evidence_id: format!("{phase}-{device}"),
        candidate_id: candidate_id.into(),
        execution_evidence: ExecutionEvidence {
            paddle_instance_id: if metal {
                "2".repeat(32)
            } else {
                "1".repeat(32)
            },
            context_offload_kqv: metal,
            context_op_offload: metal,
            inference_completed: true,
            raw_inference_log_relpath: format!(
                "source-gate/{phase}/{entry_id}/{device}/{candidate_id}.log"
            ),
            raw_inference_log_sha256: synthetic_hash(9),
            source_gate_diagnostic_relpath: format!(
                "source-gate/{phase}/{entry_id}/{device}/{candidate_id}.source-gate.json"
            ),
            source_gate_diagnostic_sha256: synthetic_hash(10),
            context_buffer_bytes_by_backend: BTreeMap::from([
                ("CPU".into(), 1),
                ("Metal".into(), u64::from(metal)),
            ]),
            compute_buffer_bytes_by_backend: BTreeMap::from([
                ("CPU".into(), 1),
                ("Metal".into(), u64::from(metal)),
            ]),
        },
        runtime_nodes: vec![RuntimeNode {
            node_id: format!("{entry_id}-node"),
            recognition_anchor: [0.0, 0.0, 1.0, 1.0],
            node_rotation: 0.0,
            text_rotation: 0.0,
            selected_as_han: true,
        }],
        derived: DerivedEvidence {
            actual_device: device.into(),
            matched_target_ids: vec!["target".into()],
            selected_target_ids: vec!["target".into()],
            selected_protected_node_ids: Vec::new(),
            selected_rotation_target_ids: Vec::new(),
            unmatched_selected_node_ids: Vec::new(),
            target_recall: 1.0,
            protected_false_positive_count: 0,
            rotation_targets_excluded: true,
            source_coverage_preflight: SourceCoveragePreflight {
                pp_han_scalar_count: 1,
                vl_expected_han_scalar_count: 1,
                pp_vl_complete_coverage: true,
                rejected_after_vl: false,
                pp_vl_incomplete_coverage: false,
                covered_source_roi_ids: vec!["target".into()],
                source_text_roi_coverage: 1.0,
                source_removal_preflight_passed: true,
            },
            passed: true,
        },
    }
}

#[test]
fn source_gate_coverage_uses_raster_proof_not_pp_vl_count_equality() {
    let process = synthetic_process("calibration", "cpu");
    let processes = HashMap::from([(process.id.as_str(), &process)]);
    let mut result = synthetic_result("calibration", "r59-c01", "cpu", "S25L4");
    result.derived.source_coverage_preflight.pp_han_scalar_count = 0;
    result
        .derived
        .source_coverage_preflight
        .vl_expected_han_scalar_count = 4;

    assert!(validate_result(&result, &processes, "calibration").is_ok());
}

fn r59_test_schema_and_oracle() -> (VisualManifestEntry, OracleValidatedEntry) {
    let schema = serde_json::from_value(serde_json::json!({
        "id": "r59-c01",
        "path": "source.png",
        "sha256": synthetic_hash(40),
        "decoded_rgba_blake3": synthetic_hash(41),
        "clean_reference_path": "clean.png",
        "clean_reference_sha256": synthetic_hash(42),
        "clean_reference_decoded_rgba_blake3": synthetic_hash(43),
        "role": "calibration",
        "dimension_bin": "lt720",
        "aspect": "square_or_near",
        "background": "pure",
        "targets": [{
            "id": "target",
            "source_roi": [0, 0, 50, 50],
            "clean_reference_edit_roi": [0, 0, 50, 50],
            "erase_source_ink_mask_path": "erase.bin",
            "erase_source_ink_mask_sha256": synthetic_hash(44),
            "residual_source_ink_mask_path": "residual.bin",
            "residual_source_ink_mask_sha256": synthetic_hash(45),
            "position": "interior",
            "writing": "horizontal",
            "effect": "plain",
            "translation_length": "equal",
            "expected": "automatic_strict"
        }],
        "protected_rois": [[50, 0, 64, 64]],
        "multi_node": false
    }))
    .unwrap();
    let oracle = OracleValidatedEntry {
        protected_rois: vec![ValidatedHalfOpenRect {
            left: 50,
            top: 0,
            right: 64,
            bottom: 64,
        }],
        targets: vec![OracleValidatedTarget {
            source_roi: ValidatedHalfOpenRect {
                left: 0,
                top: 0,
                right: 50,
                bottom: 50,
            },
            edit_roi: ValidatedHalfOpenRect {
                left: 0,
                top: 0,
                right: 50,
                bottom: 50,
            },
            delta_mask: vec![1; 50 * 50].into_boxed_slice(),
        }],
    };
    (schema, oracle)
}

#[test]
fn empty_prepared_masks_record_zero_support_and_failed_coverage() {
    for prepared in [
        PreparedInpaintMask::NoEligibleHanTargets,
        PreparedInpaintMask::EmptyMask,
    ] {
        let support = removal_support_from_prepared(prepared, 64, 64);
        assert_eq!(support.dimensions(), (64, 64));
        assert!(support.pixels().all(|pixel| pixel.0[0] == 0));
    }

    let mut prepared = GrayImage::new(3, 2);
    prepared.put_pixel(1, 1, image::Luma([255]));
    let support = removal_support_from_prepared(
        PreparedInpaintMask::Prepared {
            mask: DynamicImage::ImageLuma8(prepared.clone()),
            blocks: Vec::new(),
        },
        64,
        64,
    );
    assert_eq!(support.as_raw(), prepared.as_raw());

    let (schema, oracle) = r59_test_schema_and_oracle();
    let scene = scene_for_entry(&schema, &oracle, 64, 64);
    let page = *scene.pages.keys().next().unwrap();
    let ink = vec![255; 64 * 64];
    let (_, derived, _) = derive_result(
        "cpu",
        &scene,
        page,
        &schema,
        &oracle,
        &[SourceInkMask::page(&ink, 64, 64)],
        &GrayImage::new(64, 64),
        "lama-manga",
        "speech-bubble-segmentation",
        &synthetic_hash(90),
        &[],
    )
    .unwrap();
    assert_eq!(
        derived.source_coverage_preflight.source_text_roi_coverage,
        0.0
    );
    assert!(
        !derived
            .source_coverage_preflight
            .source_removal_preflight_passed
    );
    assert!(!derived.passed);
}

fn r59_test_quad_bits(left: f32, top: f32, right: f32, bottom: f32) -> [u32; 8] {
    [
        left.to_bits(),
        top.to_bits(),
        right.to_bits(),
        top.to_bits(),
        right.to_bits(),
        bottom.to_bits(),
        left.to_bits(),
        bottom.to_bits(),
    ]
}

#[test]
fn r57_detector_geometry_rejects_each_one_pixel_mutation() {
    let baseline = vec![0, 1, 1, 0];
    assert!(r57_detector_supports_equal(
        &baseline, &baseline, &baseline, &baseline
    ));
    for changed in 0..4 {
        let mut supports = [
            baseline.clone(),
            baseline.clone(),
            baseline.clone(),
            baseline.clone(),
        ];
        supports[changed][0] = 1;
        assert!(!r57_detector_supports_equal(
            &supports[0],
            &supports[1],
            &supports[2],
            &supports[3],
        ));
    }
}

#[test]
fn r57_calibration_and_probe_require_zero_protected_overlap() {
    assert!(r57_cell_passed(true, true, 0));
    assert!(!r57_cell_passed(true, true, 1));

    let stage = |missing_pixels, protected_overlap_pixels| EraseStageMetric {
        stage: EraseDiagnosticStage::InpaintFinal,
        branch: EraseDiagnosticBranch::HanOnly,
        grayscale_blake3: synthetic_hash(91),
        nonzero_pixels: 1,
        protected_overlap_pixels,
        targets: vec![EraseStageTargetMetric {
            target_id: "target".into(),
            oracle_pixels: 1,
            intersection_pixels: 1_u64.saturating_sub(missing_pixels),
            missing_pixels,
        }],
    };
    assert!(r57_final_erase_stage_passed(&stage(0, 0)));
    assert!(!r57_final_erase_stage_passed(&stage(1, 0)));
    assert!(!r57_final_erase_stage_passed(&stage(0, 1)));
}

#[test]
fn r57_actual_scene_support_rejects_transform_and_rotation_drift() {
    let baseline = Transform {
        x: 10.0,
        y: 10.0,
        width: 20.0,
        height: 20.0,
        rotation_deg: 0.0,
    };
    let text = TextData::default();
    let (support, rotations_zero) = r57_actual_scene_support(64, 64, &baseline, &text).unwrap();
    assert!(rotations_zero);

    let expanded = Transform {
        width: 21.0,
        ..baseline.clone()
    };
    assert_ne!(
        support.mask,
        r57_actual_scene_support(64, 64, &expanded, &text)
            .unwrap()
            .0
            .mask
    );

    let node_rotated = Transform {
        rotation_deg: 1.0,
        ..baseline.clone()
    };
    assert!(
        !r57_actual_scene_support(64, 64, &node_rotated, &text)
            .unwrap()
            .1
    );

    let text_rotated = TextData {
        rotation_deg: Some(1.0),
        ..Default::default()
    };
    assert!(
        !r57_actual_scene_support(64, 64, &baseline, &text_rotated)
            .unwrap()
            .1
    );
}

#[test]
fn r59_selection_geometry_closes_detector_ownership_preimages() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);
    let environment = parse_test_environment(&values).unwrap();
    let (schema, oracle) = r59_test_schema_and_oracle();
    let result = synthetic_result("calibration", "r59-c01", "cpu", "S25L4");
    let node_id = NodeId::new();
    let target_bits = r59_test_quad_bits(10.0, 10.0, 20.0, 20.0);
    let second_target_bits = r59_test_quad_bits(25.0, 10.0, 35.0, 20.0);
    let protected_bits = r59_test_quad_bits(52.0, 10.0, 60.0, 20.0);
    let diagnostics = vec![
        SourceGateDiagnosticEvent::Input {
            backend: "pp-ocr-v5",
            width: 64,
            height: 64,
            decoded_rgba_hash: synthetic_hash(46),
        },
        SourceGateDiagnosticEvent::Crop {
            candidate_index: 0,
            node_id,
            bounds: [0, 0, 64, 64],
            crop_rgba_hash: synthetic_hash(47),
            vl_bounds: [0, 0, 64, 64],
            vl_crop_rgba_hash: synthetic_hash(48),
        },
        SourceGateDiagnosticEvent::PpSummary {
            node_id,
            words: Vec::new(),
            raw_detectors: vec![
                PpDetectorDiagnostic {
                    occurrence_index: 0,
                    source_scaled_quad_f32_bits: target_bits,
                },
                PpDetectorDiagnostic {
                    occurrence_index: 1,
                    source_scaled_quad_f32_bits: second_target_bits,
                },
                PpDetectorDiagnostic {
                    occurrence_index: 2,
                    source_scaled_quad_f32_bits: protected_bits,
                },
            ],
            canonical_lines: vec![
                PpCanonicalLineDiagnostic {
                    line_index: 0,
                    detector_occurrences: vec![PpCanonicalOccurrenceDiagnostic {
                        occurrence_index: 0,
                        canonical_corners_f32_bits: target_bits,
                    }],
                    recognition: Some(PpRecognitionDiagnostic {
                        present: true,
                        recognition_class: "han",
                        segment_count: 1,
                    }),
                },
                PpCanonicalLineDiagnostic {
                    line_index: 1,
                    detector_occurrences: vec![PpCanonicalOccurrenceDiagnostic {
                        occurrence_index: 1,
                        canonical_corners_f32_bits: second_target_bits,
                    }],
                    recognition: Some(PpRecognitionDiagnostic {
                        present: true,
                        recognition_class: "han",
                        segment_count: 1,
                    }),
                },
                PpCanonicalLineDiagnostic {
                    line_index: 2,
                    detector_occurrences: vec![PpCanonicalOccurrenceDiagnostic {
                        occurrence_index: 2,
                        canonical_corners_f32_bits: protected_bits,
                    }],
                    recognition: Some(PpRecognitionDiagnostic {
                        present: true,
                        recognition_class: "protected_latin",
                        segment_count: 1,
                    }),
                },
            ],
        },
        SourceGateDiagnosticEvent::SelectionGeometry {
            node_id,
            targets: vec![
                SourceGateTargetGeometryDiagnostic {
                    scene_quad_f32_bits: target_bits,
                    eligible_line_quads_f32_bits: vec![target_bits],
                },
                SourceGateTargetGeometryDiagnostic {
                    scene_quad_f32_bits: second_target_bits,
                    eligible_line_quads_f32_bits: vec![second_target_bits],
                },
            ],
            protected_lines: vec![SourceGateTargetGeometryDiagnostic {
                scene_quad_f32_bits: protected_bits,
                eligible_line_quads_f32_bits: vec![protected_bits],
            }],
            detector_ownership: vec![
                SourceGateDetectorOwnershipDiagnostic {
                    occurrence_index: 0,
                    canonical_line_index: Some(0),
                    scene_quad_f32_bits: target_bits,
                    eligible_text_line_quad_f32_bits: Some(target_bits),
                    assignment: SourceGateDetectorAssignmentDiagnostic::Target {
                        target_index: 0,
                    },
                },
                SourceGateDetectorOwnershipDiagnostic {
                    occurrence_index: 1,
                    canonical_line_index: Some(1),
                    scene_quad_f32_bits: second_target_bits,
                    eligible_text_line_quad_f32_bits: Some(second_target_bits),
                    assignment: SourceGateDetectorAssignmentDiagnostic::Target {
                        target_index: 1,
                    },
                },
                SourceGateDetectorOwnershipDiagnostic {
                    occurrence_index: 2,
                    canonical_line_index: Some(2),
                    scene_quad_f32_bits: protected_bits,
                    eligible_text_line_quad_f32_bits: Some(protected_bits),
                    assignment: SourceGateDetectorAssignmentDiagnostic::Protected {
                        protected_index: 0,
                    },
                },
            ],
        },
    ];
    let target_support = r59_rect_mask(64, 64, r59_quad_bits_rect(target_bits).unwrap());
    let second_target_support =
        r59_rect_mask(64, 64, r59_quad_bits_rect(second_target_bits).unwrap());
    let supports = CellSupportEvidence {
        width: 64,
        height: 64,
        scene_by_target: BTreeMap::from([(
            "target".to_owned(),
            vec![
                SceneSupportEvidence {
                    rect: [10, 10, 20, 20],
                    mask: target_support.clone(),
                    downstream_mask: target_support.clone(),
                },
                SceneSupportEvidence {
                    rect: [25, 10, 35, 20],
                    mask: second_target_support.clone(),
                    downstream_mask: second_target_support,
                },
            ],
        )]),
        selected_scene_rotations_zero: true,
        runtime_inpainter_id: "lama-manga".to_owned(),
        bubble_segmenter_id: "speech-bubble-segmentation".to_owned(),
        bubble_support_sha256: synthetic_hash(90),
        removal_support: vec![0; 64 * 64],
    };
    let (_, _, _, records, geometry_passed) = r59_detector_diagnostics(
        &environment,
        &result,
        &schema,
        &oracle,
        &diagnostics,
        &None,
        Some(&supports),
    )
    .unwrap();
    assert!(geometry_passed);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["preimage"]["target_id"], "target");
    assert_eq!(
        records[0]["preimage"]["canonical_assignment"],
        "selected_han"
    );
    assert_eq!(records[0]["preimage"]["ownership_verdict"], "unique");
    assert_eq!(
        records[0]["preimage"]["emitted_scene_quad"],
        serde_json::json!([10, 10, 20, 10, 20, 20, 10, 20])
    );
    assert_eq!(
        records[0]["preimage"]["detector_support_mask"],
        records[0]["preimage"]["line_support_mask"]
    );
    assert_eq!(
        records[0]["preimage"]["detector_support_mask"],
        records[0]["preimage"]["emitted_scene_support_mask"]
    );
    assert_eq!(
        records[0]["preimage"]["detector_support_mask"],
        records[0]["preimage"]["downstream_line_support_mask"]
    );
    assert_eq!(
        records[1]["preimage"]["canonical_assignment"],
        "selected_han"
    );
    assert_eq!(
        records[2]["preimage"]["canonical_assignment"],
        "preserved_source"
    );
    assert!(
        records[2]["preimage"]["protected_support_pixels"]
            .as_u64()
            .unwrap()
            > 0
    );
    let rejected_reason = Some("pp_vl_incomplete_coverage".to_owned());
    let (_, _, _, rejected, _) = r59_detector_diagnostics(
        &environment,
        &result,
        &schema,
        &oracle,
        &diagnostics[..3],
        &rejected_reason,
        None,
    )
    .unwrap();
    assert_eq!(rejected.len(), 3);
    assert!(rejected.iter().all(|record| {
        record["preimage"]["ownership_verdict"] == "unassigned"
            && record["preimage"]["selection_verdict"] == "rejected"
            && record["preimage"]["emitted_scene_quad"].is_null()
            && record["preimage"]["detector_support_mask"].is_object()
            && record["preimage"]["line_support_mask"].is_object()
            && record["preimage"]["agreed_mask"].is_object()
    }));
}

#[test]
fn r59_validated_execution_view_preserves_local_coverage_mask() {
    let mut page_mask = vec![0_u8; 64 * 64];
    for y in 10..20 {
        page_mask[y * 64 + 10..y * 64 + 20].fill(1);
    }
    let prepared = prepare_r59_execution_entries(R59ValidatedExecutionView {
        entries: vec![R59ValidatedExecutionEntry {
            id: "r59-h01".into(),
            source_encoded_bytes: vec![1].into_boxed_slice(),
            clean_reference_encoded_bytes: vec![2].into_boxed_slice(),
            validated_source_rgba: RgbaImage::new(64, 64),
            validated_clean_reference_rgba: RgbaImage::new(64, 64),
            source_width: 64,
            source_height: 64,
            clean_width: 64,
            clean_height: 64,
            protected_rois: vec![[40, 40, 50, 50]],
            targets: vec![R59ValidatedExecutionTarget {
                id: "target".into(),
                source_roi: [10, 10, 20, 20],
                clean_reference_edit_roi: [10, 10, 20, 20],
                erase_source_ink_mask_encoded_bytes: vec![3].into_boxed_slice(),
                residual_source_ink_mask_encoded_bytes: vec![3].into_boxed_slice(),
                validated_binary_mask: page_mask.into_boxed_slice(),
                expected: "automatic_strict".into(),
                writing: "horizontal".into(),
                effect: "plain".into(),
                position: "interior".into(),
                translation_length: "short".into(),
            }],
        }],
    })
    .unwrap();

    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].0.id, "r59-h01");
    assert_eq!(prepared[0].0.targets[0].id, "target");
    assert_eq!(&*prepared[0].2.targets[0].delta_mask, &[1; 100]);
    assert_eq!(
        prepared[0].2.protected_rois[0],
        ValidatedHalfOpenRect {
            left: 40,
            top: 40,
            right: 50,
            bottom: 50,
        }
    );
}

#[test]
fn r59_selected_and_downstream_support_have_independent_geometry_sources() {
    let (schema, oracle) = r59_test_schema_and_oracle();
    let node_id = NodeId::new();
    let diagnostics = vec![SourceGateDiagnosticEvent::SelectionGeometry {
        node_id,
        targets: vec![SourceGateTargetGeometryDiagnostic {
            scene_quad_f32_bits: r59_test_quad_bits(10.0, 10.0, 20.0, 20.0),
            eligible_line_quads_f32_bits: vec![r59_test_quad_bits(10.0, 10.0, 20.0, 20.0)],
        }],
        protected_lines: Vec::new(),
        detector_ownership: Vec::new(),
    }];
    let selected =
        r59_selected_support_from_diagnostics(64, 64, &schema, &oracle, &diagnostics).unwrap();
    let mut page = Page::new("r59-c01", 64, 64);
    page.nodes.insert(
        node_id,
        Node {
            id: node_id,
            transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
            visible: true,
            kind: NodeKind::Text(TextData {
                text: Some("汉".into()),
                detector: Some(SOURCE_GATE_TARGET_DETECTOR.into()),
                line_polygons: Some(vec![[
                    [10.0, 10.0],
                    [30.0, 10.0],
                    [30.0, 30.0],
                    [10.0, 30.0],
                ]]),
                ..Default::default()
            }),
        },
    );
    let downstream = r59_downstream_support_from_scene(&page, &schema, &oracle).unwrap();
    assert_ne!(
        selected["target"].as_slice(),
        downstream["target"].as_raw().as_slice()
    );
    assert_eq!(
        selected["target"]
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>(),
        100
    );
    assert!(
        downstream["target"]
            .as_raw()
            .iter()
            .filter(|value| **value != 0)
            .count()
            > 100
    );
}

fn synthetic_formal_cell(entry: &str, device: &str, passed: bool) -> R59TerminalCellResult {
    let candidate_id = "S25L4";
    R59TerminalCellResult {
        cell_key: format!("{entry}/{device}"),
        result: if passed { "pass" } else { "fail-closed" }.into(),
        selection_result: Some(if passed { "selected" } else { "rejected" }.into()),
        target_recall: R59TargetRecall {
            target_total: 1,
            selected: usize::from(passed),
            covered: usize::from(passed),
            uncovered: usize::from(!passed),
        },
        pp_han_count: 1,
        vl_han_count: 1,
        rejection_reason: (!passed).then(|| "coverage_failure".into()),
        device_evidence_sha256: synthetic_hash(11),
        log_sha256: synthetic_hash(12),
        diagnostic_sha256: synthetic_hash(13),
        target_coverage_index_sha256: Some(synthetic_hash(14)),
        diagnostic_cell_key: format!("holdout/{candidate_id}/{device}/{entry}"),
        phase: "holdout".into(),
        candidate_id: candidate_id.into(),
        entry_id: entry.into(),
        device: device.into(),
        terminal_reason: (!passed).then(|| "coverage_failure".into()),
        diagnostic_path: format!(
            "cells/holdout/{candidate_id}/{device}/{entry}/cell-diagnostic.json"
        ),
        diagnostic_byte_length: 1,
        target_coverage_index_path: Some(format!(
            "cells/holdout/{candidate_id}/{device}/{entry}/target-coverage-index.json"
        )),
        target_coverage_index_byte_length: Some(1),
        device_evidence_path: format!(
            "cells/holdout/{candidate_id}/{device}/{entry}/device-evidence.json"
        ),
        device_evidence_byte_length: 1,
        log_path: format!("cells/holdout/{candidate_id}/{device}/{entry}/inference.log"),
        log_byte_length: 1,
    }
}

fn synthetic_formal_run(cells: Vec<R59TerminalCellResult>) -> R59FormalRunEvidence {
    let first_failed_cell = cells
        .iter()
        .find(|cell| cell.result != "pass")
        .map(|cell| cell.cell_key.clone());
    R59FormalRunEvidence {
        bundle_validation_receipt: Some(PublishedArtifact {
            path: "reports/r59/bundle-validation.json".into(),
            sha256: synthetic_hash(15),
            byte_length: 1,
        }),
        cells,
        first_failed_cell,
    }
}

#[test]
fn r59_publication_is_create_new_mode_0600_and_canonical_without_newline() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);
    let environment = parse_test_environment(&values).unwrap();
    let bytes = canonical_json(&serde_json::json!({"b": 2, "a": 1})).unwrap();
    let published = publish_r59_artifact(&environment, "r59/publication.json", &bytes).unwrap();
    let path = root.join(&published.path);

    assert_eq!(bytes, br#"{"a":1,"b":2}"#);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert!(publish_r59_artifact(&environment, "r59/publication.json", &bytes).is_err());
    assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

fn calibration_evidence() -> RunnerEvidence {
    RunnerEvidence {
        selected_candidate_id: "S25L4".into(),
        process_evidence: ["cpu", "metal"]
            .map(|device| synthetic_process("calibration", device))
            .into(),
        results: synthetic_entry_ids("calibration")
            .iter()
            .flat_map(|entry_id| {
                ["cpu", "metal"].into_iter().flat_map(move |device| {
                    candidates_schema().into_iter().map(move |candidate| {
                        synthetic_result("calibration", entry_id, device, &candidate.id)
                    })
                })
            })
            .collect(),
        formal: None,
    }
}

fn holdout_evidence() -> RunnerEvidence {
    RunnerEvidence {
        selected_candidate_id: "S25L4".into(),
        process_evidence: ["cpu", "metal"]
            .map(|device| synthetic_process("holdout", device))
            .into(),
        results: synthetic_entry_ids("holdout")
            .iter()
            .flat_map(|entry_id| {
                ["cpu", "metal"]
                    .into_iter()
                    .map(move |device| synthetic_result("holdout", entry_id, device, "S25L4"))
            })
            .collect(),
        formal: None,
    }
}

#[test]
fn source_gate_selection_preflight_fails_before_model_runner() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let model_calls = std::cell::Cell::new(0);
    let run = |values: &HashMap<&'static str, String>,
               head: io::Result<String>,
               fixture: io::Result<()>| {
        run_with(
            |name| values.get(name).cloned(),
            &test_repository(),
            |_| head,
            |_| fixture,
            |_| {
                model_calls.set(model_calls.get() + 1);
                Ok(calibration_evidence())
            },
        )
    };

    let mut missing = valid_environment(&root);
    missing.remove(PHASE_ENV);
    assert!(run(&missing, Ok("a".repeat(40)), Ok(())).is_err());

    let valid = valid_environment(&root);
    let mut missing_check = valid.clone();
    missing_check.remove(REQUIRED_CHECK_ENV);
    assert!(run(&missing_check, Ok("a".repeat(40)), Ok(())).is_err());
    let mut invalid = valid.clone();
    invalid.insert(PHASE_ENV, "selection".into());
    assert!(run(&invalid, Ok("a".repeat(40)), Ok(())).is_err());
    invalid.insert(PHASE_ENV, "calibration-freeze".into());
    invalid.insert(B0_SHA_ENV, "A".repeat(40));
    assert!(run(&invalid, Ok("a".repeat(40)), Ok(())).is_err());
    assert!(run(&valid, Ok("b".repeat(40)), Ok(())).is_err());

    fs::write(root.join("selection.json"), b"frozen").unwrap();
    assert!(run(&valid, Ok("a".repeat(40)), Ok(())).is_err());
    fs::remove_file(root.join("selection.json")).unwrap();

    let mut holdout = valid.clone();
    holdout.insert(PHASE_ENV, "holdout".into());
    assert!(run(&holdout, Ok("a".repeat(40)), Ok(())).is_err());
    fs::create_dir(root.join("selection.json")).unwrap();
    assert!(run(&holdout, Ok("a".repeat(40)), Ok(())).is_err());
    fs::remove_dir(root.join("selection.json")).unwrap();

    assert!(
        run(
            &valid,
            Ok("a".repeat(40)),
            Err(io::Error::other("fixed fixture is dirty")),
        )
        .is_err()
    );
    assert_eq!(model_calls.get(), 0);

    let result = run_with(
        |name| valid.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| {
            model_calls.set(model_calls.get() + 1);
            Err(io::Error::other(
                "Source Gate model runner is not implemented",
            ))
        },
    );
    assert!(result.is_err());
    assert_eq!(model_calls.get(), 1);
    assert!(!root.join("selection.json").exists());
}

#[test]
fn source_gate_selection_calibration_writes_synced_canonical_pre_holdout_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);

    run_with(
        |name| values.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| Ok(calibration_evidence()),
    )
    .unwrap();

    let bytes = fs::read(root.join("selection.json")).unwrap();
    let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bytes, canonical_json(&artifact).unwrap());
    assert_ne!(bytes.last(), Some(&b'\n'));
    assert_eq!(artifact.process_evidence.len(), 2);
    assert_eq!(artifact.calibration_results.len(), 32);
    assert_eq!(artifact.required_checks.len(), 1);
    assert_eq!(artifact.holdout_entry_ids, r59_entry_ids('h'));
    assert_eq!(artifact.enabled_cargo_features, ["hanonly-test-evidence"]);
    assert_eq!(
        fs::metadata(root.join("selection.json")).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(
        artifact.frozen_recall_contract,
        frozen_recall_contract(&artifact.selected_candidate_id)
    );
    assert!(artifact.holdout_results.is_empty());
    assert!(artifact.holdout_completed_at_utc.is_none());
}

#[test]
fn source_gate_selection_holdout_builds_closed_final_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let mut values = valid_environment(&root);
    run_with(
        |name| values.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| Ok(calibration_evidence()),
    )
    .unwrap();
    let calibration_bytes = fs::read(root.join("selection.json")).unwrap();

    values.insert(PHASE_ENV, "holdout".into());
    set_required_check(&mut values, &root, Phase::Holdout);
    run_with(
        |name| values.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| Ok(holdout_evidence()),
    )
    .unwrap();

    assert_eq!(
        fs::read(root.join("selection.json")).unwrap(),
        calibration_bytes
    );
    let bytes = fs::read(root.join("selection.json.holdout.json")).unwrap();
    let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bytes, canonical_json(&artifact).unwrap());
    assert_eq!(artifact.process_evidence.len(), 4);
    assert_eq!(artifact.calibration_results.len(), 32);
    assert_eq!(artifact.holdout_results.len(), 8);
    assert_eq!(artifact.required_checks.len(), 2);
    assert_eq!(artifact.holdout_entry_ids, r59_entry_ids('h'));
    assert_eq!(artifact.enabled_cargo_features, ["hanonly-test-evidence"]);
    assert!(!artifact.retuned_after_freeze);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from_iter([
            "version".into(),
            "plan_revision".into(),
            "b0_sha".into(),
            "manifest_sha256".into(),
            "holdout_manifest_sha256".into(),
            "source_gate_fixture_manifest_sha256".into(),
            "image_input_contract_sha256".into(),
            "source_color_contract_sha256".into(),
            "color_constant_set_sha256".into(),
            "requested_devices".into(),
            "enabled_cargo_features".into(),
            "backend_evidence_parser_version".into(),
            "required_checks".into(),
            "frozen_recall_contract".into(),
            "candidates".into(),
            "calibration_entry_ids".into(),
            "holdout_entry_ids".into(),
            "process_evidence".into(),
            "calibration_results".into(),
            "selected_candidate_id".into(),
            "frozen_at_utc".into(),
            "frozen_payload_sha256".into(),
            "holdout_results".into(),
            "holdout_completed_at_utc".into(),
            "retuned_after_freeze".into(),
        ])
    );
}

#[test]
fn source_gate_selection_rejects_invalid_candidate_missing_cell_and_device_evidence() {
    for evidence in [
        RunnerEvidence {
            selected_candidate_id: "R100".into(),
            ..calibration_evidence()
        },
        {
            let mut evidence = calibration_evidence();
            evidence.results.pop();
            evidence
        },
        {
            let mut evidence = calibration_evidence();
            evidence.process_evidence[0]
                .load_evidence
                .loaded_model_devices
                .clear();
            evidence
        },
        {
            let mut evidence = calibration_evidence();
            evidence.results[0].execution_evidence.paddle_instance_id = "9".repeat(32);
            evidence
        },
        {
            let mut evidence = calibration_evidence();
            evidence.process_evidence[1].load_evidence.n_gpu_layers = 32;
            evidence
        },
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);
        assert!(
            run_with(
                |name| values.get(name).cloned(),
                &test_repository(),
                |_| Ok("a".repeat(40)),
                |_| Ok(()),
                |_| Ok(evidence),
            )
            .is_err()
        );
        assert!(!root.join("selection.json").exists());
    }
}

#[test]
fn source_gate_selection_writes_calibration_diagnostic_when_no_candidate_passes() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);
    let mut evidence = calibration_evidence();
    for result in &mut evidence.results {
        result.derived.passed = false;
    }

    let error = run_with(
        |name| values.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| Ok(evidence),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("no all-pass Source Gate crop candidate")
    );
    assert!(!root.join("selection.json").exists());
    let bytes = fs::read(root.join("calibration-diagnostic.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bytes, canonical_json(&value).unwrap());
    assert_eq!(
        value["schema"],
        "hanonly-source-gate-calibration-diagnostic-v1"
    );
    assert_eq!(value["calibration_results"].as_array().unwrap().len(), 32);
    assert!(
        value["failure"]
            .as_str()
            .unwrap()
            .contains("no all-pass Source Gate crop candidate")
    );
}

#[test]
fn source_gate_selection_rejects_frozen_projection_hash_drift() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);
    run_with(
        |name| values.get(name).cloned(),
        &test_repository(),
        |_| Ok("a".repeat(40)),
        |_| Ok(()),
        |_| Ok(calibration_evidence()),
    )
    .unwrap();

    let bytes = fs::read(root.join("selection.json")).unwrap();
    let mut artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
    artifact.selected_candidate_id = "S25L6".into();
    assert!(
        validate_artifact(&artifact, Phase::CalibrationFreeze, &{
            parse_test_environment(&values).unwrap()
        })
        .is_err()
    );
}

#[test]
fn source_gate_selection_rejects_root_and_escaping_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    for artifact in [root.clone(), root.join("../escape")] {
        let mut values = valid_environment(&root);
        values.insert(ARTIFACT_ENV, artifact.to_string_lossy().into_owned());
        assert!(parse_test_environment(&values).is_err());
    }
}

#[test]
fn hanonly_test_evidence_bridge_reachable() {
    let accessor: for<'a> fn(
        &'a koharu_ml::aot_inpainting::AotInpainting,
    ) -> &'a koharu_ml::Device = koharu_ml::aot_inpainting::AotInpainting::device;
    let _ = accessor;
}

#[test]
fn r52_bridge_request_schema_is_closed() {
    let request_value = serde_json::json!({
        "contract": "hanonly-r52-evidence-bridge-request-v1",
        "plan_revision": 52,
        "mode": "challenge",
        "b0_sha": "a".repeat(40),
        "repo_root": "/repo",
        "evidence_root": "/evidence",
        "result_path": "/evidence/.r52-challenge-result-1.tmp",
        "selected_candidate_id": "S25L4",
        "challenge_manifest_path": "/challenge/manifest.json",
        "challenge_manifest_sha256": R52_CHALLENGE_MANIFEST_SHA256,
        "challenge_hash_record_path": "/challenge/hashes.json",
        "challenge_hash_record_sha256": R52_CHALLENGE_HASHES_SHA256,
        "r49_visual_manifest_path": R49_VISUAL_MANIFEST,
        "r49_visual_manifest_sha256": R49_VISUAL_MANIFEST_SHA256,
        "source_gate_fixture_manifest_sha256": "b".repeat(64),
        "calibration_selection_artifact_path": "/evidence/selection.json",
        "b0_preflight_attestation_path": "/evidence/preflight.json",
    });
    let request: R52BridgeRequest =
        serde_json::from_value(request_value.clone()).expect("closed R52 request");
    let temp = tempfile::tempdir().expect("R52 request temp");
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
        .expect("secure R52 request parent");
    let path = temp.path().join("request.json");
    fs::write(
        &path,
        canonical_json(&request).expect("canonical R52 request"),
    )
    .expect("write R52 request");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("secure R52 request");
    assert!(load_r52_bridge_request_path(fs::canonicalize(&path).unwrap()).is_ok());
    let mut unknown = request_value;
    unknown["operator"] = serde_json::json!("forbidden");
    assert!(serde_json::from_value::<R52BridgeRequest>(unknown).is_err());
}

#[test]
fn r52_bridge_applies_only_exact_protected_latin_correction() {
    let (mut schema, mut oracle) = r59_test_schema_and_oracle();
    schema.id = "r49-h04".into();
    schema.protected_rois.clear();
    oracle.protected_rois.clear();
    schema.targets.push(VisualManifestTarget {
        id: "product-id".into(),
        source_roi: [50, 0, 64, 64],
        clean_reference_edit_roi: [50, 0, 64, 64],
        erase_source_ink_mask_path: "product-id-erase.bin".into(),
        erase_source_ink_mask_sha256: synthetic_hash(46),
        residual_source_ink_mask_path: "product-id-residual.bin".into(),
        residual_source_ink_mask_sha256: synthetic_hash(47),
        position: Position::Interior,
        writing: Writing::Horizontal,
        effect: Effect::Plain,
        translation_length: TranslationLength::Equal,
        expected: Expected::AutomaticStrict,
    });
    oracle.targets.push(OracleValidatedTarget {
        source_roi: ValidatedHalfOpenRect {
            left: 50,
            top: 0,
            right: 64,
            bottom: 64,
        },
        edit_roi: ValidatedHalfOpenRect {
            left: 50,
            top: 0,
            right: 64,
            bottom: 64,
        },
        delta_mask: vec![1; 14 * 64].into_boxed_slice(),
    });
    apply_r52_protected_latin_correction(&mut schema, &mut oracle)
        .expect("apply protected Latin correction");
    assert!(
        schema
            .targets
            .iter()
            .all(|target| target.id != "product-id")
    );
    assert_eq!(schema.targets.len(), oracle.targets.len());

    let mut result = synthetic_result("holdout", "r49-h04", "cpu", "S25L4");
    result.derived.passed = false;
    result.derived.source_coverage_preflight.rejected_after_vl = true;
    result
        .derived
        .source_coverage_preflight
        .pp_vl_complete_coverage = false;
    result
        .derived
        .source_coverage_preflight
        .source_removal_preflight_passed = false;
    result.derived.protected_false_positive_count = 0;
    assert!(r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_protected_latin"),
        "regression"
    ));
    result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids
        .clear();
    assert!(!r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_protected_latin"),
        "regression"
    ));
    result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids = vec!["target".into()];
    assert!(!r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_unprotected"),
        "regression"
    ));
    result.entry_id = "r49-h03".into();
    assert!(!r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_protected_latin"),
        "regression"
    ));
}

#[test]
#[ignore = "requires a canonical R52 bridge request and installed Source Gate models"]
fn han_only_r52_evidence_bridge() {
    run_r52_evidence_bridge().expect("R52 evidence bridge failed");
}

#[test]
#[ignore = "requires frozen B0 selection environment and installed Source Gate models"]
fn han_only_source_ink_erase_stage_probe() {
    let environment = SelectionEnvironment::parse(|name| std::env::var(name).ok())
        .expect("erase-stage probe environment");
    assert!(!environment.artifact.exists());
    let evidence = run_erase_stage_probe(&environment).expect("erase-stage probe failed");
    assert_eq!(evidence.selected_candidate_id, "S25L4");
    assert_eq!(evidence.results.len(), 8);
    assert!(evidence.results.iter().all(|result| {
        environment.calibration_entry_ids.contains(&result.entry_id)
            && result.candidate_id == "S25L4"
    }));
    assert!(!environment.artifact.exists());
}

#[test]
#[ignore = "requires frozen B0 selection environment and installed Source Gate models"]
fn han_only_source_gate_crop_selection_matrix() {
    let repository = repository_root().expect("repository root");
    run_with(
        |name| std::env::var(name).ok(),
        &repository,
        git_head,
        require_fixture_clean,
        run_real_model,
    )
    .expect("Source Gate selection harness failed");
}
