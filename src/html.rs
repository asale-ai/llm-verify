// SPDX-License-Identifier: Apache-2.0
//! Self-contained HTML report.
//!
//! Everything is inlined — no CDN, no fonts, no scripts fetched at view time —
//! so the file can be mailed to a provider as evidence and still render years
//! later. Charts are hand-built SVG with CSS animation rather than a charting
//! library, which keeps the artefact under a few hundred kilobytes.

use crate::i18n::Lang;
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
    let l = rep.lang;

    let _ = write!(
        h,
        r#"<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="report-language" content="{lang}">
<title>llm-verify · {host}</title>
<style>{CSS}</style>
"#,
        lang = l.html_lang(),
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
    limits_panel(&mut h, l);
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
    let l = rep.lang;
    let _ = write!(
        h,
        r#"<header class="mast"><div class="wrap">
<div class="brand"><span class="name">llm-verify</span><span class="ver">v{ver} · {started}</span></div>
<div class="target">
<div><b>{k_ep}</b><span>{base}</span></div>
<div><b>{k_model}</b><span>{model}</span></div>
<div><b>{k_proto}</b><span>{proto}</span></div>
<div><b>{k_depth}</b><span>{depth}</span></div>
</div></div></header>"#,
        ver = esc(&rep.tool_version),
        started = esc(&rep.started_at),
        k_ep = ts!(l, "Endpoint", "端点"),
        k_model = ts!(l, "Model", "模型"),
        k_proto = ts!(l, "Protocol", "协议"),
        k_depth = ts!(l, "Depth", "深度"),
        base = esc(&rep.base_url),
        model = esc(&rep.model),
        proto = rep.protocol,
        depth = esc(&rep.depth),
    );
}

fn hero(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let v = &rep.verdict;
    // r=64 ring on a 168px box; circumference drives the draw animation.
    const R: f64 = 64.0;
    let circ = 2.0 * std::f64::consts::PI * R;
    let final_offset = circ * (1.0 - (v.score / 100.0).clamp(0.0, 1.0));

    let _ = write!(
        h,
        r#"<div class="wrap"><div class="hero">
<div class="donut">
<svg width="168" height="168" viewBox="0 0 168 168" role="img" aria-label="{aria}">
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
<span class="pill"><b>{k_origin}</b> {chan}</span>
<span class="pill"><b>{k_ident}</b> {ident}</span>
<span class="pill"><b>{k_conf}</b> {conf:.0}%</span>
<span class="pill"><b>{k_probes}</b> {probes}</span>
<span class="pill"><b>{k_reqs}</b> {reqs}</span>
</div>
</div></div></div>"#,
        aria = esc(&t!(
            l,
            "Overall score {score:.1} out of 100",
            "综合评分 {score:.1} 分",
            score = v.score
        )),
        r = R,
        circ = circ,
        fin = final_offset,
        col = score_colour(v.score),
        score = v.score,
        vcss = v.authenticity.css(),
        vlabel = esc(v.authenticity.label(l)),
        host = esc(&rep.host),
        vdesc = esc(v.authenticity.desc(l)),
        k_origin = ts!(l, "Origin", "来源"),
        chan = esc(v.channel.label(l)),
        k_ident = ts!(l, "Identity", "身份"),
        ident = esc(rep.identity.status.label(l)),
        k_conf = ts!(l, "Confidence", "置信度"),
        conf = v.confidence * 100.0,
        k_probes = ts!(l, "Probes", "探针"),
        probes = esc(&t!(
            l,
            "{} passed / {} warned / {} failed",
            "{} 通过 / {} 警告 / {} 失败",
            rep.count(Status::Pass),
            rep.count(Status::Warn),
            rep.count(Status::Fail)
        )),
        k_reqs = ts!(l, "Requests", "请求"),
        reqs = esc(&t!(
            l,
            "{} · {:.1}s",
            "{} 次 · {:.1}s",
            rep.request_count,
            rep.duration_ms as f64 / 1000.0
        )),
    );
}

