// SPDX-License-Identifier: Apache-2.0
//! Performance probes.
//!
//! Two audiences. A buyer wants to know whether the endpoint is fast enough.
//! The verdict layer wants throughput as *identity evidence*: a model that
//! claims to be a flagship but streams at small-model speed is a downgrade
//! signal that no amount of prompt engineering can hide.

use super::{Ctx, PerfSample};
use crate::protocol::ChatRequest;
use crate::report::{Group, ProbeResult};
use crate::util::{estimate_tokens, mean, percentile, stddev};

const G: Group = Group::Perf;

/// Throughput bands, tokens/sec. Calibrated against the flagship-vs-small
/// split that holds across vendors rather than any one model's number.
const TPS_LARGE_CEILING: f64 = 60.0;
const TPS_SMALL_FLOOR: f64 = 90.0;

/// Coefficient of variation above which latency is unstable enough to suggest
/// the endpoint is fanning out across more than one backend.
const CV_UNSTABLE: f64 = 0.5;

pub async fn run(ctx: &Ctx) -> Vec<ProbeResult> {
    sample(ctx).await;
    let samples = ctx.perf.borrow().clone();
    vec![
        ttft(&samples),
        latency(&samples),
        throughput(&samples),
        jitter(&samples),
    ]
}

/// A fixed-length generation so throughput numbers are comparable run to run.
async fn sample(ctx: &Ctx) {
    let n = ctx.depth.repeats();
    for i in 0..n {
        let req = ChatRequest::new(
            &ctx.client.endpoint.model,
            "Write a single paragraph of exactly 80 words about the ocean. \
             Plain prose, no lists, no preamble.",
        )
        .max_tokens(200)
        .temperature(0.0);

        match ctx.client.stream(&req).await {
            Ok(s) => {
                let out = s
                    .usage
                    .as_ref()
                    .map(|u| u.output_tokens)
                    .filter(|t| *t > 0)
                    .unwrap_or_else(|| estimate_tokens(&s.text));
                ctx.add_perf(PerfSample {
                    probe: format!("perf_{}", i + 1),
                    ttft_ms: s.ttft_ms,
                    latency_ms: s.total_ms,
                    output_tokens: out,
                });
            }
            Err(_) => {
                // A failed sample is simply absent; the probes below report
                // reduced sample counts rather than pretending to a number.
            }
        }
    }
}

