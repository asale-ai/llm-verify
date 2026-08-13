// SPDX-License-Identifier: Apache-2.0
//! Model identity probes.
//!
//! The question is never "can it answer?" but "does the model behind this
//! endpoint match the one that was sold?". Three independent angles:
//!
//!   surface   — what it says it is. Cheap, and a single system prompt can
//!               forge it, so it never decides anything on its own.
//!   knowledge — what it demonstrably knows. Cross-checked against its own
//!               claimed cutoff, so the check needs no external ground truth
//!               beyond a few well-established dated transitions.
//!   capability— what it can actually do, measured with questions generated
//!               fresh each run so no provider can pre-cache the answers.
//!
//! Capability is the one that catches a silent downgrade, because it is the
//! one the seller cannot fake without actually serving the better model.

use super::Ctx;
use crate::i18n::Lang;
use crate::protocol::ChatRequest;
use crate::report::{Group, ProbeResult};
use crate::util::{now_ms, Rng};
use std::collections::BTreeMap;

const G: Group = Group::Identity;

pub async fn run(ctx: &Ctx) -> Vec<ProbeResult> {
    let mut out = Vec::with_capacity(8);
    out.push(self_id(ctx).await);
    out.push(meta_creator(ctx).await);
    out.push(context_claim(ctx).await);
    out.push(cutoff_claim(ctx).await);
    out.push(world_knowledge(ctx).await);
    let battery = capability_battery(ctx).await;
    out.push(battery.0);
    out.push(verbosity(ctx).await);
    out.push(tier_estimate(ctx, &battery.1));
    out
}

// ── family recognition ─────────────────────────────────────────────────────

/// Keyword → family. Ordered longest-first at match time so `gpt-4` does not
/// shadow a more specific vendor string.
const FAMILY_MARKERS: &[(&str, &str)] = &[
    ("claude", "anthropic"),
    ("anthropic", "anthropic"),
    ("chatgpt", "openai"),
    ("openai", "openai"),
    ("gpt-", "openai"),
    ("gemini", "google"),
    ("google deepmind", "google"),
    ("deepmind", "google"),
    ("llama", "meta"),
    ("meta ai", "meta"),
    ("mistral", "mistral"),
    ("deepseek", "deepseek"),
    ("qwen", "alibaba"),
    ("tongyi", "alibaba"),
    ("通义", "alibaba"),
    ("阿里", "alibaba"),
    ("glm", "zhipu"),
    ("chatglm", "zhipu"),
    ("智谱", "zhipu"),
    ("zhipu", "zhipu"),
    ("kimi", "moonshot"),
    ("moonshot", "moonshot"),
    ("月之暗面", "moonshot"),
    ("grok", "xai"),
    ("xai", "xai"),
    ("ernie", "baidu"),
    ("文心", "baidu"),
];

/// The family a piece of text most strongly identifies with, and how many
/// distinct markers backed that call.
pub fn detect_family(text: &str) -> Option<(String, usize)> {
    let t = text.to_ascii_lowercase();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (needle, family) in FAMILY_MARKERS {
        if t.contains(needle) {
            *counts.entry(family).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(f, n)| (f.to_string(), n))
}

/// The family implied by a model ID, used as the *claim* side of the check.
pub fn family_from_model_id(model: &str) -> Option<String> {
    detect_family(model).map(|(f, _)| f)
}

// ── tier ───────────────────────────────────────────────────────────────────

/// Capability class. Deliberately vendor-neutral: the same three bands
/// describe Opus/Sonnet/Haiku, GPT-4o/4.1-mini/nano, Pro/Flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Large,
    Mid,
    Small,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Large => "large",
            Self::Mid => "mid",
            Self::Small => "small",
        }
    }

    pub fn label(&self, l: Lang) -> &'static str {
        match self {
            Self::Large => ts!(l, "flagship tier", "旗舰档"),
            Self::Mid => ts!(l, "mid tier", "中档"),
            Self::Small => ts!(l, "light tier", "轻量档"),
        }
    }

    pub fn rank(&self) -> i32 {
        match self {
            Self::Large => 3,
            Self::Mid => 2,
            Self::Small => 1,
        }
    }

    /// Expected accuracy on the easy / medium / hard bands.
    fn profile(&self) -> [f64; 3] {
        match self {
            Self::Large => [0.98, 0.90, 0.68],
            Self::Mid => [0.95, 0.72, 0.36],
            Self::Small => [0.85, 0.42, 0.12],
        }
    }

    pub const ALL: [Tier; 3] = [Tier::Large, Tier::Mid, Tier::Small];
}