fn gates(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let gates = &rep.verdict.hard_gate_hits;
    if gates.is_empty() {
        return;
    }
    let _ = write!(
        h,
        r#"<div class="gates rise"><header>⛔ {}</header><ul>"#,
        esc(&t!(
            l,
            "{n} hard gate(s) tripped — no weighted score can excuse these",
            "硬门禁命中 {n} 项 —— 加权分再高也不能忽略",
            n = gates.len()
        ))
    );
    for g in gates {
        let _ = write!(
            h,
            r#"<li><b>{}</b> — {}<br><span style="color:var(--muted);font-size:12.5px">{}</span></li>"#,
            esc(&g.name),
            esc(&g.reason),
            esc(&t!(l, "from probe {}", "来自探针 {}", g.probe))
        );
    }
    let _ = write!(h, "</ul></div>");
}

fn key_stats(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let b = &rep.billing;
    let p = &rep.perf;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="grid g4">"#,
        esc(ts!(l, "Key numbers", "关键指标")),
        esc(ts!(
            l,
            "The state of this endpoint at a glance",
            "一眼看懂这个端点当前的状态"
        ))
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
            format!("{:.2}x", b.input_ratio),
            cls,
            t!(
                l,
                "billed {} / actual {} tokens",
                "计费 {} / 实测 {} token",
                b.billed_input,
                b.honest_input
            ),
        )
    } else {
        (
            "—".to_string(),
            "",
            t!(
                l,
                "no independent count to compare against",
                "没有可对照的独立计数"
            ),
        )
    };
    stat_tile(
        h,
        ts!(l, "Billing ratio", "计费倍率"),
        &ratio_txt,
        ratio_cls,
        &ratio_sub,
    );

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
            t!(
                l,
                "P95 {:.0}ms · {} samples",
                "P95 {:.0}ms · {} 个样本",
                p.ttft_p95,
                p.ttft_ms.len()
            ),
        )
    } else {
        ("—".into(), "", t!(l, "no streamed samples", "没有流式样本"))
    };
    stat_tile(
        h,
        ts!(l, "TTFT P50", "首字延迟 P50"),
        &ttft.0,
        ttft.1,
        &ttft.2,
    );

    let tps = if p.tps_mean > 0.0 {
        (
            format!("{:.0}", p.tps_mean),
            "",
            format!(
                "tok/s · {}",
                match crate::probes::perf::tier_band(p.tps_mean) {
                    "large" => t!(l, "large-model speed", "偏大模型速度"),
                    "small" => t!(l, "small-model speed", "偏小模型速度"),
                    _ => t!(l, "in between", "中间档"),
                }
            ),
        )
    } else {
        ("—".into(), "", "tok/s".into())
    };
    stat_tile(h, ts!(l, "Throughput", "生成吞吐"), &tps.0, tps.1, &tps.2);

    let cov = (1.0 - rep.verdict.coverage_gap) * 100.0;
    stat_tile(
        h,
        ts!(l, "Probe coverage", "探针覆盖率"),
        &format!("{cov:.0}%"),
        stat_class(cov),
        &t!(l, "{} did not run", "{} 项未执行", rep.skipped.len()),
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
    let l = rep.lang;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="card"><div class="bars">"#,
        esc(ts!(l, "Scores by group", "分组得分")),
        esc(ts!(
            l,
            "Locate which layer the problems are in",
            "问题出在哪一层，一眼定位"
        ))
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
            hint = esc(g.blurb(l)),
            label = esc(g.label(l)),
            pct = pct,
            col = score_colour(*pct),
        );
    }
    let _ = write!(h, "</div></div></section>");
}

