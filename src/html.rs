// SPDX-License-Identifier: Apache-2.0
//! Self-contained HTML report.
//!
//! Everything is inlined — no CDN, no fonts, no scripts fetched at view time —
//! so the file can be mailed to a provider as evidence and still render years
//! later. Charts are hand-built SVG with CSS animation rather than a charting
//! library, which keeps the artefact under a few hundred kilobytes.

use crate::report::*;
use crate::util::html_escape as esc;
use std::fmt::Write;

const CSS: &str = r#"
:root{
  --ground:#F5F7F8; --surface:#FFFFFF; --surface-2:#EAEEF1; --surface-3:#DFE5E9;
  --ink:#131A1D; --ink-2:#3C484F; --muted:#68757E;
  --rule:#D8E0E4; --rule-2:#BCC7CD;
  --accent:#0D5A66; --accent-2:#0A454F; --accent-wash:#DBEDEF;
  --pass:#1B6E48; --warn:#8C6115; --fail:#A3352B; --skip:#7A868D;
  --pass-bg:#E3F2E9; --warn-bg:#FBF0DC; --fail-bg:#FBE5E2; --skip-bg:#ECEFF1;
  --shadow:0 1px 2px rgba(16,26,30,.05),0 8px 24px -12px rgba(16,26,30,.14);
  --mono:ui-monospace,"SF Mono",SFMono-Regular,"Cascadia Mono",Menlo,Consolas,monospace;
  --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,"PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;
}
@media (prefers-color-scheme:dark){
  :root:not([data-theme="light"]){
    --ground:#0D1114; --surface:#151B1F; --surface-2:#1D252A; --surface-3:#263036;
    --ink:#E2E8EB; --ink-2:#B0BCC3; --muted:#839099;
    --rule:#232C32; --rule-2:#33414A;
    --accent:#4EA8B6; --accent-2:#7FC7D2; --accent-wash:#102E34;
    --pass:#4FAE81; --warn:#C99A45; --fail:#DB7568; --skip:#78868E;
    --pass-bg:#12291F; --warn-bg:#2B2213; --fail-bg:#2E1A18; --skip-bg:#1D2429;
    --shadow:0 1px 2px rgba(0,0,0,.4),0 8px 24px -12px rgba(0,0,0,.6);
  }
}
:root[data-theme="dark"]{
  --ground:#0D1114; --surface:#151B1F; --surface-2:#1D252A; --surface-3:#263036;
  --ink:#E2E8EB; --ink-2:#B0BCC3; --muted:#839099;
  --rule:#232C32; --rule-2:#33414A;
  --accent:#4EA8B6; --accent-2:#7FC7D2; --accent-wash:#102E34;
  --pass:#4FAE81; --warn:#C99A45; --fail:#DB7568; --skip:#78868E;
  --pass-bg:#12291F; --warn-bg:#2B2213; --fail-bg:#2E1A18; --skip-bg:#1D2429;
  --shadow:0 1px 2px rgba(0,0,0,.4),0 8px 24px -12px rgba(0,0,0,.6);
}

*{box-sizing:border-box}
body{margin:0;background:var(--ground);color:var(--ink);font-family:var(--sans);
     font-size:15px;line-height:1.7;-webkit-font-smoothing:antialiased}
.wrap{max-width:1120px;margin:0 auto;padding:0 26px}
h1,h2,h3{font-family:var(--mono);font-weight:600;letter-spacing:-.01em;margin:0;text-wrap:balance}

/* ── entrance animation ─────────────────────────────── */
.rise{opacity:0;transform:translateY(14px);animation:rise .55s cubic-bezier(.2,.7,.3,1) forwards}
@keyframes rise{to{opacity:1;transform:none}}
@media (prefers-reduced-motion:reduce){
  .rise{animation:none;opacity:1;transform:none}
  .donut-ring,.bar-fill,.gauge-needle,.dist-bar{animation:none !important}
  .donut-ring{stroke-dashoffset:var(--final) !important}
  .bar-fill{width:var(--w) !important}
  .dist-bar{height:var(--h) !important}
}

/* ── masthead ───────────────────────────────────────── */
.mast{background:var(--surface);border-bottom:1px solid var(--rule)}
.mast .wrap{padding:30px 26px 26px}
.brand{display:flex;align-items:baseline;gap:12px;flex-wrap:wrap}
.brand .name{font-family:var(--mono);font-size:15px;font-weight:600;color:var(--accent)}
.brand .ver{font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.target{margin-top:14px;display:grid;gap:4px 22px;
        grid-template-columns:repeat(auto-fit,minmax(210px,1fr));
        font-family:var(--mono);font-size:12.5px;color:var(--ink-2)}
.target b{color:var(--muted);font-weight:500;margin-right:6px}
.target span{overflow-wrap:anywhere}

/* ── hero ───────────────────────────────────────────── */
.hero{display:grid;grid-template-columns:auto 1fr;gap:32px;align-items:center;
      padding:30px 0 6px}
@media (max-width:720px){.hero{grid-template-columns:1fr;gap:20px}}
.donut{position:relative;width:168px;height:168px;flex:0 0 auto}
.donut svg{transform:rotate(-90deg)}
.donut-track{fill:none;stroke:var(--surface-3);stroke-width:13}
.donut-ring{fill:none;stroke-width:13;stroke-linecap:round;
            stroke-dasharray:var(--circ);stroke-dashoffset:var(--circ);
            animation:draw 1.3s cubic-bezier(.25,.8,.3,1) .25s forwards}
@keyframes draw{to{stroke-dashoffset:var(--final)}}
.donut-label{position:absolute;inset:0;display:grid;place-content:center;text-align:center}
.donut-label .n{font-family:var(--mono);font-size:38px;font-weight:600;line-height:1;
                font-variant-numeric:tabular-nums}
.donut-label .u{font-size:11px;color:var(--muted);font-family:var(--mono);letter-spacing:.1em}

.verdict-chip{display:inline-flex;align-items:center;gap:9px;padding:7px 15px;border-radius:999px;
              font-weight:600;font-size:15px;border:1px solid transparent}
.verdict-chip .dot{width:9px;height:9px;border-radius:50%;background:currentColor;
                   animation:pulse 2.4s ease-in-out infinite}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.35}}