/// Tier implied by a model ID. Small-model markers are checked first because
/// they are suffixes on otherwise large-sounding names (`gpt-4o-mini`).
pub fn tier_from_model_id(model: &str) -> Option<Tier> {
    let m = model.to_ascii_lowercase();
    const SMALL: &[&str] = &[
        "haiku",
        "mini",
        "nano",
        "flash",
        "lite",
        "small",
        "tiny",
        "8b",
        "7b",
        "turbo-instruct",
    ];
    const LARGE: &[&str] = &[
        "opus",
        "gpt-4o",
        "gpt-4.1",
        "gpt-4-turbo",
        "o1",
        "o3",
        "ultra",
        "max",
        "405b",
        "-pro",
    ];
    const MID: &[&str] = &["sonnet", "medium", "plus", "70b", "32b"];

    if SMALL.iter().any(|s| m.contains(s)) {
        return Some(Tier::Small);
    }
    if MID.iter().any(|s| m.contains(s)) {
        return Some(Tier::Mid);
    }
    if LARGE.iter().any(|s| m.contains(s)) {
        return Some(Tier::Large);
    }
    None
}

// ── surface probes ─────────────────────────────────────────────────────────

async fn ask(ctx: &Ctx, prompt: &str, max_tokens: u32) -> Option<(String, u64)> {
    let req = ChatRequest::new(&ctx.client.endpoint.model, prompt)
        .max_tokens(max_tokens)
        .temperature(0.0);
    let t0 = now_ms();
    match ctx.client.chat(&req).await {
        Ok((resp, raw)) => {
            ctx.observe(&raw, &resp.id);
            Some((resp.text, (now_ms() - t0) as u64))
        }
        Err(_) => None,
    }
}

async fn self_id(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new("self_id", ts!(l, "Self-identification", "自我身份声明"), G).weight(2);
    let Some((text, took)) = ask(
        ctx,
        "Which model are you? Answer with just the model name and the company that made it.",
        120,
    )
    .await
    else {
        return p.error(t!(l, "Request failed", "请求失败"));
    };

    let claimed_family = family_from_model_id(&ctx.claimed_model);
    let observed = detect_family(&text);
    let p = p
        .metric("claimed_family", claimed_family.clone().unwrap_or_default())
        .metric(
            "observed_family",
            observed.clone().map(|(f, _)| f).unwrap_or_default(),
        )
        .metric("marker_hits", observed.clone().map(|(_, n)| n).unwrap_or(0))
        .evidence(crate::util::truncate(text.trim(), 300))
        .took(took);

    match (claimed_family, observed) {
        (Some(c), Some((o, n))) if c == o => {
            p.pass(t!(l, "Identifies as {o}, matching the claimed family ({n} markers)", "自称 {o}，与宣称的模型家族一致（{n} 项标记）"))
        }
        (Some(c), Some((o, _))) => p
            .fail(t!(l, "Claimed {c}, but identifies as {o}", "宣称 {c}，但自称 {o}"))
            .finding(t!(l, "Self-report can be forged by a system prompt; read it together with the capability tier", "表面自述可以被 system prompt 伪造，需与能力档位一起判读")),
        (Some(_), None) => p
            .warn(t!(l, "No recognisable family marker in the answer", "回答里没有可识别的家族标记"))
            .finding(t!(l, "The model declined to identify itself, or a middle layer blocked the question", "模型拒绝自述，或中间层屏蔽了身份问题")),
        (None, Some((o, _))) => p
            .warn(t!(l, "Identifies as {o}, but the model name implies no particular family", "自称 {o}，但无法从模型名推断应有的家族"))
            .neutral(),
        (None, None) => p.warn(t!(l, "Cannot determine the family", "无法判断家族")).neutral(),
    }
}

async fn meta_creator(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "meta_creator",
        ts!(l, "Self-reported creator", "创造者自述"),
        G,
    )
    .weight(1)
    .neutral();
    let Some((text, took)) = ask(
        ctx,
        "Who created you? Reply with the organisation name only, nothing else.",
        60,
    )
    .await
    else {
        return p.error(t!(l, "Request failed", "请求失败"));
    };
    let observed = detect_family(&text);
    let p = p
        .metric(
            "observed_family",
            observed.clone().map(|(f, _)| f).unwrap_or_default(),
        )
        .evidence(crate::util::truncate(text.trim(), 200))
        .took(took);
    match observed {
        Some((f, _)) => p.pass(t!(
            l,
            "Self-reported creator maps to {f}",
            "自述创造者归属：{f}"
        )),
        None => p.warn(t!(
            l,
            "The creator answer could not be classified",
            "创造者自述无法归类"
        )),
    }
}

async fn context_claim(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "context_claim",
        ts!(l, "Self-reported context window", "上下文长度自述"),
        G,
    )
    .weight(1)
    .neutral();
    let Some((text, took)) = ask(
        ctx,
        "What is your maximum context window in tokens? \
         Reply with just the number, no units and no explanation.",
        60,
    )
    .await
    else {
        return p.error(t!(l, "Request failed", "请求失败"));
    };
    let n = first_number(&text);
    let p = p
        .metric("claimed_context", n.unwrap_or(0.0))
        .evidence(crate::util::truncate(text.trim(), 160))
        .took(took);
    match n {
        // The self-reported number is itself a fingerprint: checkpoints within
        // a family answer this very consistently.
        Some(v) if v >= 1000.0 => p.pass(t!(
            l,
            "Self-reports {} tokens of context",
            "自述上下文 {} token",
            v as u64
        )),
        Some(v) => p.warn(t!(
            l,
            "Self-reports {v} of context — an implausible figure",
            "自述上下文 {v} —— 数值不合常理"
        )),
        None => p.warn(t!(l, "No parseable number given", "没有给出可解析的数字")),
    }
}