fn identity_panel(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let id = &rep.identity;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="grid g2">"#,
        esc(ts!(l, "Model identity", "模型身份")),
        esc(ts!(
            l,
            "Is the model behind this the one that was sold?",
            "背后跑的是不是它声称的那个模型"
        ))
    );

    // Claim vs observation.
    let _ = write!(
        h,
        r#"<div class="card"><h3>{title}</h3>
<div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">{k1}</span><span class="n">{claimed}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k2}</span><span class="n">{cf} / {ct}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k3}</span><span class="n">{of}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k4}</span><span class="n">{et}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k5}</span><span class="n">{basis}</span></div></div>
</div>
<div class="ratio-note {ncls}" style="margin-top:14px">{istatus}</div>{thin}
</div>"#,
        title = esc(ts!(l, "Claimed vs measured", "宣称 vs 实测")),
        k1 = esc(ts!(l, "Claimed model", "宣称模型")),
        claimed = esc(&id.claimed_model),
        k2 = esc(ts!(l, "Claimed family / tier", "宣称家族 / 档位")),
        cf = esc(id
            .claimed_family
            .as_deref()
            .unwrap_or(ts!(l, "unknown", "未知"))),
        ct = esc(id
            .claimed_tier
            .as_deref()
            .unwrap_or(ts!(l, "unknown", "未知"))),
        k3 = esc(ts!(l, "Measured family", "实测家族")),
        of = esc(&t!(
            l,
            "{} (confidence {:.0}%)",
            "{}（信心 {:.0}%）",
            id.observed_family
                .as_deref()
                .unwrap_or(ts!(l, "unidentified", "未识别")),
            id.family_confidence * 100.0
        )),
        k4 = esc(ts!(l, "Measured tier", "实测档位")),
        et = esc(&t!(
            l,
            "{} (fit {:.2})",
            "{}（拟合 {:.2}）",
            id.estimated_tier
                .as_deref()
                .unwrap_or(ts!(l, "not measured", "未测出")),
            id.tier_confidence
        )),
        k5 = esc(ts!(l, "Tier verdict rests on", "档位判定依据")),
        basis = esc(&t!(
            l,
            "{} questions · beat runner-up by {:.3}",
            "{} 道能力题 · 领先次优 {:.3}",
            id.tier_questions,
            id.tier_margin
        )),
        ncls = id.status.css(),
        istatus = esc(id.status.label(l)),
        thin = if id.tier_questions > 0 && id.tier_questions < 9 {
            format!(
                r#"<div style="margin-top:10px;font-size:12.5px;color:var(--muted)">{}</div>"#,
                t!(
                    l,
                    "The tier call rests on a thin sample; adjacent tiers can swing \
                     on a single question. Re-run with <code>--depth forensic</code> \
                     for a verdict that holds up.",
                    "档位判定的样本量偏小，相邻档位之间可能因单题得失而摆动。需要更有把握的结论请用 <code>--depth forensic</code> 重跑。"
                )
            )
        } else {
            String::new()
        },
    );

    // Capability profile.
    let _ = write!(
        h,
        r#"<div class="card"><h3>{}</h3><div class="bars">"#,
        esc(ts!(l, "Tier hypothesis fit", "能力档位拟合"))
    );
    for (tier, label) in [
        ("large", ts!(l, "flagship", "旗舰档")),
        ("mid", ts!(l, "mid", "中档")),
        ("small", ts!(l, "light", "轻量档")),
    ] {
        let score = id.tier_scores.get(tier).copied().unwrap_or(0.0);
        let is_best = id.estimated_tier.as_deref() == Some(tier);
        let _ = write!(
            h,
            r#"<div class="bar-row"><div class="lbl">{label}{mark}</div>
<div class="bar-track"><div class="bar-fill" style="--w:{w:.1}%;background:{col}"></div></div>
<div class="num">{score:.2}</div></div>"#,
            label = esc(label),
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
            r#"<h3 style="margin-top:16px">{}</h3><div class="bars">"#,
            esc(ts!(l, "Accuracy by difficulty", "分难度正确率"))
        );
        for (key, label) in [
            ("easy", ts!(l, "easy", "简单")),
            ("medium", ts!(l, "medium", "中等")),
            ("hard", ts!(l, "hard", "困难")),
        ] {
            let acc = id.accuracy_by_difficulty.get(key).copied().unwrap_or(0.0);
            let _ = write!(
                h,
                r#"<div class="bar-row"><div class="lbl">{label}</div>
<div class="bar-track"><div class="bar-fill" style="--w:{w:.0}%;background:{col}"></div></div>
<div class="num">{w:.0}%</div></div>"#,
                label = esc(label),
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
            r#"<div class="card" style="grid-column:1/-1"><h3>{}</h3><ul class="ev">"#,
            esc(ts!(l, "Identity evidence", "身份证据"))
        );
        for e in &id.evidence {
            let _ = write!(h, "<li>{}</li>", esc(e));
        }
        let _ = write!(h, "</ul></div>");
    }
    let _ = write!(h, "</div></section>");
}