.v-good{background:var(--pass-bg);color:var(--pass);border-color:color-mix(in srgb,var(--pass) 30%,transparent)}
.v-mid{background:var(--accent-wash);color:var(--accent);border-color:color-mix(in srgb,var(--accent) 30%,transparent)}
.v-warn{background:var(--warn-bg);color:var(--warn);border-color:color-mix(in srgb,var(--warn) 30%,transparent)}
.v-bad{background:var(--fail-bg);color:var(--fail);border-color:color-mix(in srgb,var(--fail) 30%,transparent)}
.v-none{background:var(--skip-bg);color:var(--skip);border-color:var(--rule-2)}

.hero-body h1{font-size:clamp(23px,3.2vw,31px);margin:14px 0 8px}
.hero-body p{margin:0;color:var(--ink-2);max-width:62ch}
.hero-meta{margin-top:16px;display:flex;flex-wrap:wrap;gap:8px}
.pill{font-family:var(--mono);font-size:11.5px;padding:4px 10px;border-radius:5px;
      background:var(--surface-2);color:var(--ink-2);border:1px solid var(--rule)}
.pill b{color:var(--ink);font-weight:600}

/* ── gate alert ─────────────────────────────────────── */
.gates{margin:22px 0 0;border:1px solid color-mix(in srgb,var(--fail) 40%,transparent);
       border-radius:9px;background:var(--fail-bg);overflow:hidden}
.gates header{padding:12px 18px;font-family:var(--mono);font-size:13px;font-weight:600;
              color:var(--fail);border-bottom:1px solid color-mix(in srgb,var(--fail) 25%,transparent);
              display:flex;align-items:center;gap:9px}
.gates ul{margin:0;padding:12px 18px 14px 36px}
.gates li{margin-bottom:6px;font-size:14px;color:var(--ink)}
.gates li b{font-family:var(--mono);font-size:13px}

/* ── layout ─────────────────────────────────────────── */
section{padding:34px 0;border-top:1px solid var(--rule)}
.sec-head{display:flex;align-items:baseline;gap:12px;margin-bottom:5px;flex-wrap:wrap}
.sec-head h2{font-size:18px}
.sec-head .hint{font-size:13px;color:var(--muted)}
.grid{display:grid;gap:14px}
.g2{grid-template-columns:repeat(auto-fit,minmax(330px,1fr))}
.g4{grid-template-columns:repeat(auto-fit,minmax(178px,1fr))}
.card{background:var(--surface);border:1px solid var(--rule);border-radius:9px;
      padding:17px 19px;box-shadow:var(--shadow)}
.card h3{font-size:12px;letter-spacing:.09em;text-transform:uppercase;color:var(--muted);
         margin-bottom:11px}

/* ── stat tiles ─────────────────────────────────────── */
.stat .v{font-family:var(--mono);font-size:29px;font-weight:600;line-height:1.15;
         font-variant-numeric:tabular-nums}
.stat .s{font-size:12.5px;color:var(--muted);margin-top:3px}
.stat .v.good{color:var(--pass)} .stat .v.warn{color:var(--warn)} .stat .v.bad{color:var(--fail)}

/* ── bars ───────────────────────────────────────────── */
.bars{display:grid;gap:9px}
.bar-row{display:grid;grid-template-columns:96px 1fr 52px;gap:11px;align-items:center;font-size:13px}
.bar-row .lbl{color:var(--ink-2);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bar-track{height:9px;border-radius:5px;background:var(--surface-3);overflow:hidden}
.bar-fill{height:100%;border-radius:5px;width:0;animation:grow 1s cubic-bezier(.22,.8,.3,1) forwards}
@keyframes grow{to{width:var(--w)}}
.bar-row .num{font-family:var(--mono);font-size:12.5px;text-align:right;
              font-variant-numeric:tabular-nums;color:var(--ink-2)}

/* ── billing comparison ─────────────────────────────── */
.cmp{display:grid;gap:12px;margin-top:4px}
.cmp-row .top{display:flex;justify-content:space-between;font-size:12.5px;margin-bottom:5px}
.cmp-row .top .k{color:var(--muted)}
.cmp-row .top .n{font-family:var(--mono);font-variant-numeric:tabular-nums}
.ratio-note{margin-top:13px;padding:11px 14px;border-radius:7px;font-size:13.5px;line-height:1.6}
.ratio-ok,.v-good.ratio-note{background:var(--pass-bg);color:var(--pass)}
.ratio-hi{background:var(--warn-bg);color:var(--warn)}
.ratio-bad{background:var(--fail-bg);color:var(--fail)}

/* ── distribution chart ─────────────────────────────── */
.dist{display:flex;align-items:flex-end;gap:4px;height:96px;padding-top:8px}
.dist-col{flex:1;display:flex;flex-direction:column;justify-content:flex-end;min-width:5px}
.dist-bar{width:100%;border-radius:3px 3px 0 0;background:var(--accent);height:0;
          animation:rise-bar .8s cubic-bezier(.22,.8,.3,1) forwards;opacity:.85}
@keyframes rise-bar{to{height:var(--h)}}
.dist-axis{display:flex;justify-content:space-between;font-family:var(--mono);font-size:10.5px;
           color:var(--muted);margin-top:6px;border-top:1px solid var(--rule);padding-top:5px}

/* ── hop chain ──────────────────────────────────────── */
.chain{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-bottom:12px}
.hop{font-family:var(--mono);font-size:12px;padding:5px 11px;border-radius:6px;
     background:var(--accent-wash);color:var(--accent);border:1px solid color-mix(in srgb,var(--accent) 26%,transparent)}
.hop.you{background:var(--surface-2);color:var(--ink-2);border-color:var(--rule)}
.arrow{color:var(--muted);font-family:var(--mono);font-size:13px}

/* ── evidence lists ─────────────────────────────────── */
.ev{margin:0;padding-left:19px;font-size:13.5px;color:var(--ink-2)}
.ev li{margin-bottom:4px}

/* ── probe table ────────────────────────────────────── */
.tbl-wrap{overflow-x:auto;border:1px solid var(--rule);border-radius:9px;background:var(--surface)}
table{border-collapse:collapse;width:100%;min-width:660px;font-size:13.5px}
thead th{font-family:var(--mono);font-size:10.5px;letter-spacing:.09em;text-transform:uppercase;
         color:var(--muted);font-weight:600;text-align:left;padding:10px 14px;
         background:var(--surface-2);border-bottom:1px solid var(--rule-2);white-space:nowrap}
tbody td{padding:9px 14px;border-bottom:1px solid var(--rule);vertical-align:top}
tbody tr:last-child td{border-bottom:none}
tbody tr.grp td{background:var(--surface-2);font-family:var(--mono);font-size:11.5px;
                letter-spacing:.06em;color:var(--muted);padding:7px 14px}
td.st{width:74px;white-space:nowrap}
td.pid{font-family:var(--mono);font-size:11.5px;color:var(--muted);white-space:nowrap}
td.plabel{font-weight:600;white-space:nowrap}
td.psum{color:var(--ink-2)}
td.pms{font-family:var(--mono);font-size:12px;text-align:right;color:var(--muted);
       white-space:nowrap;font-variant-numeric:tabular-nums}
.tag{display:inline-flex;align-items:center;gap:5px;font-family:var(--mono);font-size:11px;
     font-weight:600;padding:2px 8px;border-radius:4px}
.tag.pass{background:var(--pass-bg);color:var(--pass)}
.tag.warn{background:var(--warn-bg);color:var(--warn)}
.tag.fail{background:var(--fail-bg);color:var(--fail)}
.tag.skip{background:var(--skip-bg);color:var(--skip)}
.tag.err{background:var(--fail-bg);color:var(--fail)}
details.detail{margin-top:6px}
details.detail summary{cursor:pointer;font-size:12px;color:var(--accent);font-family:var(--mono);
                       list-style:none;display:inline-block;padding:1px 0}
details.detail summary::-webkit-details-marker{display:none}
details.detail summary:before{content:"▸ ";transition:none}
details.detail[open] summary:before{content:"▾ "}
details.detail summary:hover{text-decoration:underline}
.detail-body{margin-top:7px;padding:10px 13px;background:var(--surface-2);border-radius:6px;
             font-size:12.5px;color:var(--ink-2);border-left:2px solid var(--rule-2)}
.detail-body ul{margin:0 0 7px;padding-left:17px}
.detail-body pre{margin:6px 0 0;white-space:pre-wrap;overflow-wrap:anywhere;
                 font-family:var(--mono);font-size:11.5px;color:var(--ink);
                 background:var(--surface);padding:9px 11px;border-radius:5px;
                 border:1px solid var(--rule);max-height:230px;overflow:auto}
.kv{display:flex;flex-wrap:wrap;gap:5px 9px;margin-top:5px}
.kv code{font-family:var(--mono);font-size:11px;background:var(--surface);padding:1px 6px;
         border-radius:4px;border:1px solid var(--rule);color:var(--ink-2)}

/* ── trace / notes ──────────────────────────────────── */
ol.trace{margin:0;padding-left:22px;font-family:var(--mono);font-size:12.5px;color:var(--ink-2)}
ol.trace li{margin-bottom:4px}
.note{border-left:3px solid var(--accent);background:var(--surface);padding:14px 18px;
      border-radius:0 7px 7px 0;font-size:13.5px;line-height:1.7;color:var(--ink-2)}
.note b{color:var(--ink)}
.note.warnbox{border-left-color:var(--warn)}
.limits{margin:0;padding-left:20px;font-size:13.5px;color:var(--ink-2)}
.limits li{margin-bottom:5px}

footer{padding:26px 0 46px;color:var(--muted);font-size:12px;font-family:var(--mono);
       border-top:1px solid var(--rule);margin-top:10px}
footer p{margin:3px 0;overflow-wrap:anywhere}
"#;

/// Bar colour by percentage, shared by every scored bar in the report.
fn score_colour(pct: f64) -> &'static str {
    if pct >= 85.0 {
        "var(--pass)"
    } else if pct >= 60.0 {
        "var(--warn)"
    } else {
        "var(--fail)"
    }
}

