//! Metrics collection: VRAM, shared memory, tokens/s via llama-server /metrics.

use std::time::Duration;

use crate::backend::pdh_vram;

fn parse_metric_value(body: &str, metric_name: &str) -> Option<f32> {
    body.lines().find_map(|line| {
        if line.starts_with('#') {
            return None;
        }
        let (name, value) = line.split_once(' ')?;
        if name == metric_name {
            return value.trim().parse::<f32>().ok();
        }
        None
    })
}

pub fn dedicated_vram_mb(pdh: &Option<pdh_vram::PdhQuery>) -> Option<f32> {
    #[cfg(target_os = "windows")]
    {
        return pdh.as_ref().and_then(|query| query.collect_dedicated_mb());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = pdh;
        None
    }
}

pub fn shared_memory_mb(pdh: &Option<pdh_vram::PdhQuery>) -> Option<f32> {
    #[cfg(target_os = "windows")]
    {
        return pdh.as_ref().and_then(|query| query.collect_shared_mb());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = pdh;
        None
    }
}

pub fn poll_metrics(
    llama_process_running: bool,
    llama_base_url: &str,
    pdh: &Option<pdh_vram::PdhQuery>,
) -> Option<(Option<f32>, Option<f32>, Option<f32>)> {
    if !llama_process_running {
        return None;
    }

    let vram_mb = dedicated_vram_mb(pdh);
    let shared_mb = shared_memory_mb(pdh);
    let metrics_url = format!("{}/metrics", llama_base_url);
    let tokens_per_second = ureq::get(&metrics_url)
        .timeout(Duration::from_millis(500))
        .call()
        .ok()
        .and_then(|response| response.into_string().ok())
        .and_then(|body| parse_metric_value(&body, "llamacpp:predicted_tokens_seconds"));

    if tokens_per_second.is_none() && vram_mb.is_none() && shared_mb.is_none() {
        return None;
    }

    Some((tokens_per_second, vram_mb, shared_mb))
}