fn billing_panel(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let b = &rep.billing;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="grid g2">"#,
        esc(ts!(l, "Metering & billing", "计量与计费")),
        esc(ts!(
            l,
            "Are the token counts honest, or are you overcharged?",
            "计费数字可信吗，有没有多收钱"
        ))
    );

    // Billed vs honest bars, scaled to whichever is larger.
    let max = b.billed_input.max(b.honest_input).max(1) as f64;
    let _ = write!(
        h,
        r#"<div class="card"><h3>{title}</h3><div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">{k1}</span><span class="n">{billed}</span></div>
<div class="bar-track"><div class="bar-fill" style="--w:{bw:.1}%;background:{bc}"></div></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k2}</span><span class="n">{honest}</span></div>
<div class="bar-track"><div class="bar-fill" style="--w:{hw:.1}%;background:var(--accent)"></div></div></div>
</div>
<div class="ratio-note {rcls}">{rtext}</div>
<div style="margin-top:11px;font-size:12.5px;color:var(--muted)">{method}</div>
</div>"#,
        title = esc(ts!(
            l,
            "Input tokens: billed vs independent recount",
            "输入 token：计费 vs 独立重算"
        )),
        k1 = esc(ts!(l, "Endpoint billed", "端点计费")),
        billed = b.billed_input,
        k2 = esc(ts!(l, "Independent recount", "独立重算")),
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
        rtext = esc(&if b.honest_input == 0 {
            t!(
                l,
                "No independent count to compare against, so no ratio can be given",
                "没有可对照的独立计数，无法给出倍率"
            )
        } else if b.input_ratio > 1.15 {
            t!(
                l,
                "Billing ratio {:.2}x — roughly {:.0}% more than actual",
                "计费倍率 {:.2}×——比实际多算了约 {:.0}%",
                b.input_ratio,
                (b.input_ratio - 1.0) * 100.0
            )
        } else {
            t!(
                l,
                "Billing ratio {:.2}x, within the normal range",
                "计费倍率 {:.2}×，在正常范围内",
                b.input_ratio
            )
        }),
        method = esc(&t!(l, "Compared against: {}", "对照方式：{}", b.method)),
    );

    // Cost, or an explicit statement that no price was applied.
    let _ = write!(
        h,
        r#"<div class="card"><h3>{}</h3>"#,
        esc(ts!(l, "Cost", "成本折算"))
    );
    if b.billed_cost_usd > 0.0 {
        let _ = write!(
            h,
            r#"<div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">{k1}</span><span class="n">${bc:.6}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k2}</span><span class="n">${hc:.6}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k3}</span><span class="n">${d:.6}</span></div></div>
</div>
<div style="margin-top:11px;font-size:12.5px;color:var(--muted)">{note}</div>"#,
            k1 = esc(ts!(l, "At the billed counts", "按计费数字")),
            bc = b.billed_cost_usd,
            k2 = esc(ts!(l, "At the measured counts", "按实测数字")),
            hc = b.honest_cost_usd,
            k3 = esc(ts!(l, "Difference", "差额")),
            d = b.billed_cost_usd - b.honest_cost_usd,
            note = esc(&t!(
                l,
                "{}. These figures cover only this run's few probe requests — they \
                 illustrate the ratio, they are not a monthly estimate.",
                "{}。金额只是本次几个探测请求的量级，用于说明倍率，不是月账单预估。",
                b.pricing_source
            )),
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
            r#"<h3 style="margin-top:16px">{}</h3><ul class="ev">"#,
            esc(ts!(l, "Metering anomalies", "计量异常"))
        );
        for a in &b.anomalies {
            let _ = write!(h, "<li>{}</li>", esc(a));
        }
        let _ = write!(h, "</ul>");
    }
    let _ = write!(h, "</div></div></section>");
}