fn stat_class(pct: f64) -> &'static str {
    if pct >= 85.0 {
        "good"
    } else if pct >= 60.0 {
        "warn"
    } else {
        "bad"
    }
}

pub fn render(rep: &Report) -> String {
    let mut h = String::with_capacity(96 * 1024);

    let _ = write!(
        h,
        r#"<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>llm-verify · {host}</title>
<style>{CSS}</style>
"#,
        host = esc(&rep.host),
    );

    masthead(&mut h, rep);
    hero(&mut h, rep);

    let _ = write!(h, r#"<main class="wrap">"#);
    gates(&mut h, rep);
    key_stats(&mut h, rep);
    group_scores(&mut h, rep);
    identity_panel(&mut h, rep);
    billing_panel(&mut h, rep);
    perf_panel(&mut h, rep);
    channel_panel(&mut h, rep);
    probe_table(&mut h, rep);
    trace_panel(&mut h, rep);
    limits_panel(&mut h);
    let _ = write!(h, "</main>");

    footer(&mut h, rep);
    let _ = write!(h, "{}", ANIM_STAGGER);
    h
}

/// Stagger the entrance animations without shipping a script: each `.rise`
/// gets a delay from its index via a small inline style block.
const ANIM_STAGGER: &str = r#"<style>
.rise:nth-of-type(1){animation-delay:.02s}.rise:nth-of-type(2){animation-delay:.06s}
.rise:nth-of-type(3){animation-delay:.10s}.rise:nth-of-type(4){animation-delay:.14s}
.rise:nth-of-type(5){animation-delay:.18s}.rise:nth-of-type(6){animation-delay:.22s}
.rise:nth-of-type(7){animation-delay:.26s}.rise:nth-of-type(8){animation-delay:.30s}
.rise:nth-of-type(n+9){animation-delay:.34s}
</style>"#;

fn masthead(h: &mut String, rep: &Report) {
    let _ = write!(
        h,
        r#"<header class="mast"><div class="wrap">
<div class="brand"><span class="name">llm-verify</span><span class="ver">v{ver} · {started}</span></div>
<div class="target">
<div><b>端点</b><span>{base}</span></div>
<div><b>模型</b><span>{model}</span></div>
<div><b>协议</b><span>{proto}</span></div>
<div><b>深度</b><span>{depth}</span></div>
</div></div></header>"#,
        ver = esc(&rep.tool_version),
        started = esc(&rep.started_at),
        base = esc(&rep.base_url),
        model = esc(&rep.model),
        proto = rep.protocol,
        depth = esc(&rep.depth),
    );
}

fn hero(h: &mut String, rep: &Report) {
    let v = &rep.verdict;
    // r=64 ring on a 168px box; circumference drives the draw animation.
    const R: f64 = 64.0;
    let circ = 2.0 * std::f64::consts::PI * R;
    let final_offset = circ * (1.0 - (v.score / 100.0).clamp(0.0, 1.0));

    let _ = write!(
        h,
        r#"<div class="wrap"><div class="hero">
<div class="donut">
<svg width="168" height="168" viewBox="0 0 168 168" role="img" aria-label="综合评分 {score:.1} 分">
<circle class="donut-track" cx="84" cy="84" r="{r}"></circle>
<circle class="donut-ring" cx="84" cy="84" r="{r}"
        style="--circ:{circ:.2};--final:{fin:.2};stroke:{col}"></circle>
</svg>
<div class="donut-label"><div class="n">{score:.0}</div><div class="u">/ 100</div></div>
</div>
<div class="hero-body">
<span class="verdict-chip {vcss}"><span class="dot"></span>{vlabel}</span>
<h1>{host}</h1>
<p>{vdesc}</p>
<div class="hero-meta">
<span class="pill"><b>来源</b> {chan}</span>
<span class="pill"><b>身份</b> {ident}</span>
<span class="pill"><b>置信度</b> {conf:.0}%</span>
<span class="pill"><b>探针</b> {pass} 通过 / {warn} 警告 / {fail} 失败</span>
<span class="pill"><b>请求</b> {reqs} 次 · {secs:.1}s</span>
</div>
</div></div></div>"#,
        r = R,
        circ = circ,
        fin = final_offset,
        col = score_colour(v.score),
        score = v.score,
        vcss = v.authenticity.css(),
        vlabel = v.authenticity.label_zh(),
        host = esc(&rep.host),
        vdesc = esc(v.authenticity.desc_zh()),
        chan = v.channel.label_zh(),
        ident = rep.identity.status.label_zh(),
        conf = v.confidence * 100.0,
        pass = rep.count(Status::Pass),
        warn = rep.count(Status::Warn),
        fail = rep.count(Status::Fail),
        reqs = rep.request_count,
        secs = rep.duration_ms as f64 / 1000.0,
    );
}

fn gates(h: &mut String, rep: &Report) {
    let gates = &rep.verdict.hard_gate_hits;
    if gates.is_empty() {
        return;
    }
    let _ = write!(
        h,
        r#"<div class="gates rise"><header>⛔ 硬门禁命中 {n} 项 —— 加权分再高也不能忽略</header><ul>"#,
        n = gates.len()
    );
    for g in gates {
        let _ = write!(
            h,
            "<li><b>{}</b> — {}<br><span style=\"color:var(--muted);font-size:12.5px\">来自探针 {}</span></li>",
            esc(&g.name),
            esc(&g.reason),
            esc(&g.probe)
        );
    }
    let _ = write!(h, "</ul></div>");
}

fn key_stats(h: &mut String, rep: &Report) {
    let b = &rep.billing;
    let p = &rep.perf;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>关键指标</h2>
<span class="hint">一眼看懂这个端点当前的状态</span></div><div class="grid g4">"#
    );

    // Billing ratio.
    let (ratio_txt, ratio_cls, ratio_sub) = if b.honest_input > 0 {
        let cls = if b.input_ratio > 1.5 {
            "bad"
        } else if b.input_ratio > 1.15 {
            "warn"
        } else {
            "good"
        };
        (
            format!("{:.2}×", b.input_ratio),
            cls,
            format!("计费 {} / 实测 {} token", b.billed_input, b.honest_input),
        )
    } else {
        ("—".to_string(), "", "没有可对照的独立计数".to_string())
    };
    stat_tile(h, "计费倍率", &ratio_txt, ratio_cls, &ratio_sub);

    let ttft = if p.ttft_p50 > 0.0 {
        (
            format!("{:.0}ms", p.ttft_p50),
            if p.ttft_p50 <= 1000.0 {
                "good"
            } else if p.ttft_p50 <= 3000.0 {
                "warn"
            } else {
                "bad"
            },
            format!("P95 {:.0}ms · {} 个样本", p.ttft_p95, p.ttft_ms.len()),
        )
    } else {
        ("—".into(), "", "没有流式样本".into())
    };
    stat_tile(h, "首字延迟 P50", &ttft.0, ttft.1, &ttft.2);

    let tps = if p.tps_mean > 0.0 {
        (
            format!("{:.0}", p.tps_mean),
            "",
            format!(
                "tok/s · {}",
                match crate::probes::perf::tier_band(p.tps_mean) {
                    "large" => "偏大模型速度",
                    "small" => "偏小模型速度",
                    _ => "中间档",
                }
            ),
        )
    } else {
        ("—".into(), "", "tok/s".into())
    };
    stat_tile(h, "生成吞吐", &tps.0, tps.1, &tps.2);

    let cov = (1.0 - rep.verdict.coverage_gap) * 100.0;
    stat_tile(
        h,
        "探针覆盖率",
        &format!("{cov:.0}%"),
        stat_class(cov),
        &format!("{} 项未执行", rep.skipped.len()),
    );

    let _ = write!(h, "</div></section>");
}

