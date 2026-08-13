// SPDX-License-Identifier: Apache-2.0
//! Terminal output. Hand-rolled ANSI rather than a colour crate — this is the
//! entire surface we need, and it keeps the dependency list at four crates.

use crate::client::Endpoint;
use crate::probes::Depth;
use crate::report::{Group, ProbeResult, Report, Status};
use crate::util::pad_display;

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const BLUE: &str = "\x1b[36m";
const GREY: &str = "\x1b[90m";

fn paint(s: &str, colour: &str, on: bool) -> String {
    if on {
        format!("{colour}{s}{RESET}")
    } else {
        s.to_string()
    }
}

fn status_colour(s: Status) -> &'static str {
    match s {
        Status::Pass => GREEN,
        Status::Warn => YELLOW,
        Status::Fail => RED,
        Status::Skip => GREY,
        Status::Error => RED,
    }
}

pub fn banner(ep: &Endpoint, depth: Depth, claimed: &str, colour: bool) {
    println!();
    println!(
        "{}",
        paint(
            &format!("llm-verify {}", env!("CARGO_PKG_VERSION")),
            BOLD,
            colour
        )
    );
    println!("  端点   : {}", ep.base_url);
    println!("  模型   : {}", ep.model);
    if claimed != ep.model {
        println!("  宣称   : {claimed}");
    }
    println!("  协议   : {}", ep.protocol);
    println!("  深度   : {}", depth.as_str());
    println!("{}", paint(&"─".repeat(64), DIM, colour));
}

pub fn progress_line(r: &ProbeResult, i: usize, total: usize, colour: bool) {
    let sym = paint(r.status.symbol(), status_colour(r.status), colour);
    let idx = paint(&format!("[{i:>2}/{total}]"), DIM, colour);
    let summary = crate::util::truncate(&r.summary, 46);
    println!(
        "  {idx} {sym} {} {}",
        pad_display(&r.label, 20),
        paint(&summary, GREY, colour)
    );
}

pub fn summary(rep: &Report, colour: bool) {
    let v = &rep.verdict;
    println!("{}", paint(&"─".repeat(64), DIM, colour));

    let verdict_colour = match v.authenticity {
        crate::report::Authenticity::Authentic => GREEN,
        crate::report::Authenticity::AuthenticDegraded
        | crate::report::Authenticity::ThirdParty => BLUE,
        crate::report::Authenticity::Suspicious => YELLOW,
        crate::report::Authenticity::Counterfeit => RED,
        crate::report::Authenticity::Inconclusive => GREY,
    };

    println!(
        "  判定     : {}   来源 : {}",
        paint(
            &format!("{} ", v.authenticity.label_zh()),
            &format!("{BOLD}{verdict_colour}"),
            colour
        ),
        v.channel.label_zh()
    );
    println!(
        "  评分     : {:.1} / 100     置信度 : {:.0}%",
        v.score,
        v.confidence * 100.0
    );
    println!("  身份     : {}", rep.identity.status.label_zh());
    if rep.billing.honest_input > 0 {
        println!(
            "  计费倍率 : {:.2}×  （计费 {} / 实际 {} token，{}）",
            rep.billing.input_ratio,
            rep.billing.billed_input,
            rep.billing.honest_input,
            rep.billing.method
        );
    }
    if rep.perf.samples > 0 && rep.perf.ttft_p50 > 0.0 {
        println!(
            "  性能     : TTFT P50 {:.0}ms   吞吐 {:.1} tok/s",
            rep.perf.ttft_p50, rep.perf.tps_mean
        );
    }

    println!(
        "  探针     : {} 通过  {} 警告  {} 失败  {} 跳过  {} 错误",
        paint(&rep.count(Status::Pass).to_string(), GREEN, colour),
        paint(&rep.count(Status::Warn).to_string(), YELLOW, colour),
        paint(&rep.count(Status::Fail).to_string(), RED, colour),
        paint(&rep.count(Status::Skip).to_string(), GREY, colour),
        paint(&rep.count(Status::Error).to_string(), RED, colour),
    );
    println!(
        "  请求数   : {}   耗时 {:.1}s",
        rep.request_count,
        rep.duration_ms as f64 / 1000.0
    );

    if !v.hard_gate_hits.is_empty() {
        println!();
        println!(
            "  {}",
            paint(
                &format!("硬门禁命中 {} 项", v.hard_gate_hits.len()),
                &format!("{BOLD}{RED}"),
                colour
            )
        );
        for g in &v.hard_gate_hits {
            println!("    {} {} — {}", paint("!", RED, colour), g.name, g.reason);
        }
    }

    // Group breakdown gives a quick read of *where* the problems are.
    println!();
    for g in Group::ALL {
        if let Some(s) = v.group_scores.get(g.key()) {
            let bar_len = (*s / 100.0 * 20.0).round() as usize;
            let bar_colour = if *s >= 85.0 {
                GREEN
            } else if *s >= 60.0 {
                YELLOW
            } else {
                RED
            };
            println!(
                "  {} {} {:>5.1}",
                pad_display(g.label_zh(), 14),
                paint(
                    &format!("{}{}", "█".repeat(bar_len), "·".repeat(20 - bar_len)),
                    bar_colour,
                    colour
                ),
                s
            );
        }
    }

    if !rep.skipped.is_empty() {
        println!();
        println!(
            "  {} {} 项探针未执行（未测 ≠ 通过）",
            paint("–", GREY, colour),
            rep.skipped.len()
        );
    }
    println!("{}", paint(&"─".repeat(64), DIM, colour));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_is_a_noop_when_colour_is_off() {
        assert_eq!(paint("x", RED, false), "x");
        assert!(paint("x", RED, true).contains("\x1b[31m"));
    }
}