fn perf_panel(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let p = &rep.perf;
    if p.samples == 0 {
        return;
    }
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="grid g2">"#,
        esc(ts!(l, "Performance", "性能")),
        esc(ts!(
            l,
            "Latency, throughput and jitter — also identity evidence",
            "首字延迟、吞吐与抖动，也是身份旁证"
        ))
    );

    dist_chart(
        h,
        ts!(l, "Time to first token (ms)", "首字延迟分布（ms）"),
        &p.ttft_ms,
        "ms",
        l,
    );
    dist_chart(
        h,
        ts!(l, "End-to-end latency (ms)", "端到端延迟分布（ms）"),
        &p.latency_ms,
        "ms",
        l,
    );

    let _ = write!(
        h,
        r#"<div class="card" style="grid-column:1/-1"><h3>{}</h3><div class="grid g4" style="gap:11px">"#,
        esc(ts!(l, "Summary", "汇总"))
    );
    for (label, value) in [
        ("TTFT P50".to_string(), format!("{:.0} ms", p.ttft_p50)),
        ("TTFT P95".to_string(), format!("{:.0} ms", p.ttft_p95)),
        (
            t!(l, "Latency P50", "延迟 P50"),
            format!("{:.0} ms", p.latency_p50),
        ),
        (
            t!(l, "Latency P95", "延迟 P95"),
            format!("{:.0} ms", p.latency_p95),
        ),
        (
            t!(l, "Mean throughput", "平均吞吐"),
            format!("{:.1} tok/s", p.tps_mean),
        ),
        (
            t!(l, "Coefficient of variation", "变异系数"),
            format!("{:.2}", p.latency_cv),
        ),
        (t!(l, "Samples", "采样数"), format!("{}", p.samples)),
    ] {
        let _ = write!(
            h,
            r#"<div><div style="font-size:11.5px;color:var(--muted);font-family:var(--mono)">{}</div>
<div style="font-family:var(--mono);font-size:17px;font-variant-numeric:tabular-nums">{}</div></div>"#,
            esc(&label),
            esc(&value)
        );
    }
    if p.latency_cv > 0.5 {
        let _ = write!(
            h,
            r#"</div><div class="ratio-note ratio-hi" style="margin-top:14px">{}</div>"#,
            esc(&t!(
                l,
                "A coefficient of variation of {:.2} is high: a scattered latency \
                 distribution on one endpoint is typical of round-robin across \
                 several providers, or heavy oversubscription.",
                "变异系数 {:.2} 偏高：同一端点的耗时分布分散，常见于后端轮询多个供应商或严重超卖。",
                p.latency_cv
            ))
        );
    } else {
        let _ = write!(h, "</div>");
    }
    let _ = write!(h, "</div></div></section>");
}

fn dist_chart(h: &mut String, title: &str, values: &[f64], unit: &str, l: Lang) {
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
        r#"</div><div class="dist-axis"><span>{lo}</span><span>{n}</span><span>{hi}</span></div></div>"#,
        lo = esc(&t!(
            l,
            "min {:.0}{unit}",
            "最小 {:.0}{unit}",
            values.iter().cloned().fold(f64::INFINITY, f64::min)
        )),
        n = esc(&t!(l, "{} samples", "{} 个样本", values.len())),
        hi = esc(&t!(l, "max {max:.0}{unit}", "最大 {max:.0}{unit}")),
    );
}