fn stat_tile(h: &mut String, label: &str, value: &str, cls: &str, sub: &str) {
    let _ = write!(
        h,
        r#"<div class="card stat"><h3>{}</h3><div class="v {}">{}</div><div class="s">{}</div></div>"#,
        esc(label),
        cls,
        esc(value),
        esc(sub)
    );
}

fn group_scores(h: &mut String, rep: &Report) {
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>分组得分</h2>
<span class="hint">问题出在哪一层，一眼定位</span></div><div class="card"><div class="bars">"#
    );
    for g in Group::ALL {
        let Some(pct) = rep.verdict.group_scores.get(g.key()) else {
            continue;
        };
        let _ = write!(
            h,
            r#"<div class="bar-row"><div class="lbl" title="{hint}">{label}</div>
<div class="bar-track"><div class="bar-fill" style="--w:{pct:.1}%;background:{col}"></div></div>
<div class="num">{pct:.1}</div></div>"#,
            hint = esc(g.blurb_zh()),
            label = esc(g.label_zh()),
            pct = pct,
            col = score_colour(*pct),
        );
    }
    let _ = write!(h, "</div></div></section>");
}

fn identity_panel(h: &mut String, rep: &Report) {
    let id = &rep.identity;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>模型身份</h2>
<span class="hint">背后跑的是不是它声称的那个模型</span></div><div class="grid g2">"#
    );

    // Claim vs observation.
    let _ = write!(
        h,
        r#"<div class="card"><h3>宣称 vs 实测</h3>
<div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">宣称模型</span><span class="n">{claimed}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">宣称家族 / 档位</span><span class="n">{cf} / {ct}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">实测家族</span><span class="n">{of}（信心 {fc:.0}%）</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">实测档位</span><span class="n">{et}（拟合 {tc:.2}）</span></div></div>
</div>
<div class="ratio-note {ncls}" style="margin-top:14px">{istatus}</div>
</div>"#,
        claimed = esc(&id.claimed_model),
        cf = esc(id.claimed_family.as_deref().unwrap_or("未知")),
        ct = esc(id.claimed_tier.as_deref().unwrap_or("未知")),
        of = esc(id.observed_family.as_deref().unwrap_or("未识别")),
        fc = id.family_confidence * 100.0,
        et = esc(id.estimated_tier.as_deref().unwrap_or("未测出")),
        tc = id.tier_confidence,
        ncls = id.status.css(),
        istatus = esc(id.status.label_zh()),
    );

    // Capability profile.
    let _ = write!(
        h,
        r#"<div class="card"><h3>能力档位拟合</h3><div class="bars">"#
    );
    for (tier, label) in [("large", "旗舰档"), ("mid", "中档"), ("small", "轻量档")] {
        let score = id.tier_scores.get(tier).copied().unwrap_or(0.0);
        let is_best = id.estimated_tier.as_deref() == Some(tier);
        let _ = write!(
            h,
            r#"<div class="bar-row"><div class="lbl">{label}{mark}</div>
<div class="bar-track"><div class="bar-fill" style="--w:{w:.1}%;background:{col}"></div></div>
<div class="num">{score:.2}</div></div>"#,
            label = label,
            mark = if is_best { " ●" } else { "" },
            w = score * 100.0,
            col = if is_best {
                "var(--accent)"
            } else {
                "var(--rule-2)"
            },
            score = score,
        );
    }
    let _ = write!(h, "</div>");

    if !id.accuracy_by_difficulty.is_empty() {
        let _ = write!(
            h,
            r#"<h3 style="margin-top:16px">分难度正确率</h3><div class="bars">"#
        );
        for (key, label) in [("easy", "简单"), ("medium", "中等"), ("hard", "困难")] {
            let acc = id.accuracy_by_difficulty.get(key).copied().unwrap_or(0.0);
            let _ = write!(
                h,
                r#"<div class="bar-row"><div class="lbl">{label}</div>
<div class="bar-track"><div class="bar-fill" style="--w:{w:.0}%;background:{col}"></div></div>
<div class="num">{w:.0}%</div></div>"#,
                label = label,
                w = acc * 100.0,
                col = score_colour(acc * 100.0),
            );
        }
        let _ = write!(h, "</div>");
    }
    let _ = write!(h, "</div>");

    if !id.evidence.is_empty() {
        let _ = write!(
            h,
            r#"<div class="card" style="grid-column:1/-1"><h3>身份证据</h3><ul class="ev">"#
        );
        for e in &id.evidence {
            let _ = write!(h, "<li>{}</li>", esc(e));
        }
        let _ = write!(h, "</ul></div>");
    }
    let _ = write!(h, "</div></section>");
}