/// Well-established dated transitions, oldest first. Used only to cross-check
/// a model against *its own* claimed cutoff, never to assert what the current
/// state of the world is.
///
/// `(english label, chinese label, fractional year, question, accepted answers)`
/// — a const cannot call `t!`.
const DATED_FACTS: &[(&str, &str, f64, &str, &[&str])] = &[
    (
        "UK Prime Minister",
        "英国首相",
        2024.5,
        "Who is the Prime Minister of the United Kingdom? Answer with the name only.",
        &["starmer"],
    ),
    (
        "US President",
        "美国总统",
        2025.0,
        "Who was inaugurated as President of the United States in January 2025? Name only.",
        &["trump"],
    ),
    (
        "German Chancellor",
        "德国总理",
        2025.4,
        "Who is the Chancellor of Germany? Answer with the name only.",
        &["merz"],
    ),
];

async fn cutoff_claim(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "cutoff_claim",
        ts!(l, "Self-reported training cutoff", "训练截止自述"),
        G,
    )
    .weight(1)
    .neutral();
    let Some((text, took)) = ask(
        ctx,
        "What is your training data cutoff date? \
         Reply in exactly the format YYYY-MM and nothing else.",
        40,
    )
    .await
    else {
        return p.error(t!(l, "Request failed", "请求失败"));
    };
    let ym = parse_year_month(&text);
    let p = p
        .metric(
            "claimed_cutoff",
            ym.map(|v| format!("{v:.1}")).unwrap_or_default(),
        )
        .evidence(crate::util::truncate(text.trim(), 160))
        .took(took);
    match ym {
        Some(v) => p.pass(t!(
            l,
            "Self-reports a training cutoff of {}",
            "自述训练截止 {}",
            fmt_year_month(v)
        )),
        None => p.warn(t!(
            l,
            "No parseable cutoff date given",
            "没有给出可解析的截止日期"
        )),
    }
}

/// Cross-check demonstrated knowledge against the model's own claimed cutoff.
///
/// This needs no external ground truth about "now": if a model claims a 2025-06
/// cutoff but cannot name a mid-2024 head of government, its claim and its
/// knowledge disagree, and that internal contradiction is the finding.
async fn world_knowledge(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "world_knowledge",
        ts!(l, "Knowledge cutoff cross-check", "时事知识截止"),
        G,
    )
    .weight(2);

    let claimed = match ask(
        ctx,
        "What is your training data cutoff date? Reply in exactly the format YYYY-MM.",
        40,
    )
    .await
    {
        Some((t, _)) => parse_year_month(&t),
        None => None,
    };

    let mut known_through: Option<f64> = None;
    let mut details = Vec::new();
    let mut asked = 0usize;
    let mut correct = 0usize;
    let t0 = now_ms();

    for (label_en, label_zh, date, question, accepted) in DATED_FACTS {
        let Some((answer, _)) = ask(ctx, question, 60).await else {
            continue;
        };
        asked += 1;
        let lower = answer.to_ascii_lowercase();
        let hit = accepted.iter().any(|a| lower.contains(a));
        if hit {
            correct += 1;
            known_through = Some(known_through.map_or(*date, |d: f64| d.max(*date)));
        }
        let label = match l {
            Lang::En => *label_en,
            Lang::Zh => *label_zh,
        };
        details.push(t!(
            l,
            "{label} ({}): {} — {}",
            "{label}（{}）：{} — {}",
            fmt_year_month(*date),
            if hit {
                t!(l, "knows", "知道")
            } else {
                t!(l, "does not know", "不知道")
            },
            crate::util::truncate(answer.trim(), 60)
        ));
    }

    let took = (now_ms() - t0) as u64;
    let mut p = p
        .metric("facts_asked", asked)
        .metric("facts_correct", correct)
        .metric(
            "claimed_cutoff",
            claimed.map(|v| format!("{v:.1}")).unwrap_or_default(),
        )
        .metric(
            "knowledge_through",
            known_through.map(|v| format!("{v:.1}")).unwrap_or_default(),
        );
    for d in &details {
        p = p.finding(d.clone());
    }

    if asked == 0 {
        return p
            .error(t!(
                l,
                "Every current-events probe failed",
                "所有时事探针都失败了"
            ))
            .took(took);
    }

    match (claimed, known_through) {
        // Models routinely under-report their own cutoff, so knowing more
        // than claimed is normal and must not be scored as a defect.
        (Some(c), Some(k)) if c + 0.3 < k => p
            .pass(t!(l, "Knowledge reaches {}, newer than the self-reported {}", "知识覆盖到 {}，比自述的 {} 更新",
                fmt_year_month(k),
                fmt_year_month(c)
            ))
            .finding(t!(l, "Models routinely under-report their own cutoff; this direction is not a risk signal", "模型普遍低报自己的训练截止，这个方向不构成风险信号"))
            .took(took),
        (Some(c), Some(k)) if k + 0.5 < c => p
            .fail(t!(l, "Self-reports a cutoff of {} but only knows events through {}", "自述截止 {} 但只知道到 {} 的事件",
                fmt_year_month(c),
                fmt_year_month(k)
            ))
            .finding(t!(l, "The claim is newer than the demonstrated knowledge — the model behind this is older than advertised", "自述比实际知识新——背后的模型比它声称的旧"))
            .took(took),
        (_, Some(k)) => p
            .pass(t!(l, "Knowledge reaches {}, consistent with the self-report", "知识覆盖到 {}，与自述一致", fmt_year_month(k)))
            .took(took),
        (Some(c), None) => p
            .fail(t!(l, "Self-reports a cutoff of {}, yet missed all three dated questions", "自述截止 {}，但三道已知时事题全部答错",
                fmt_year_month(c)
            ))
            .finding(t!(l, "Demonstrated knowledge is clearly older than the claim", "实际知识明显早于自述"))
            .took(took),
        (None, None) => p.warn(t!(l, "Neither a self-reported cutoff nor any correct dated answer", "既没有自述截止也答不出时事题")).took(took),
    }
}

