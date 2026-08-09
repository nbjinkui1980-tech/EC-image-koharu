use std::collections::{BTreeMap, HashSet};
use std::io;

use koharu_llm::safe::{LlamaBackendDeviceType, list_llama_ggml_backend_devices};

use super::{Candidate, EnumeratedDevice, LoadedModelDevice, SelectionResult, candidates_schema};
use super::super::invalid_data;

pub(super) fn enumerated_devices() -> io::Result<Vec<EnumeratedDevice>> {
    list_llama_ggml_backend_devices()
        .into_iter()
        .map(|device| {
            Ok(EnumeratedDevice {
                index: u32::try_from(device.index)
                    .map_err(|_| invalid_data("device index overflow"))?,
                name: device.name,
                description: device.description,
                backend: device.backend,
                device_type: device_type(device.device_type).into(),
            })
        })
        .collect()
}

pub(super) fn loaded_model_devices(
    enumerated: &[EnumeratedDevice],
    buffers: &BTreeMap<String, u64>,
) -> io::Result<Vec<LoadedModelDevice>> {
    buffers
        .iter()
        .filter(|(_, bytes)| **bytes > 0)
        .enumerate()
        .map(|(ordinal, (backend, _))| {
            let device = enumerated
                .iter()
                .find(|device| canonical_device_backend(&device.backend) == Some(backend))
                .ok_or_else(|| invalid_data("loaded backend was not enumerated"))?;
            Ok(LoadedModelDevice {
                model_device_ordinal: ordinal as u32,
                name: if device.name.is_empty() {
                    device.description.clone()
                } else {
                    device.name.clone()
                },
                backend: backend.clone(),
                device_type: device.device_type.clone(),
            })
        })
        .collect()
}

fn canonical_device_backend(backend: &str) -> Option<&'static str> {
    let lower = backend.to_ascii_lowercase();
    if lower.contains("metal") || lower.contains("mtl") {
        Some("Metal")
    } else if lower.contains("cpu") {
        Some("CPU")
    } else {
        None
    }
}

fn device_type(device_type: LlamaBackendDeviceType) -> &'static str {
    match device_type {
        LlamaBackendDeviceType::Cpu => "cpu",
        LlamaBackendDeviceType::Accelerator => "accelerator",
        LlamaBackendDeviceType::Gpu => "gpu",
        LlamaBackendDeviceType::IntegratedGpu => "integrated_gpu",
        LlamaBackendDeviceType::Unknown => "unknown",
    }
}

pub(super) fn select_smallest_all_pass(
    results: &[SelectionResult],
    entry_ids: &[String],
) -> io::Result<String> {
    let mut failures = Vec::new();
    for candidate in candidates_schema() {
        let cells = results
            .iter()
            .filter(|result| result.candidate_id == candidate.id)
            .collect::<Vec<_>>();
        let expected_cells = entry_ids.len() * 2;
        if cells.len() != expected_cells {
            failures.push(format!(
                "{}: incomplete={}/{}",
                candidate.id,
                cells.len(),
                expected_cells
            ));
            continue;
        }
        let failed = cells
            .iter()
            .filter(|result| !result.derived.passed)
            .map(|result| {
                format!(
                    "{}/{} recall={:.3} protected={} unmatched={} rotation_excluded={}",
                    result.entry_id,
                    result
                        .process_evidence_id
                        .rsplit('-')
                        .next()
                        .unwrap_or("unknown"),
                    result.derived.target_recall,
                    result.derived.protected_false_positive_count,
                    result.derived.unmatched_selected_node_ids.len(),
                    result.derived.rotation_targets_excluded
                )
            })
            .collect::<Vec<_>>();
        if !failed.is_empty() {
            failures.push(format!("{}: {}", candidate.id, failed.join(", ")));
            continue;
        }
        let observed = cells
            .iter()
            .map(|result| {
                (
                    result.entry_id.as_str(),
                    result.process_evidence_id.rsplit('-').next().unwrap_or(""),
                )
            })
            .collect::<HashSet<_>>();
        if observed.len() == expected_cells
            && entry_ids.iter().all(|entry_id| {
                ["cpu", "metal"]
                    .iter()
                    .all(|device| observed.contains(&(entry_id.as_str(), *device)))
            })
        {
            return Ok(candidate.id);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "no all-pass Source Gate crop candidate; {}",
            failures.join("; ")
        ),
    ))
}