fn channel_panel(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let c = &rep.channel;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="card">"#,
        esc(ts!(l, "Channel provenance", "渠道来源")),
        esc(ts!(
            l,
            "What the request passes through before it reaches the model",
            "请求到达模型前经过了什么"
        ))
    );

    let _ = write!(
        h,
        r#"<div class="chain"><span class="hop you">{}</span>"#,
        esc(ts!(l, "your request", "你的请求"))
    );
    if c.all_hops.is_empty() {
        let _ = write!(
            h,
            r#"<span class="arrow">→</span><span class="hop">{}</span>"#,
            esc(&c.display)
        );
    } else {
        for hop in &c.all_hops {
            let _ = write!(
                h,
                r#"<span class="arrow">→</span><span class="hop">{}</span>"#,
                esc(hop)
            );
        }
    }
    let _ = write!(
        h,
        r#"<span class="arrow">→</span><span class="hop you">{}</span></div>"#,
        esc(ts!(l, "model", "模型"))
    );

    let _ = write!(
        h,
        r#"<div class="cmp"><div class="cmp-row"><div class="top">
<span class="k">{k1}</span><span class="n">{ident}</span></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k2}</span><span class="n">{chan}</span></div></div></div>
<div style="margin-top:11px;font-size:13px;color:var(--ink-2)">{desc}</div>"#,
        k1 = esc(ts!(l, "Classified as", "识别结果")),
        ident = esc(&t!(
            l,
            "{} (tier-{} signal, {:.0}% confidence)",
            "{}（{} 层信号，置信度 {:.0}%）",
            c.display,
            c.tier,
            c.confidence * 100.0
        )),
        k2 = esc(ts!(l, "Origin category", "来源归类")),
        chan = esc(rep.verdict.channel.label(l)),
        desc = esc(rep.verdict.channel.desc(l)),
    );

    if !c.evidence.is_empty() {
        let _ = write!(
            h,
            r#"<h3 style="margin-top:16px">{}</h3><ul class="ev">"#,
            esc(ts!(l, "Evidence", "识别依据"))
        );
        for e in &c.evidence {
            let _ = write!(h, "<li>{}</li>", esc(e));
        }
        let _ = write!(h, "</ul>");
    }
    let _ = write!(h, "</div></section>");
}