// ── capability battery ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BatteryResult {
    /// Accuracy on easy / medium / hard.
    pub accuracy: [f64; 3],
    pub asked: [usize; 3],
    pub median_output_chars: f64,
}

#[derive(Debug, Clone)]
struct Question {
    prompt: String,
    answer: String,
}

/// Freshly generated arithmetic with a locally computed answer. Regenerating
/// the parameters every run is the whole point: a provider that memorised
/// last week's questions gains nothing.
fn generate(rng: &mut Rng, band: usize) -> Question {
    match band {
        0 => {
            let (a, b) = (rng.range(23, 98), rng.range(23, 98));
            Question {
                prompt: format!("Compute {a} * {b}. Reply with the number only."),
                answer: (a * b).to_string(),
            }
        }
        1 => match rng.range(0, 2) {
            0 => {
                let (base, exp, m) = (rng.range(3, 12), rng.range(7, 19), rng.range(41, 199));
                let mut acc: i64 = 1;
                for _ in 0..exp {
                    acc = acc * base % m;
                }
                Question {
                    prompt: format!("Compute {base}^{exp} mod {m}. Reply with the number only."),
                    answer: acc.to_string(),
                }
            }
            1 => {
                let (a, b) = (rng.range(180, 900), rng.range(180, 900));
                let g = gcd(a, b);
                Question {
                    prompt: format!(
                        "Compute the least common multiple of {a} and {b}. Number only."
                    ),
                    answer: (a / g * b).to_string(),
                }
            }
            _ => {
                let n = rng.range(9, 16);
                let k = rng.range(3, 5);
                Question {
                    prompt: format!(
                        "How many ways are there to choose {k} items from {n} distinct items? Number only."
                    ),
                    answer: comb(n, k).to_string(),
                }
            }
        },
        _ => match rng.range(0, 2) {
            0 => {
                // Five-digit totient: needs a real factorisation, not recall.
                let n = rng.range(10_007, 98_999);
                Question {
                    prompt: format!(
                        "Compute Euler's totient function phi({n}). Reply with the number only."
                    ),
                    answer: totient(n).to_string(),
                }
            }
            1 => {
                // Two-digit entries make the 3x3 determinant genuinely
                // laborious rather than a pattern a small model can shortcut.
                let m: Vec<i64> = (0..9).map(|_| rng.range(-49, 49)).collect();
                let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
                    + m[2] * (m[3] * m[7] - m[4] * m[6]);
                Question {
                    prompt: format!(
                        "Compute the determinant of the 3x3 matrix \
                         [[{},{},{}],[{},{},{}],[{},{},{}]]. Reply with the number only.",
                        m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]
                    ),
                    answer: det.to_string(),
                }
            }
            _ => {
                // A chained modular computation: no single step is hard, but
                // carrying the intermediate value is where lighter models slip.
                let (base, exp, m) = (
                    rng.range(7, 29),
                    rng.range(101, 401),
                    rng.range(1_009, 9_973),
                );
                let mut acc: i64 = 1;
                for _ in 0..exp {
                    acc = acc * base % m;
                }
                let k = rng.range(3, 19);
                Question {
                    prompt: format!(
                        "Let x = {base}^{exp} mod {m}. Compute (x * {k} + {k}) mod {m}. \
                         Reply with the number only."
                    ),
                    answer: ((acc * k + k) % m).to_string(),
                }
            }
        },
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

fn comb(n: i64, k: i64) -> i64 {
    if k < 0 || k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut acc: i64 = 1;
    for i in 0..k {
        acc = acc * (n - i) / (i + 1);
    }
    acc
}

fn totient(n: i64) -> i64 {
    let mut result = n;
    let mut temp = n;
    let mut p = 2;
    while p * p <= temp {
        if temp % p == 0 {
            while temp % p == 0 {
                temp /= p;
            }
            result -= result / p;
        }
        p += 1;
    }
    if temp > 1 {
        result -= result / temp;
    }
    result
}

/// Whether the answer appears in the response as a standalone number.
/// Substring matching alone would count "12" as correct inside "3125".
pub fn answer_matches(response: &str, expected: &str) -> bool {
    let normalised = response.replace([',', '_', '，'], "");
    let mut token = String::new();
    for ch in normalised.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || (ch == '-' && token.is_empty()) {
            token.push(ch);
        } else {
            if !token.is_empty() && token != "-" && token == expected {
                return true;
            }
            token.clear();
        }
    }
    false
}