fn billing_panel(h: &mut String, rep: &Report) {
    let b = &rep.billing;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>计量与计费</h2>
<span class="hint">计费数字可信吗，有没有多收钱</span></div><div class="grid g2">"#
    );

    // Billed vs honest bars, scaled to whichever is larger.
    let max = b.billed_input.max(b.honest_input).max(1) as f64;
    let _ = write!(
        h,
        r#"<div class="card"><h3>输入 token：计费 vs 独立重算</h3><div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">端点计费</span><span class="n">{billed}</span></div>
<div class="bar-track"><div class="bar-fill" style="--w:{bw:.1}%;background:{bc}"></div></div></div>
<div class="cmp-row"><div class="top"><span class="k">独立重算</span><span class="n">{honest}</span></div>
<div class="bar-track"><div class="bar-fill" style="--w:{hw:.1}%;background:var(--accent)"></div></div></div>
</div>
<div class="ratio-note {rcls}">{rtext}</div>
<div style="margin-top:11px;font-size:12.5px;color:var(--muted)">对照方式：{method}</div>
</div>"#,
        billed = b.billed_input,
        honest = b.honest_input,
        bw = b.billed_input as f64 / max * 100.0,
        hw = b.honest_input as f64 / max * 100.0,
        bc = if b.input_ratio > 1.15 {
            "var(--fail)"
        } else {
            "var(--pass)"
        },
        rcls = if b.input_ratio > 1.5 {
            "ratio-bad"
        } else if b.input_ratio > 1.15 {
            "ratio-hi"
        } else {
            "ratio-ok"
        },
        rtext = if b.honest_input == 0 {
            "没有可对照的独立计数，无法给出倍率".to_string()
        } else if b.input_ratio > 1.15 {
            format!(
                "计费倍率 {:.2}×——比实际多算了约 {:.0}%",
                b.input_ratio,
                (b.input_ratio - 1.0) * 100.0
            )
        } else {
            format!("计费倍率 {:.2}×，在正常范围内", b.input_ratio)
        },
        method = esc(&b.method),
    );

    // Cost, or an explicit statement that no price was applied.
    let _ = write!(h, r#"<div class="card"><h3>成本折算</h3>"#);
    if b.billed_cost_usd > 0.0 {
        let _ = write!(
            h,
            r#"<div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">按计费数字</span><span class="n">${bc:.6}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">按实测数字</span><span class="n">${hc:.6}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">差额</span><span class="n">${d:.6}</span></div></div>
</div>
<div style="margin-top:11px;font-size:12.5px;color:var(--muted)">{src}。金额只是本次几个探测请求的量级，用于说明倍率，不是月账单预估。</div>"#,
            bc = b.billed_cost_usd,
            hc = b.honest_cost_usd,
            d = b.billed_cost_usd - b.honest_cost_usd,
            src = esc(&b.pricing_source),
        );
    } else {
        let _ = write!(
            h,
            r#"<div class="note warnbox" style="border-radius:7px">{}</div>"#,
            esc(&b.pricing_source)
        );
    }
    if !b.anomalies.is_empty() {
        let _ = write!(
            h,
            r#"<h3 style="margin-top:16px">计量异常</h3><ul class="ev">"#
        );
        for a in &b.anomalies {
            let _ = write!(h, "<li>{}</li>", esc(a));
        }
        let _ = write!(h, "</ul>");
    }
    let _ = write!(h, "</div></div></section>");
}