fn ttft_values(samples: &[PerfSample]) -> Vec<f64> {
    let mut v: Vec<f64> = samples
        .iter()
        .filter_map(|s| s.ttft_ms)
        .map(|t| t as f64)
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn latency_values(samples: &[PerfSample]) -> Vec<f64> {
    let mut v: Vec<f64> = samples.iter().map(|s| s.latency_ms as f64).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn tps_values(samples: &[PerfSample]) -> Vec<f64> {
    let mut v: Vec<f64> = samples.iter().filter_map(|s| s.tps()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn ttft(samples: &[PerfSample]) -> ProbeResult {
    let p = ProbeResult::new("ttft", "首字延迟 TTFT", G).weight(2);
    let v = ttft_values(samples);
    if v.is_empty() {
        return p.skip("没有可用的流式样本");
    }
    let p50 = percentile(&v, 50.0);
    let p95 = percentile(&v, 95.0);
    let p = p
        .metric("samples", v.len())
        .metric("p50_ms", p50.round())
        .metric("p95_ms", p95.round())
        .metric("min_ms", v[0].round())
        .metric("max_ms", v[v.len() - 1].round());

    if p50 <= 1000.0 {
        p.pass(format!("P50 {:.0}ms，响应迅速", p50))
    } else if p50 <= 3000.0 {
        p.warn(format!("P50 {:.0}ms，偏慢", p50))
    } else {
        p.fail(format!("P50 {:.0}ms，明显迟滞", p50))
            .finding("首字延迟超过 3 秒，用户会明显感到卡顿")
    }
}

fn latency(samples: &[PerfSample]) -> ProbeResult {
    let p = ProbeResult::new("latency", "端到端延迟", G).weight(1);
    let v = latency_values(samples);
    if v.is_empty() {
        return p.skip("没有可用的样本");
    }
    let p50 = percentile(&v, 50.0);
    let p95 = percentile(&v, 95.0);
    // Naming the contributing probes makes an outlier traceable back to the
    // request that produced it.
    let sources: Vec<&str> = samples.iter().map(|s| s.probe.as_str()).collect();
    ProbeResult::pass(
        p.metric("samples", v.len())
            .metric("p50_ms", p50.round())
            .metric("p95_ms", p95.round())
            .metric("sources", sources.join(", ")),
        format!("P50 {:.0}ms / P95 {:.0}ms", p50, p95),
    )
}

fn throughput(samples: &[PerfSample]) -> ProbeResult {
    let p = ProbeResult::new("tps", "生成吞吐", G).weight(2).neutral();
    let v = tps_values(samples);
    if v.is_empty() {
        return p.skip("没有足够的流式样本计算吞吐");
    }
    let avg = mean(&v);
    let band = tier_band(avg);
    let p = p
        .metric("samples", v.len())
        .metric("mean_tps", (avg * 10.0).round() / 10.0)
        .metric("p50_tps", (percentile(&v, 50.0) * 10.0).round() / 10.0)
        .metric("speed_band", band);

    p.pass(format!(
        "平均 {:.1} tok/s（{}）",
        avg,
        match band {
            "large" => "偏大模型速度",
            "small" => "偏小模型速度",
            _ => "中间档",
        }
    ))
    .finding("吞吐本身也是身份证据：声称旗舰却跑出小模型的速度，是降档的旁证".to_string())
}

/// Which size class this throughput is consistent with.
pub fn tier_band(tps: f64) -> &'static str {
    if tps <= 0.0 {
        "unknown"
    } else if tps < TPS_LARGE_CEILING {
        "large"
    } else if tps >= TPS_SMALL_FLOOR {
        "small"
    } else {
        "mid"
    }
}

fn jitter(samples: &[PerfSample]) -> ProbeResult {
    let p = ProbeResult::new("jitter", "延迟抖动", G).weight(2);
    let v = latency_values(samples);
    if v.len() < 3 {
        return p.skip(format!("样本只有 {} 个，不足以判断抖动", v.len()));
    }
    let m = mean(&v);
    let sd = stddev(&v);
    let cv = if m > 0.0 { sd / m } else { 0.0 };
    let p = p
        .metric("samples", v.len())
        .metric("mean_ms", m.round())
        .metric("stddev_ms", sd.round())
        .metric("cv", (cv * 1000.0).round() / 1000.0);

    if cv < 0.25 {
        p.pass(format!("变异系数 {cv:.2}，延迟稳定"))
    } else if cv < CV_UNSTABLE {
        p.warn(format!("变异系数 {cv:.2}，延迟有些波动"))
    } else {
        p.fail(format!("变异系数 {cv:.2}，延迟高度不稳定"))
            .finding("同一端点的耗时分布分散，常见于后端轮询多个供应商或严重超卖")
    }
}

/// Everything the report's perf panel needs, computed once.
pub fn summarize(samples: &[PerfSample]) -> crate::report::PerfSummary {
    let ttft = ttft_values(samples);
    let lat = latency_values(samples);
    let tps = tps_values(samples);
    let m = mean(&lat);
    crate::report::PerfSummary {
        samples: samples.len(),
        ttft_p50: percentile(&ttft, 50.0),
        ttft_p95: percentile(&ttft, 95.0),
        latency_p50: percentile(&lat, 50.0),
        latency_p95: percentile(&lat, 95.0),
        tps_mean: mean(&tps),
        latency_cv: if m > 0.0 { stddev(&lat) / m } else { 0.0 },
        ttft_ms: ttft,
        latency_ms: lat,
        tps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Status;

    fn s(ttft: Option<u64>, lat: u64, out: u32) -> PerfSample {
        PerfSample {
            probe: "t".into(),
            ttft_ms: ttft,
            latency_ms: lat,
            output_tokens: out,
        }
    }

    #[test]
    fn tier_band_splits_large_from_small() {
        assert_eq!(tier_band(45.0), "large");
        assert_eq!(tier_band(120.0), "small");
        assert_eq!(tier_band(75.0), "mid");
        assert_eq!(tier_band(0.0), "unknown");
        // Boundaries land on the intended side.
        assert_eq!(tier_band(59.9), "large");
        assert_eq!(tier_band(60.0), "mid");
        assert_eq!(tier_band(90.0), "small");
    }

    #[test]
    fn probes_skip_rather_than_report_zero_when_starved() {
        assert_eq!(ttft(&[]).status, Status::Skip);
        assert_eq!(latency(&[]).status, Status::Skip);
        assert_eq!(throughput(&[]).status, Status::Skip);
        // Jitter needs three samples before a spread means anything.
        assert_eq!(
            jitter(&[s(Some(1), 100, 5), s(Some(1), 120, 5)]).status,
            Status::Skip
        );
    }

    #[test]
    fn ttft_bands_track_user_perceived_lag() {
        assert_eq!(ttft(&[s(Some(300), 900, 10)]).status, Status::Pass);
        assert_eq!(ttft(&[s(Some(2000), 4000, 10)]).status, Status::Warn);
        assert_eq!(ttft(&[s(Some(5000), 9000, 10)]).status, Status::Fail);
    }

    #[test]
    fn jitter_flags_a_scattered_distribution() {
        let steady = vec![s(Some(1), 1000, 9), s(Some(1), 1010, 9), s(Some(1), 990, 9)];
        assert_eq!(jitter(&steady).status, Status::Pass);

        // 300ms / 2000ms / 5000ms — the shape of a multi-backend fan-out.
        let scattered = vec![s(Some(1), 300, 9), s(Some(1), 2000, 9), s(Some(1), 5000, 9)];
        let r = jitter(&scattered);
        assert_eq!(r.status, Status::Fail);
        assert!(r.metric_f64("cv").unwrap() > CV_UNSTABLE);
    }

    #[test]
    fn summarize_ignores_samples_that_cannot_yield_tps() {
        let samples = vec![
            s(Some(500), 1500, 100), // tps = 100
            s(None, 2000, 100),      // no TTFT, no tps
        ];
        let sum = summarize(&samples);
        assert_eq!(sum.samples, 2);
        assert_eq!(sum.tps.len(), 1, "only the streamed sample yields tps");
        assert_eq!(sum.ttft_ms.len(), 1);
        assert_eq!(sum.latency_ms.len(), 2);
        assert!((sum.tps_mean - 100.0).abs() < 1e-6);
    }

    #[test]
    fn summarize_on_empty_input_is_all_zero_not_a_panic() {
        let sum = summarize(&[]);
        assert_eq!(sum.samples, 0);
        assert_eq!(sum.latency_p50, 0.0);
        assert_eq!(sum.latency_cv, 0.0);
    }
}
