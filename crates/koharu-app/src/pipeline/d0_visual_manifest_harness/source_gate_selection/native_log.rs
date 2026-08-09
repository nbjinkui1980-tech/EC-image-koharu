use std::collections::BTreeMap;
use std::io;
use std::str;

use super::super::{invalid_data, require};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ParsedLoadLog {
    pub(super) offloaded_layers: u32,
    pub(super) offloadable_layers: u32,
    pub(super) model_buffer_bytes_by_backend: BTreeMap<String, u64>,
    pub(super) mtmd_backend: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ParsedInferenceLog {
    pub(super) context_buffer_bytes_by_backend: BTreeMap<String, u64>,
    pub(super) compute_buffer_bytes_by_backend: BTreeMap<String, u64>,
}

pub(super) fn parse_native_load_log(bytes: &[u8]) -> io::Result<ParsedLoadLog> {
    let text = std::str::from_utf8(bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let mut offloaded = None;
    let mut model_buffers = BTreeMap::new();
    let mut mtmd_backend = None;
    for line in text.lines() {
        if let Some((loaded, total)) = parse_offloaded_layers(line) {
            offloaded = Some((loaded, total));
        }
        accumulate_buffer_line(line, "model buffer size", &mut model_buffers)?;
        if line.contains("CLIP using") && line.contains("backend") {
            mtmd_backend = canonical_backend(line).map(str::to_owned);
        }
    }
    let (offloaded_layers, offloadable_layers) =
        offloaded.ok_or_else(|| invalid_data("native load log omitted offloaded layers"))?;
    require(
        !model_buffers.is_empty(),
        "native load log omitted model buffers",
    )?;
    Ok(ParsedLoadLog {
        offloaded_layers,
        offloadable_layers,
        model_buffer_bytes_by_backend: model_buffers,
        mtmd_backend: mtmd_backend
            .ok_or_else(|| invalid_data("native load log omitted MTMD backend"))?,
    })
}

pub(super) fn parse_native_inference_log(bytes: &[u8]) -> io::Result<ParsedInferenceLog> {
    let text = std::str::from_utf8(bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let mut context_buffers = BTreeMap::new();
    let mut compute_buffers = BTreeMap::new();
    for line in text.lines() {
        accumulate_buffer_line(line, "output buffer size", &mut context_buffers)?;
        accumulate_buffer_line(line, "KV buffer size", &mut context_buffers)?;
        accumulate_buffer_line(line, "compute buffer size", &mut compute_buffers)?;
    }
    require(
        !context_buffers.is_empty() && !compute_buffers.is_empty(),
        "native inference log omitted context or compute buffers",
    )?;
    Ok(ParsedInferenceLog {
        context_buffer_bytes_by_backend: context_buffers,
        compute_buffer_bytes_by_backend: compute_buffers,
    })
}

fn parse_offloaded_layers(line: &str) -> Option<(u32, u32)> {
    let suffix = line.split_once("offloaded ")?.1;
    let ratio = suffix.split_whitespace().next()?;
    let (loaded, total) = ratio.split_once('/')?;
    Some((loaded.parse().ok()?, total.parse().ok()?))
}

fn accumulate_buffer_line(
    line: &str,
    marker: &str,
    buffers: &mut BTreeMap<String, u64>,
) -> io::Result<()> {
    if !line.contains(marker) {
        return Ok(());
    }
    let Some(backend) = canonical_backend(line) else {
        return Err(invalid_data("native buffer log used an unknown backend"));
    };
    let value = line
        .split_once('=')
        .and_then(|(_, suffix)| suffix.split_whitespace().next())
        .map(str::parse::<f64>)
        .transpose()
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let Some(value) = value else {
        return Ok(());
    };
    require(
        value.is_finite() && value >= 0.0,
        "native buffer size is invalid",
    )?;
    let bytes = (value * 1024.0 * 1024.0).round() as u64;
    *buffers.entry(backend.into()).or_default() += bytes;
    Ok(())
}

fn canonical_backend(line: &str) -> Option<&'static str> {
    if line.contains("MTL") || line.contains("Metal") {
        Some("Metal")
    } else if line.contains("CPU") {
        Some("CPU")
    } else {
        None
    }
}