fn perf_panel(h: &mut String, rep: &Report) {
    let p = &rep.perf;
    if p.samples == 0 {
        return;
    }
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>性能</h2>
<span class="hint">首字延迟、吞吐与抖动，也是身份旁证</span></div><div class="grid g2">"#
    );

    dist_chart(h, "首字延迟分布（ms）", &p.ttft_ms, "ms");
    dist_chart(h, "端到端延迟分布（ms）", &p.latency_ms, "ms");

    let _ = write!(
        h,
        r#"<div class="card" style="grid-column:1/-1"><h3>汇总</h3><div class="grid g4" style="gap:11px">"#
    );
    for (label, value) in [
        ("TTFT P50", format!("{:.0} ms", p.ttft_p50)),
        ("TTFT P95", format!("{:.0} ms", p.ttft_p95)),
        ("延迟 P50", format!("{:.0} ms", p.latency_p50)),
        ("延迟 P95", format!("{:.0} ms", p.latency_p95)),
        ("平均吞吐", format!("{:.1} tok/s", p.tps_mean)),
        ("变异系数", format!("{:.2}", p.latency_cv)),
        ("采样数", format!("{}", p.samples)),
    ] {
        let _ = write!(
            h,
            r#"<div><div style="font-size:11.5px;color:var(--muted);font-family:var(--mono)">{}</div>
<div style="font-family:var(--mono);font-size:17px;font-variant-numeric:tabular-nums">{}</div></div>"#,
            esc(label),
            esc(&value)
        );
    }
    if p.latency_cv > 0.5 {
        let _ = write!(
            h,
            r#"</div><div class="ratio-note ratio-hi" style="margin-top:14px">变异系数 {:.2} 偏高：同一端点的耗时分布分散，常见于后端轮询多个供应商或严重超卖。</div>"#,
            p.latency_cv
        );
    } else {
        let _ = write!(h, "</div>");
    }
    let _ = write!(h, "</div></div></section>");
}

fn dist_chart(h: &mut String, title: &str, values: &[f64], unit: &str) {
    if values.is_empty() {
        return;
    }
    let max = values.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let _ = write!(
        h,
        r#"<div class="card"><h3>{}</h3><div class="dist">"#,
        esc(title)
    );
    for (i, v) in values.iter().enumerate() {
        // Floor the height so a genuinely fast sample is still a visible bar
        // rather than an empty column.
        let pct = (v / max * 100.0).max(4.0);
        let _ = write!(
            h,
            r#"<div class="dist-col"><div class="dist-bar" style="--h:{pct:.1}%;animation-delay:{d:.2}s"></div></div>"#,
            pct = pct,
            d = 0.3 + i as f64 * 0.05,
        );
    }
    let _ = write!(
        h,
        r#"</div><div class="dist-axis"><span>最小 {min:.0}{unit}</span><span>{n} 个样本</span><span>最大 {max:.0}{unit}</span></div></div>"#,
        min = values.iter().cloned().fold(f64::INFINITY, f64::min),
        max = max,
        n = values.len(),
        unit = unit,
    );
}

fn channel_panel(h: &mut String, rep: &Report) {
    let c = &rep.channel;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>渠道来源</h2>
<span class="hint">请求到达模型前经过了什么</span></div><div class="card">"#
    );

    let _ = write!(
        h,
        r#"<div class="chain"><span class="hop you">你的请求</span>"#
    );
    let hops: Vec<&String> = if c.all_hops.is_empty() {
        Vec::new()
    } else {
        c.all_hops.iter().collect()
    };
    if hops.is_empty() {
        let _ = write!(
            h,
            r#"<span class="arrow">→</span><span class="hop">{}</span>"#,
            esc(&c.display)
        );
    } else {
        for hop in hops {
            let _ = write!(
                h,
                r#"<span class="arrow">→</span><span class="hop">{}</span>"#,
                esc(hop)
            );
        }
    }
    let _ = write!(
        h,
        r#"<span class="arrow">→</span><span class="hop you">模型</span></div>"#
    );

    let _ = write!(
        h,
        r#"<div class="cmp"><div class="cmp-row"><div class="top">
<span class="k">识别结果</span><span class="n">{label}（{tier} 层信号，置信度 {conf:.0}%）</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">来源归类</span><span class="n">{chan}</span></div></div></div>
<div style="margin-top:11px;font-size:13px;color:var(--ink-2)">{desc}</div>"#,
        label = esc(&c.display),
        tier = c.tier,
        conf = c.confidence * 100.0,
        chan = rep.verdict.channel.label_zh(),
        desc = esc(rep.verdict.channel.desc_zh()),
    );

    if !c.evidence.is_empty() {
        let _ = write!(
            h,
            r#"<h3 style="margin-top:16px">识别依据</h3><ul class="ev">"#
        );
        for e in &c.evidence {
            let _ = write!(h, "<li>{}</li>", esc(e));
        }
        let _ = write!(h, "</ul>");
    }
    let _ = write!(h, "</div></section>");
}