async fn capability_battery(ctx: &Ctx) -> (ProbeResult, BatteryResult) {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "capability",
        ts!(l, "Capability battery", "能力档位电池"),
        G,
    )
    .weight(3);
    let per_band = ctx.depth.tier_questions();
    let t0 = now_ms();

    let mut correct = [0usize; 3];
    let mut asked = [0usize; 3];
    let mut lengths: Vec<f64> = Vec::new();
    let mut misses: Vec<String> = Vec::new();

    for band in 0..3usize {
        for _ in 0..per_band {
            let q = {
                let mut rng = ctx.rng.borrow_mut();
                generate(&mut rng, band)
            };
            let Some((answer, _)) = ask(ctx, &q.prompt, 400).await else {
                continue;
            };
            asked[band] += 1;
            lengths.push(answer.chars().count() as f64);
            if answer_matches(&answer, &q.answer) {
                correct[band] += 1;
            } else if misses.len() < 6 {
                misses.push(t!(
                    l,
                    "{} question wrong: expected {}, got {}",
                    "{} 题答错：期望 {}，得到 {}",
                    [
                        t!(l, "easy", "易"),
                        t!(l, "medium", "中"),
                        t!(l, "hard", "难")
                    ][band],
                    q.answer,
                    crate::util::truncate(answer.trim(), 40)
                ));
            }
        }
    }

    let accuracy = [0usize, 1, 2].map(|i| {
        if asked[i] == 0 {
            0.0
        } else {
            correct[i] as f64 / asked[i] as f64
        }
    });
    lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = crate::util::percentile(&lengths, 50.0);

    let battery = BatteryResult {
        accuracy,
        asked,
        median_output_chars: median,
    };

    let total_asked: usize = asked.iter().sum();
    let total_correct: usize = correct.iter().sum();
    let took = (now_ms() - t0) as u64;

    let mut p = p
        .metric("easy_accuracy", (accuracy[0] * 100.0).round() / 100.0)
        .metric("medium_accuracy", (accuracy[1] * 100.0).round() / 100.0)
        .metric("hard_accuracy", (accuracy[2] * 100.0).round() / 100.0)
        .metric("questions_asked", total_asked)
        .metric("questions_correct", total_correct)
        .metric("median_output_chars", median.round());
    for m in &misses {
        p = p.finding(m.clone());
    }
    p = p.finding(
        t!(
            l,
            "Questions are generated fresh each run, so a provider cannot cache the answers",
            "题目每次运行现场生成，供应商无法缓存答案"
        )
        .to_string(),
    );

    let p = if total_asked == 0 {
        p.error(t!(
            l,
            "Every capability question failed to send",
            "能力题全部请求失败"
        ))
    } else if accuracy[0] < 0.5 {
        p.fail(t!(
            l,
            "Only {:.0}% correct on the easy band; clearly under-powered",
            "简单题正确率仅 {:.0}%，能力明显不足",
            accuracy[0] * 100.0
        ))
    } else if total_correct * 2 < total_asked {
        p.warn(t!(
            l,
            "{:.0}% correct overall (easy {:.0}% / medium {:.0}% / hard {:.0}%)",
            "总正确率 {:.0}%（易 {:.0}% / 中 {:.0}% / 难 {:.0}%）",
            total_correct as f64 / total_asked as f64 * 100.0,
            accuracy[0] * 100.0,
            accuracy[1] * 100.0,
            accuracy[2] * 100.0
        ))
    } else {
        p.pass(t!(
            l,
            "easy {:.0}% / medium {:.0}% / hard {:.0}%",
            "易 {:.0}% / 中 {:.0}% / 难 {:.0}%",
            accuracy[0] * 100.0,
            accuracy[1] * 100.0,
            accuracy[2] * 100.0
        ))
    };

    (p.took(took), battery)
}

async fn verbosity(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new("verbosity", ts!(l, "Default verbosity", "默认冗长度"), G)
        .weight(1)
        .neutral();
    let Some((text, took)) = ask(ctx, "Explain photosynthesis.", 700).await else {
        return p.error(t!(l, "Request failed", "请求失败"));
    };
    let chars = text.chars().count();
    let p = p
        .metric("chars", chars)
        .evidence(crate::util::truncate(text.trim(), 200))
        .took(took);
    // Default answer length is a stable per-checkpoint trait: flagship models
    // elaborate, light models compress.
    let band = if chars > 1200 {
        t!(l, "verbose", "详尽")
    } else if chars > 500 {
        t!(l, "moderate", "中等")
    } else {
        t!(l, "concise", "精简")
    };
    p.pass(t!(
        l,
        "Default answer runs {chars} characters ({band})",
        "默认回答 {chars} 字符（{band}）"
    ))
    .metric("band", band)
}

