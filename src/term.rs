// SPDX-License-Identifier: Apache-2.0
//! Terminal output. Hand-rolled ANSI rather than a colour crate — this is the
//! entire surface we need, and it keeps the dependency list at six crates.
//!
//! Column widths are derived from the language rather than hard-coded, because
//! the same row is roughly twice as wide in English as in Chinese.

use crate::client::Endpoint;
use crate::i18n::Lang;
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

/// Width of the probe-label column. English probe names are longer than their
/// Chinese equivalents, so one hard-coded width would either truncate English
/// or leave a gap in Chinese.
fn label_width(lang: Lang) -> usize {
    match lang {
        Lang::En => 30,
        Lang::Zh => 22,
    }
}

/// Width of the key column in the summary block.
fn key_width(lang: Lang) -> usize {
    match lang {
        Lang::En => 12,
        Lang::Zh => 10,
    }
}

pub fn banner(ep: &Endpoint, depth: Depth, claimed: &str, lang: Lang, colour: bool) {
    let w = key_width(lang) - 2;
    println!();
    println!(
        "{}",
        paint(
            &format!("llm-verify {}", env!("CARGO_PKG_VERSION")),
            BOLD,
            colour
        )
    );
    let row = |k: &str, v: &str| println!("  {} : {v}", pad_display(k, w));
    row(ts!(lang, "Endpoint", "端点"), &ep.base_url);
    row(ts!(lang, "Model", "模型"), &ep.model);
    if claimed != ep.model {
        row(ts!(lang, "Claimed", "宣称"), claimed);
    }
    row(ts!(lang, "Protocol", "协议"), &ep.protocol.to_string());
    row(ts!(lang, "Depth", "深度"), depth.as_str());
    row(ts!(lang, "Language", "语言"), lang.as_str());
    println!("{}", paint(&"─".repeat(72), DIM, colour));
}

pub fn progress_line(r: &ProbeResult, i: usize, total: usize, lang: Lang, colour: bool) {
    let sym = paint(r.status.symbol(), status_colour(r.status), colour);
    let idx = paint(&format!("[{i:>2}/{total}]"), DIM, colour);
    let summary = crate::util::truncate(&r.summary, if lang == Lang::En { 52 } else { 44 });
    println!(
        "  {idx} {sym} {} {}",
        pad_display(&r.label, label_width(lang)),
        paint(&summary, GREY, colour)
    );
}

pub fn summary(rep: &Report, colour: bool) {
    let l = rep.lang;
    let v = &rep.verdict;
    println!("{}", paint(&"─".repeat(72), DIM, colour));

    let verdict_colour = match v.authenticity {
        crate::report::Authenticity::Authentic => GREEN,
        crate::report::Authenticity::AuthenticDegraded
        | crate::report::Authenticity::ThirdParty => BLUE,
        crate::report::Authenticity::Suspicious => YELLOW,
        crate::report::Authenticity::Counterfeit => RED,
        crate::report::Authenticity::Inconclusive => GREY,
    };

    let key = |k: &str| pad_display(k, key_width(l));

    println!(
        "  {} : {}   {} : {}",
        key(ts!(l, "Verdict", "判定")),
        paint(
            v.authenticity.label(l),
            &format!("{BOLD}{verdict_colour}"),
            colour
        ),
        ts!(l, "origin", "来源"),
        v.channel.label(l)
    );
    println!(
        "  {} : {:.1} / 100     {} : {:.0}%",
        key(ts!(l, "Score", "评分")),
        v.score,
        ts!(l, "confidence", "置信度"),
        v.confidence * 100.0
    );
    println!(
        "  {} : {}",
        key(ts!(l, "Identity", "身份")),
        rep.identity.status.label(l)
    );
    if rep.billing.honest_input > 0 {
        println!(
            "  {} : {}",
            key(ts!(l, "Billing", "计费倍率")),
            t!(
                l,
                "{:.2}x  (billed {} / actual {} tokens, via {})",
                "{:.2}×（计费 {} / 实际 {} token，{}）",
                rep.billing.input_ratio,
                rep.billing.billed_input,
                rep.billing.honest_input,
                rep.billing.method
            )
        );
    }
    if rep.perf.samples > 0 && rep.perf.ttft_p50 > 0.0 {
        println!(
            "  {} : {}",
            key(ts!(l, "Performance", "性能")),
            t!(
                l,
                "TTFT P50 {:.0}ms   throughput {:.1} tok/s",
                "TTFT P50 {:.0}ms   吞吐 {:.1} tok/s",
                rep.perf.ttft_p50,
                rep.perf.tps_mean
            )
        );
    }

    println!(
        "  {} : {}",
        key(ts!(l, "Probes", "探针")),
        t!(
            l,
            "{} passed  {} warned  {} failed  {} skipped  {} errored",
            "{} 通过  {} 警告  {} 失败  {} 跳过  {} 错误",
            paint(&rep.count(Status::Pass).to_string(), GREEN, colour),
            paint(&rep.count(Status::Warn).to_string(), YELLOW, colour),
            paint(&rep.count(Status::Fail).to_string(), RED, colour),
            paint(&rep.count(Status::Skip).to_string(), GREY, colour),
            paint(&rep.count(Status::Error).to_string(), RED, colour),
        )
    );
    println!(
        "  {} : {}",
        key(ts!(l, "Requests", "请求数")),
        t!(
            l,
            "{}   elapsed {:.1}s",
            "{}   耗时 {:.1}s",
            rep.request_count,
            rep.duration_ms as f64 / 1000.0
        )
    );

    if !v.hard_gate_hits.is_empty() {
        println!();
        println!(
            "  {}",
            paint(
                &t!(
                    l,
                    "{} hard gate(s) tripped",
                    "硬门禁命中 {} 项",
                    v.hard_gate_hits.len()
                ),
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
                pad_display(g.label(l), label_width(l) - 4),
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
            "  {} {}",
            paint("–", GREY, colour),
            t!(
                l,
                "{} probe(s) did not run — not tested is not the same as passed",
                "{} 项探针未执行（未测 ≠ 通过）",
                rep.skipped.len()
            )
        );
    }
    println!("{}", paint(&"─".repeat(72), DIM, colour));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_is_a_noop_when_colour_is_off() {
        assert_eq!(paint("x", RED, false), "x");
        assert!(paint("x", RED, true).contains("\x1b[31m"));
    }

    #[test]
    fn english_columns_are_wider_than_chinese_ones() {
        // Rendered width, not character count, is what keeps the columns
        // aligned; English needs more of it for the same content.
        assert!(label_width(Lang::En) > label_width(Lang::Zh));
        assert!(key_width(Lang::En) > key_width(Lang::Zh));
        // The group bar borrows from the same budget and must not underflow.
        assert!(label_width(Lang::Zh) > 4);
    }
}