fn probe_table(h: &mut String, rep: &Report) {
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>探针明细</h2>
<span class="hint">{n} 项，点开可看证据与原始响应</span></div>
<div class="tbl-wrap"><table>
<thead><tr><th>状态</th><th>ID</th><th>探针</th><th>结论</th><th style="text-align:right">耗时</th></tr></thead>
<tbody>"#,
        n = rep.results.len()
    );

    for g in Group::ALL {
        let rows = rep.by_group(g);
        if rows.is_empty() {
            continue;
        }
        let _ = write!(
            h,
            r#"<tr class="grp"><td colspan="5">{} — {}</td></tr>"#,
            esc(g.label_zh()),
            esc(g.blurb_zh())
        );
        for r in rows {
            let _ = write!(
                h,
                r#"<tr><td class="st"><span class="tag {cls}">{sym} {slabel}</span></td>
<td class="pid">{id}</td><td class="plabel">{label}{neutral}</td><td class="psum">{summary}"#,
                cls = r.status.css(),
                sym = r.status.symbol(),
                slabel = r.status.label_zh(),
                id = esc(&r.id),
                label = esc(&r.label),
                neutral = if r.neutral {
                    r#"<br><span style="font-weight:400;font-size:11px;color:var(--muted)">仅取证，不计分</span>"#
                } else {
                    ""
                },
                summary = esc(&r.summary),
            );

            let has_detail =
                !r.findings.is_empty() || r.evidence.is_some() || !r.metrics.is_empty();
            if has_detail {
                let _ = write!(
                    h,
                    r#"<details class="detail"><summary>详情</summary><div class="detail-body">"#
                );
                if !r.findings.is_empty() {
                    let _ = write!(h, "<ul>");
                    for f in &r.findings {
                        let _ = write!(h, "<li>{}</li>", esc(f));
                    }
                    let _ = write!(h, "</ul>");
                }
                if !r.metrics.is_empty() {
                    let _ = write!(h, r#"<div class="kv">"#);
                    for (k, val) in &r.metrics {
                        let s = match val {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        if s.is_empty() {
                            continue;
                        }
                        let _ = write!(
                            h,
                            "<code>{}={}</code>",
                            esc(k),
                            esc(&crate::util::truncate(&s, 60))
                        );
                    }
                    let _ = write!(h, "</div>");
                }
                if let Some(e) = &r.evidence {
                    let _ = write!(h, "<pre>{}</pre>", esc(e));
                }
                let _ = write!(h, "</div></details>");
            }
            let _ = write!(h, r#"</td><td class="pms">{}ms</td></tr>"#, r.duration_ms);
        }
    }
    let _ = write!(h, "</tbody></table></div></section>");
}

fn trace_panel(h: &mut String, rep: &Report) {
    let v = &rep.verdict;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>判定过程</h2>
<span class="hint">结论怎么来的，可以自己复核</span></div><div class="grid g2">
<div class="card"><h3>决策轨迹</h3><ol class="trace">"#
    );
    for t in &v.trace {
        let _ = write!(h, "<li>{}</li>", esc(t));
    }
    let _ = write!(h, "</ol></div>");

    let _ = write!(h, r#"<div class="card"><h3>关键信号</h3>"#);
    if v.signals.is_empty() {
        let _ = write!(
            h,
            r#"<p style="margin:0;color:var(--muted);font-size:13.5px">没有触发任何风险信号。</p>"#
        );
    } else {
        let _ = write!(h, r#"<ul class="ev">"#);
        for s in &v.signals {
            let _ = write!(h, "<li>{}</li>", esc(s));
        }
        let _ = write!(h, "</ul>");
    }

    if !rep.skipped.is_empty() {
        let _ = write!(
            h,
            r#"<h3 style="margin-top:16px">未执行的探针（{n} 项）</h3>
<div class="note warnbox" style="border-radius:7px;margin-bottom:9px;font-size:13px">
<b>未测不等于通过。</b>下列探针没有跑完，相关结论不在本报告的覆盖范围内。</div>
<ul class="ev">"#,
            n = rep.skipped.len()
        );
        for s in &rep.skipped {
            let _ = write!(h, "<li>{}</li>", esc(s));
        }
        let _ = write!(h, "</ul>");
    }
    let _ = write!(h, "</div></div></section>");
}

fn limits_panel(h: &mut String) {
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>能力边界</h2>
<span class="hint">这份报告不能证明什么</span></div>
<div class="note"><b>对诚实供应商的一次误判，代价远高于一次漏检。</b>
本工具在证据不足时一律弃权，而不是猜一个结论。读报告时请一并考虑下面几条限制。</div>
<ul class="limits" style="margin-top:16px">
<li><b>同家族相邻版本不可分。</b>本工具只做到「档位」粒度（旗舰 / 中档 / 轻量），
同档位内的相邻版本（例如同系列的 4.5 与 4.6）在没有分布基线时无法区分，报告只会给出档位结论。</li>
<li><b>中间层会污染身份指纹。</b>注入的 system prompt 与响应后处理都会改变模型的表达风格，
所以协议契约层必须先跑；一旦发现注入或改写，身份结论的置信度已相应下调。</li>
<li><b>无法证明服务端权重就是官方权重。</b>本工具能证明的是「行为与预期一致或不一致」，
不能证明对方部署的是不是原始模型文件。</li>
<li><b>量化版本难以识别。</b>int4 / fp8 量化后的模型与原版在能力上差距较小，
只能通过能力档位给出概率性判断，给不了定论。</li>
<li><b>一次检测只代表此刻。</b>渐进式降级需要持续监测才能发现，建议定期重跑并对比历史报告。</li>
<li><b>估算与权威计数不同。</b>只有端点提供 count_tokens 时计量对照才是权威的；
否则报告使用本地估算，并已在「对照方式」中注明。</li>
</ul></section>"#
    );
}

fn footer(h: &mut String, rep: &Report) {
    let _ = write!(
        h,
        r#"<footer class="wrap">
<p>llm-verify v{ver} · 开始 {start} · 结束 {end} · 共 {reqs} 次请求 · 耗时 {secs:.1}s</p>
<p>目标 {base} · 模型 {model} · 宣称 {claimed} · 协议 {proto} · 深度 {depth}</p>
<p>本报告为自动化黑盒检测结果，仅供参考，不构成对任何供应商的法律指控。</p>
</footer>"#,
        ver = esc(&rep.tool_version),
        start = esc(&rep.started_at),
        end = esc(&rep.finished_at),
        reqs = rep.request_count,
        secs = rep.duration_ms as f64 / 1000.0,
        base = esc(&rep.base_url),
        model = esc(&rep.model),
        claimed = esc(&rep.claimed_model),
        proto = rep.protocol,
        depth = esc(&rep.depth),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Protocol;
    use std::collections::BTreeMap;

    fn sample_report() -> Report {
        let mut results = vec![
            ProbeResult::new("preflight", "连通性预检", Group::Contract)
                .pass("端点可达")
                .took(120),
            ProbeResult::new("model_echo", "model 回显", Group::Contract)
                .fail("请求 a，回显 b")
                .finding("回显不符")
                .metric("requested", "a")
                .evidence("<script>alert(1)</script>")
                .took(90),
        ];
        results.push(
            ProbeResult::new("tps", "吞吐", Group::Perf)
                .neutral()
                .pass("平均 50 tok/s"),
        );
        let mut group_scores = BTreeMap::new();
        group_scores.insert("contract".to_string(), 55.0);
        group_scores.insert("perf".to_string(), 100.0);

        Report {
            tool_version: "0.1.0".into(),
            started_at: "2026-08-13T00:00:00Z".into(),
            finished_at: "2026-08-13T00:01:00Z".into(),
            duration_ms: 60_000,
            host: "api.example.com".into(),
            base_url: "https://api.example.com/v1".into(),
            protocol: Protocol::Anthropic,
            model: "claude-opus-4-5".into(),
            claimed_model: "claude-opus-4-5".into(),
            depth: "balanced".into(),
            request_count: 24,
            results,
            verdict: Verdict {
                authenticity: Authenticity::Suspicious,
                channel: Channel::Proxy,
                score: 57.5,
                confidence: 0.7,
                hard_gate_hits: vec![GateHit {
                    name: "静默 fallback".into(),
                    probe: "invalid_model".into(),
                    reason: "不存在的模型也返回了内容".into(),
                }],
                signals: vec!["model 回显不符".into()],
                trace: vec!["加权分 57.5".into()],
                group_scores,
                coverage_gap: 0.0,
            },
            identity: Identity {
                claimed_model: "claude-opus-4-5".into(),
                claimed_family: Some("anthropic".into()),
                claimed_tier: Some("large".into()),
                observed_family: Some("openai".into()),
                family_confidence: 0.7,
                estimated_tier: Some("small".into()),
                tier_confidence: 0.82,
                tier_severity: 2,
                status: IdentityStatus::FamilyMismatch,
                evidence: vec!["自述指向 openai".into()],
                tier_scores: [("large".to_string(), 0.2), ("small".to_string(), 0.9)].into(),
                accuracy_by_difficulty: [("easy".to_string(), 0.9)].into(),
            },
            billing: BillingAudit {
                method: "count_tokens 端点".into(),
                billed_input: 620,
                honest_input: 12,
                input_ratio: 51.67,
                billed_cost_usd: 0.0093,
                honest_cost_usd: 0.00018,
                cost_ratio: 51.67,
                pricing_source: "内置定价表".into(),
                anomalies: vec!["隐藏 prompt 膨胀".into()],
                ..Default::default()
            },
            channel: ChannelSignature {
                label: "LiteLLM".into(),
                display: "LiteLLM".into(),
                confidence: 1.0,
                tier: 1,
                evidence: vec!["响应头 x-litellm-version".into()],
                all_hops: vec!["LiteLLM".into(), "New-API".into()],
            },
            perf: PerfSummary {
                samples: 3,
                ttft_ms: vec![300.0, 420.0, 510.0],
                latency_ms: vec![1200.0, 1500.0, 1900.0],
                tps: vec![48.0, 52.0],
                ttft_p50: 420.0,
                ttft_p95: 510.0,
                latency_p50: 1500.0,
                latency_p95: 1900.0,
                tps_mean: 50.0,
                latency_cv: 0.22,
            },
            skipped: vec!["缺版本头（missing_version）：仅适用于 Anthropic 协议".into()],
        }
    }

    #[test]
    fn renders_a_self_contained_document() {
        let h = render(&sample_report());
        assert!(h.contains("<title>llm-verify · api.example.com</title>"));
        assert!(h.contains("<style>"));
        // No network fetches of any kind: the file must render offline forever.
        assert!(!h.contains("http://"));
        assert!(!h.contains("<script"));
        assert!(!h.contains("cdn."));
        assert!(!h.contains("@import"));
    }

    #[test]
    fn escapes_probe_evidence_so_a_hostile_response_cannot_inject_markup() {
        // A malicious endpoint controls the response text that lands in the
        // report; it must never become live markup.
        let h = render(&sample_report());
        assert!(h.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!h.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn hard_gates_are_surfaced_prominently() {
        let h = render(&sample_report());
        assert!(h.contains("硬门禁命中 1 项"));
        assert!(h.contains("静默 fallback"));
    }

    #[test]
    fn skipped_probes_carry_the_not_tested_warning() {
        let h = render(&sample_report());
        assert!(h.contains("未测不等于通过"));
        assert!(h.contains("未执行的探针（1 项）"));
    }

    #[test]
    fn limits_section_is_always_present() {
        let h = render(&sample_report());
        assert!(h.contains("能力边界"));
        assert!(h.contains("无法证明服务端权重就是官方权重"));
    }

    #[test]
    fn donut_offset_matches_the_score() {
        let mut rep = sample_report();
        rep.verdict.score = 100.0;
        // A full score draws the whole ring, so the final offset is zero.
        assert!(render(&rep).contains("--final:0.00"));

        rep.verdict.score = 0.0;
        let h = render(&rep);
        let circ = 2.0 * std::f64::consts::PI * 64.0;
        assert!(h.contains(&format!("--final:{circ:.2}")));
    }

    #[test]
    fn out_of_range_scores_do_not_produce_a_broken_ring() {
        let mut rep = sample_report();
        rep.verdict.score = 140.0;
        assert!(render(&rep).contains("--final:0.00"));
        rep.verdict.score = -20.0;
        let h = render(&rep);
        let circ = 2.0 * std::f64::consts::PI * 64.0;
        assert!(h.contains(&format!("--final:{circ:.2}")));
    }

    #[test]
    fn theme_tokens_cover_all_three_viewer_states() {
        // Bare :root, the prefers-color-scheme block, and the explicit stamp.
        assert!(CSS.contains(":root{"));
        assert!(CSS.contains("@media (prefers-color-scheme:dark)"));
        assert!(CSS.contains(r#":root:not([data-theme="light"])"#));
        assert!(CSS.contains(r#":root[data-theme="dark"]"#));
        // A transparent body would borrow the host's background.
        assert!(CSS.contains("body{margin:0;background:var(--ground)"));
    }

    #[test]
    fn animation_is_disabled_under_reduced_motion() {
        assert!(CSS.contains("@media (prefers-reduced-motion:reduce)"));
        assert!(CSS.contains("stroke-dashoffset:var(--final) !important"));
    }

    #[test]
    fn empty_perf_data_omits_the_panel_rather_than_dividing_by_zero() {
        let mut rep = sample_report();
        rep.perf = PerfSummary::default();
        let h = render(&rep);
        assert!(!h.contains("首字延迟分布"));
        assert!(
            h.contains("能力边界"),
            "the rest of the report still renders"
        );
    }

    #[test]
    fn zero_honest_tokens_does_not_claim_a_ratio() {
        let mut rep = sample_report();
        rep.billing.honest_input = 0;
        rep.billing.billed_input = 0;
        let h = render(&rep);
        assert!(h.contains("没有可对照的独立计数"));
    }

    #[test]
    fn neutral_probes_are_marked_as_non_scoring() {
        let h = render(&sample_report());
        assert!(h.contains("仅取证，不计分"));
    }

    #[test]
    fn every_group_with_a_score_gets_a_bar() {
        let h = render(&sample_report());
        assert!(h.contains(Group::Contract.label_zh()));
        assert!(h.contains(Group::Perf.label_zh()));
    }
}