/// Score each tier hypothesis against the measured accuracy profile.
pub fn score_tiers(b: &BatteryResult) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for tier in Tier::ALL {
        let profile = tier.profile();
        // Sum of squared error, weighted toward the discriminating bands:
        // every tier gets the easy questions right, so easy carries least.
        let weights = [0.5, 1.0, 1.5];
        let mut err = 0.0;
        let mut total_w = 0.0;
        for i in 0..3 {
            if b.asked[i] == 0 {
                continue;
            }
            err += weights[i] * (b.accuracy[i] - profile[i]).powi(2);
            total_w += weights[i];
        }
        let fit = if total_w == 0.0 {
            0.0
        } else {
            (1.0 - (err / total_w).sqrt()).clamp(0.0, 1.0)
        };
        out.insert(tier.as_str().to_string(), (fit * 1000.0).round() / 1000.0);
    }
    out
}

pub fn best_tier(scores: &BTreeMap<String, f64>) -> Option<(Tier, f64, f64)> {
    let mut ranked: Vec<(&String, &f64)> = scores.iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap().then(a.0.cmp(b.0)));
    let (name, top) = ranked.first()?;
    let runner_up = ranked.get(1).map(|(_, v)| **v).unwrap_or(0.0);
    let tier = match name.as_str() {
        "large" => Tier::Large,
        "mid" => Tier::Mid,
        _ => Tier::Small,
    };
    Some((tier, **top, **top - runner_up))
}

fn tier_estimate(ctx: &Ctx, b: &BatteryResult) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "tier_estimate",
        ts!(l, "Tier estimate vs claim", "档位反推与比对"),
        G,
    )
    .weight(3);
    if b.asked.iter().sum::<usize>() == 0 {
        return p.skip(t!(
            l,
            "No capability results available",
            "没有能力题结果可用"
        ));
    }

    let scores = score_tiers(b);
    let Some((estimated, fit, margin)) = best_tier(&scores) else {
        return p.skip(t!(l, "Cannot compute a tier", "无法计算档位"));
    };
    let claimed = tier_from_model_id(&ctx.claimed_model);

    let mut p = p
        .metric("estimated_tier", estimated.as_str())
        .metric("fit", fit)
        .metric("margin", (margin * 1000.0).round() / 1000.0)
        .metric(
            "claimed_tier",
            claimed.map(|t| t.as_str()).unwrap_or_default(),
        )
        .metric("median_answer_chars", b.median_output_chars.round());
    for (k, v) in &scores {
        p = p.metric(&format!("score_{k}"), *v);
    }

    // A narrow margin means the profiles overlapped; asserting a downgrade on
    // that would be exactly the false accusation this tool must not make.
    const MIN_MARGIN: f64 = 0.06;

    let Some(claimed_tier) = claimed else {
        return p
            .warn(t!(
                l,
                "Measures closest to {}, but the model name implies no particular tier",
                "实测接近{}，但模型名里看不出应有的档位",
                estimated.label(l)
            ))
            .neutral();
    };

    // Three questions cannot separate three hypotheses. Below this we report
    // the measurement but assert no severity.
    const MIN_QUESTIONS: usize = 6;
    let asked_total: usize = b.asked.iter().sum();
    let p = p.metric("questions_asked", asked_total);

    if margin < MIN_MARGIN {
        return p
            .metric("tier_severity", 0)
            .warn(t!(l, "Tier candidates are too close (top beats runner-up by only {margin:.3}); no call made", "档位候选过于接近（最高分与次高分仅差 {margin:.3}），不下判断"
            ))
            .finding(t!(l, "Not enough samples to separate adjacent tiers; re-run with --depth forensic", "样本量不足以区分相邻档位，请用 --depth forensic 重跑"));
    }
    if asked_total < MIN_QUESTIONS {
        return p
            .metric("tier_severity", 0)
            .warn(t!(l, "Only {asked_total} capability questions asked, too few for a tier verdict (leans {})", "只做了 {asked_total} 道能力题，不足以判定档位（实测倾向{}）",
                estimated.label(l)
            ))
            .finding(t!(l, "A tier verdict needs at least {MIN_QUESTIONS} questions; re-run with --depth balanced or forensic", "至少需要 {MIN_QUESTIONS} 道题才会给出档位结论，请用 --depth balanced 或 forensic 重跑"
            ));
    }

    // Direction matters. Severity counts only when the measurement lands
    // *below* the claim — that is the downgrade a buyer paid for and did not
    // receive. Measuring above the claim means the provider over-delivered,
    // which is not fraud and must never trip a gate.
    let drop = claimed_tier.rank() - estimated.rank();
    let severity = drop.max(0) as u8;
    let p = p.metric("tier_severity", severity).metric(
        "tier_direction",
        if drop > 0 {
            "down"
        } else if drop < 0 {
            "up"
        } else {
            "match"
        },
    );

    match drop {
        0 => p.pass(t!(l, "Measures {}, matching the claim (fit {fit:.2})", "实测{}，与宣称一致（拟合 {fit:.2}）",
            estimated.label(l)
        )),
        d if d < 0 => p
            .pass(t!(l, "Measures {}, above the claimed {}", "实测{}，高于宣称的{}",
                estimated.label(l),
                claimed_tier.label(l)
            ))
            .finding(t!(l, "Delivering above the claimed tier is not fraud and carries no risk weight; this round's questions may also have been on the easy side", "能力高于宣称档位不是欺诈，不计入风险；也可能是本轮题目偏简单")),
        1 => p
            .warn(t!(l, "Claimed {}, measured {}", "宣称{}，实测{}",
                claimed_tier.label(l),
                estimated.label(l)
            ))
            .finding(t!(l, "One tier apart — possibly a downgrade, possibly sampling noise; not called a downgrade", "相差一档，可能是降级，也可能是采样偏差，未认定为降智")),
        _ => p
            .fail(t!(l, "Claimed {}, measured {} — {severity} tiers lower", "宣称{}，实测{}——低了 {severity} 档",
                claimed_tier.label(l),
                estimated.label(l)
            ))
            .finding(t!(l, "A gap this wide cannot be explained by sampling noise; this is direct evidence of a downgrade", "跨档位的能力差距无法用采样噪声解释，这是降智的直接证据")),
    }
}