fn probe_table(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{title}</h2>
<span class="hint">{hint}</span></div>
<div class="tbl-wrap"><table>
<thead><tr><th>{c1}</th><th>ID</th><th>{c2}</th><th>{c3}</th><th style="text-align:right">{c4}</th></tr></thead>
<tbody>"#,
        title = esc(ts!(l, "Probe detail", "探针明细")),
        hint = esc(&t!(
            l,
            "{n} probes; expand any row for evidence and the raw response",
            "{n} 项，点开可看证据与原始响应",
            n = rep.results.len()
        )),
        c1 = esc(ts!(l, "Status", "状态")),
        c2 = esc(ts!(l, "Probe", "探针")),
        c3 = esc(ts!(l, "Conclusion", "结论")),
        c4 = esc(ts!(l, "Took", "耗时")),
    );

    for g in Group::ALL {
        let rows = rep.by_group(g);
        if rows.is_empty() {
            continue;
        }
        let _ = write!(
            h,
            r#"<tr class="grp"><td colspan="5">{} — {}</td></tr>"#,
            esc(g.label(l)),
            esc(g.blurb(l))
        );
        for r in rows {
            let _ = write!(
                h,
                r#"<tr><td class="st"><span class="tag {cls}">{sym} {slabel}</span></td>
<td class="pid">{id}</td><td class="plabel">{label}{neutral}</td><td class="psum">{summary}"#,
                cls = r.status.css(),
                sym = r.status.symbol(),
                slabel = esc(r.status.label(l)),
                id = esc(&r.id),
                label = esc(&r.label),
                neutral = if r.neutral {
                    format!(
                        r#"<br><span style="font-weight:400;font-size:11px;color:var(--muted)">{}</span>"#,
                        esc(ts!(l, "evidence only, not scored", "仅取证，不计分"))
                    )
                } else {
                    String::new()
                },
                summary = esc(&r.summary),
            );

            let has_detail =
                !r.findings.is_empty() || r.evidence.is_some() || !r.metrics.is_empty();
            if has_detail {
                let _ = write!(
                    h,
                    r#"<details class="detail"><summary>{}</summary><div class="detail-body">"#,
                    esc(ts!(l, "detail", "详情"))
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
    let l = rep.lang;
    let v = &rep.verdict;
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div><div class="grid g2">
<div class="card"><h3>{}</h3><ol class="trace">"#,
        esc(ts!(l, "How the verdict was reached", "判定过程")),
        esc(ts!(
            l,
            "Every step, so you can check it yourself",
            "结论怎么来的，可以自己复核"
        )),
        esc(ts!(l, "Decision trace", "决策轨迹"))
    );
    for t in &v.trace {
        let _ = write!(h, "<li>{}</li>", esc(t));
    }
    let _ = write!(h, "</ol></div>");

    let _ = write!(
        h,
        r#"<div class="card"><h3>{}</h3>"#,
        esc(ts!(l, "Key signals", "关键信号"))
    );
    if v.signals.is_empty() {
        let _ = write!(
            h,
            r#"<p style="margin:0;color:var(--muted);font-size:13.5px">{}</p>"#,
            esc(ts!(
                l,
                "No risk signals were triggered.",
                "没有触发任何风险信号。"
            ))
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
            r#"<h3 style="margin-top:16px">{title}</h3>
<div class="note warnbox" style="border-radius:7px;margin-bottom:9px;font-size:13px">
<b>{warn}</b> {body}</div>
<ul class="ev">"#,
            title = esc(&t!(
                l,
                "Probes that did not run ({n})",
                "未执行的探针（{n} 项）",
                n = rep.skipped.len()
            )),
            warn = esc(ts!(
                l,
                "Not tested is not the same as passed.",
                "未测不等于通过。"
            )),
            body = esc(ts!(
                l,
                "The probes below did not complete, so the conclusions they would \
                 have supported are outside this report's coverage.",
                "下列探针没有跑完，相关结论不在本报告的覆盖范围内。"
            )),
        );
        for s in &rep.skipped {
            let _ = write!(h, "<li>{}</li>", esc(s));
        }
        let _ = write!(h, "</ul>");
    }
    let _ = write!(h, "</div></div></section>");
}

fn limits_panel(h: &mut String, l: Lang) {
    let _ = write!(
        h,
        r#"<section class="rise"><div class="sec-head"><h2>{title}</h2>
<span class="hint">{hint}</span></div>
<div class="note"><b>{lead}</b> {lead2}</div>
<ul class="limits" style="margin-top:16px">"#,
        title = esc(ts!(l, "What this cannot prove", "能力边界")),
        hint = esc(ts!(l, "The limits of this report", "这份报告不能证明什么")),
        lead = esc(ts!(
            l,
            "One false accusation against an honest provider costs far more than one miss.",
            "对诚实供应商的一次误判，代价远高于一次漏检。"
        )),
        lead2 = esc(ts!(
            l,
            "This tool abstains when the evidence is thin rather than guessing. \
             Please read the conclusions alongside the limits below.",
            "本工具在证据不足时一律弃权，而不是猜一个结论。读报告时请一并考虑下面几条限制。"
        )),
    );

    for (head, body) in [
        (
            ts!(l, "Adjacent versions are indistinguishable.", "同家族相邻版本不可分。"),
            ts!(
                l,
                "This tool resolves to tier granularity only (flagship / mid / light). \
                 Adjacent versions inside one tier — say 4.5 and 4.6 of the same line — \
                 cannot be separated without distribution baselines, so the report \
                 stops at the tier.",
                "本工具只做到「档位」粒度（旗舰 / 中档 / 轻量），同档位内的相邻版本（例如同系列的 4.5 与 4.6）在没有分布基线时无法区分，报告只会给出档位结论。"
            ),
        ),
        (
            ts!(l, "The tier call depends on sampling.", "档位判定依赖采样。"),
            ts!(
                l,
                "Adjacent tiers can still swing on a single question. The tool abstains \
                 when the margin is narrow, which costs it some real downgrades. \
                 Use <code>--depth forensic</code> when the answer has to hold up.",
                "相邻档位之间仍可能因单题得失而摆动。工具在差距不明显时会主动弃权，代价是可能漏掉真实的降级。需要拿得出手的结论请用 <code>--depth forensic</code>。"
            ),
        ),
        (
            ts!(l, "Middle layers contaminate identity fingerprints.", "中间层会污染身份指纹。"),
            ts!(
                l,
                "An injected system prompt and any response post-processing both change \
                 how a model expresses itself, which is why the contract layer runs \
                 first — where injection or rewriting is found, identity confidence has \
                 already been reduced accordingly.",
                "注入的 system prompt 与响应后处理都会改变模型的表达风格，所以协议契约层必须先跑；一旦发现注入或改写，身份结论的置信度已相应下调。"
            ),
        ),
        (
            ts!(l, "Server-side weights cannot be proven.", "无法证明服务端权重就是官方权重。"),
            ts!(
                l,
                "What this tool can show is whether behaviour matches expectations. It \
                 cannot show which model file the other side actually deployed.",
                "本工具能证明的是「行为与预期一致或不一致」，不能证明对方部署的是不是原始模型文件。"
            ),
        ),
        (
            ts!(l, "Quantised builds are hard to spot.", "量化版本难以识别。"),
            ts!(
                l,
                "int4 and fp8 builds sit close to the original in capability, so only a \
                 probabilistic read from the tier estimate is possible — never a verdict.",
                "int4 / fp8 量化后的模型与原版在能力上差距较小，只能通过能力档位给出概率性判断，给不了定论。"
            ),
        ),
        (
            ts!(l, "One run describes one moment.", "一次检测只代表此刻。"),
            ts!(
                l,
                "Gradual degradation only shows up under continuous monitoring. Re-run \
                 periodically and compare against earlier reports.",
                "渐进式降级需要持续监测才能发现，建议定期重跑并对比历史报告。"
            ),
        ),
        (
            ts!(l, "An estimate is not an authoritative count.", "估算与权威计数不同。"),
            ts!(
                l,
                "The metering comparison is only authoritative where the endpoint offers \
                 count_tokens. Otherwise the report uses a local estimate, and says so \
                 under \"compared against\".",
                "只有端点提供 count_tokens 时计量对照才是权威的；否则报告使用本地估算，并已在「对照方式」中注明。"
            ),
        ),
    ] {
        let _ = write!(h, "<li><b>{}</b> {}</li>", esc(head), body);
    }
    let _ = write!(h, "</ul></section>");
}

fn footer(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let _ = write!(
        h,
        r#"<footer class="wrap">
<p>{line1}</p>
<p>{line2}</p>
<p>{line3}</p>
</footer>"#,
        line1 = esc(&t!(
            l,
            "llm-verify v{ver} · started {start} · finished {end} · {reqs} requests · {secs:.1}s",
            "llm-verify v{ver} · 开始 {start} · 结束 {end} · 共 {reqs} 次请求 · 耗时 {secs:.1}s",
            ver = rep.tool_version,
            start = rep.started_at,
            end = rep.finished_at,
            reqs = rep.request_count,
            secs = rep.duration_ms as f64 / 1000.0
        )),
        line2 = esc(&t!(
            l,
            "target {base} · model {model} · claimed {claimed} · protocol {proto} · depth {depth}",
            "目标 {base} · 模型 {model} · 宣称 {claimed} · 协议 {proto} · 深度 {depth}",
            base = rep.base_url,
            model = rep.model,
            claimed = rep.claimed_model,
            proto = rep.protocol,
            depth = rep.depth
        )),
        line3 = esc(ts!(
            l,
            "This is an automated black-box measurement, offered for reference only. \
             It is not a legal accusation against any provider.",
            "本报告为自动化黑盒检测结果，仅供参考，不构成对任何供应商的法律指控。"
        )),
    );
}