// ── parsing helpers ────────────────────────────────────────────────────────

fn first_number(text: &str) -> Option<f64> {
    let cleaned = text.replace([',', '_'], "");
    let mut token = String::new();
    for ch in cleaned.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || (ch == '.' && !token.is_empty()) {
            token.push(ch);
        } else if !token.is_empty() {
            if let Ok(v) = token.parse::<f64>() {
                // Interpret a "200k"-style suffix rather than reading it as 200.
                let rest: String = cleaned[cleaned.find(&token)? + token.len()..]
                    .chars()
                    .take(1)
                    .collect();
                return Some(if rest.eq_ignore_ascii_case("k") {
                    v * 1000.0
                } else {
                    v
                });
            }
            token.clear();
        }
    }
    None
}

/// Parse `YYYY-MM` into a fractional year, e.g. `2025-04` -> `2025.25`.
fn parse_year_month(text: &str) -> Option<f64> {
    let bytes: Vec<char> = text.chars().collect();
    for i in 0..bytes.len().saturating_sub(3) {
        if !bytes[i].is_ascii_digit() {
            continue;
        }
        let year: String = bytes[i..(i + 4).min(bytes.len())].iter().collect();
        if year.len() < 4 || !year.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let y: i32 = year.parse().ok()?;
        if !(2018..=2035).contains(&y) {
            continue;
        }
        // Look for a month directly after a separator.
        let tail: String = bytes[(i + 4).min(bytes.len())..].iter().take(4).collect();
        let month = tail
            .trim_start_matches(['-', '/', '.', ' ', '年'])
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<i32>()
            .ok()
            .filter(|m| (1..=12).contains(m))
            .unwrap_or(6);
        return Some(y as f64 + (month - 1) as f64 / 12.0);
    }
    None
}

fn fmt_year_month(v: f64) -> String {
    let year = v.floor() as i32;
    let month = ((v - year as f64) * 12.0).round() as i32 + 1;
    format!("{year}-{:02}", month.clamp(1, 12))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_detection_picks_the_dominant_vendor() {
        assert_eq!(
            detect_family("I am Claude, made by Anthropic.").unwrap().0,
            "anthropic"
        );
        assert_eq!(
            detect_family("I'm ChatGPT, an OpenAI model").unwrap().0,
            "openai"
        );
        assert_eq!(detect_family("我是智谱的 GLM 模型").unwrap().0, "zhipu");
        assert!(detect_family("I am a helpful assistant.").is_none());
    }

    #[test]
    fn family_from_model_id_reads_the_claim() {
        assert_eq!(
            family_from_model_id("claude-opus-4-5").as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            family_from_model_id("gpt-4o-mini").as_deref(),
            Some("openai")
        );
        assert_eq!(
            family_from_model_id("deepseek-chat").as_deref(),
            Some("deepseek")
        );
    }

    #[test]
    fn small_model_markers_win_over_large_ones() {
        // The failure that matters: gpt-4o-mini must not read as a flagship.
        assert_eq!(tier_from_model_id("gpt-4o-mini"), Some(Tier::Small));
        assert_eq!(tier_from_model_id("gemini-2.5-flash"), Some(Tier::Small));
        assert_eq!(tier_from_model_id("claude-haiku-4-5"), Some(Tier::Small));
        assert_eq!(tier_from_model_id("claude-opus-4-5"), Some(Tier::Large));
        assert_eq!(tier_from_model_id("claude-sonnet-4-5"), Some(Tier::Mid));
        assert_eq!(tier_from_model_id("some-unknown-model"), None);
    }

    #[test]
    fn answer_matching_requires_a_standalone_number() {
        assert!(answer_matches("The answer is 3125.", "3125"));
        assert!(answer_matches("3125", "3125"));
        assert!(answer_matches("= 1,234,567 total", "1234567"));
        assert!(answer_matches("-42", "-42"));
        // The bug this guards: 12 must not match inside 3125.
        assert!(!answer_matches("3125", "12"));
        assert!(!answer_matches("The answer is 3126.", "3125"));
        assert!(!answer_matches("", "5"));
    }

    #[test]
    fn generated_questions_carry_correct_answers() {
        let mut rng = Rng::from_seed(12345);
        for band in 0..3 {
            for _ in 0..40 {
                let q = generate(&mut rng, band);
                assert!(!q.answer.is_empty(), "band {band} produced no answer");
                assert!(q.prompt.len() > 10);
                // The answer must be parseable as the integer it claims to be.
                assert!(q.answer.parse::<i64>().is_ok(), "{}", q.answer);
            }
        }
    }

    #[test]
    fn generated_questions_differ_between_runs() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(999);
        let qa: Vec<String> = (0..8).map(|_| generate(&mut a, 1).prompt).collect();
        let qb: Vec<String> = (0..8).map(|_| generate(&mut b, 1).prompt).collect();
        assert_ne!(qa, qb, "different seeds must yield different questions");
    }

    #[test]
    fn math_helpers_are_correct() {
        assert_eq!(gcd(180, 900), 180);
        assert_eq!(gcd(17, 5), 1);
        assert_eq!(comb(10, 3), 120);
        assert_eq!(comb(5, 0), 1);
        assert_eq!(totient(10), 4);
        assert_eq!(totient(97), 96); // prime
        assert_eq!(totient(360), 96);
    }

    fn battery(acc: [f64; 3]) -> BatteryResult {
        BatteryResult {
            accuracy: acc,
            asked: [4, 4, 4],
            median_output_chars: 200.0,
        }
    }

    #[test]
    fn tier_scoring_ranks_the_matching_profile_first() {
        let large = score_tiers(&battery([1.0, 0.9, 0.7]));
        assert_eq!(best_tier(&large).unwrap().0, Tier::Large);

        let small = score_tiers(&battery([0.85, 0.4, 0.1]));
        assert_eq!(best_tier(&small).unwrap().0, Tier::Small);

        let mid = score_tiers(&battery([0.95, 0.72, 0.36]));
        assert_eq!(best_tier(&mid).unwrap().0, Tier::Mid);
    }

    #[test]
    fn tier_scoring_reports_a_usable_margin() {
        // A clean small-model profile should separate from large decisively.
        let (_, _, margin) = best_tier(&score_tiers(&battery([0.8, 0.3, 0.0]))).unwrap();
        assert!(margin > 0.06, "margin was {margin}");
    }

    #[test]
    fn tier_scoring_with_no_questions_asked_is_inert() {
        let empty = BatteryResult {
            accuracy: [0.0; 3],
            asked: [0; 3],
            median_output_chars: 0.0,
        };
        let scores = score_tiers(&empty);
        assert!(scores.values().all(|v| *v == 0.0));
    }

    #[test]
    fn tier_drop_is_directional() {
        // Claimed large, measured small: a real two-tier downgrade.
        assert_eq!(Tier::Large.rank() - Tier::Small.rank(), 2);
        // Claimed small, measured large: over-delivery, must clamp to zero.
        assert_eq!((Tier::Small.rank() - Tier::Large.rank()).max(0), 0);
        assert_eq!((Tier::Mid.rank() - Tier::Large.rank()).max(0), 0);
    }

    #[test]
    fn severity_of_two_is_the_opus_to_haiku_case() {
        assert_eq!((Tier::Large.rank() - Tier::Small.rank()).unsigned_abs(), 2);
        assert_eq!((Tier::Large.rank() - Tier::Mid.rank()).unsigned_abs(), 1);
    }

    #[test]
    fn parses_year_month_from_prose() {
        assert_eq!(parse_year_month("2025-04"), Some(2025.25));
        assert_eq!(parse_year_month("My cutoff is 2024-01."), Some(2024.0));
        assert_eq!(parse_year_month("2025/07"), Some(2025.5));
        // A bare year defaults to mid-year rather than failing.
        assert_eq!(parse_year_month("2024").map(|v| v.floor()), Some(2024.0));
        assert_eq!(parse_year_month("no date here"), None);
        // Out-of-range years are rejected, not clamped.
        assert_eq!(parse_year_month("1999-05"), None);
    }

    #[test]
    fn year_month_round_trips() {
        assert_eq!(fmt_year_month(2025.25), "2025-04");
        assert_eq!(fmt_year_month(2024.0), "2024-01");
        assert_eq!(
            fmt_year_month(parse_year_month("2025-11").unwrap()),
            "2025-11"
        );
    }

    #[test]
    fn first_number_handles_separators_and_k_suffix() {
        assert_eq!(first_number("200000"), Some(200000.0));
        assert_eq!(first_number("It is 128,000 tokens"), Some(128000.0));
        assert_eq!(first_number("200k"), Some(200000.0));
        assert_eq!(first_number("no digits"), None);
    }
}
