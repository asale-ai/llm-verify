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
  --ground:#F3F6F7; --ground-2:#E8EDF0;
  --surface:#FFFFFF; --surface-2:#EEF2F5; --surface-3:#DFE6EA;
  --ink:#101619; --ink-2:#3A464D; --muted:#697680;
  --rule:#DCE3E7; --rule-2:#BFCAD1;
  --accent:#0D5A66; --accent-2:#13818F; --accent-wash:#DEEEF0;
  --pass:#1B6E48; --pass-2:#2E9A68;
  --warn:#8C6115; --warn-2:#C08A2A;
  --fail:#A3352B; --fail-2:#CB5A4C;
  --skip:#7A868D; --skip-2:#9AA5AB;
  --pass-bg:#E4F3EA; --warn-bg:#FBF1DE; --fail-bg:#FBE6E3; --skip-bg:#ECEFF1;
  --shadow-1:0 1px 2px rgba(16,26,30,.05);
  --shadow-2:0 1px 2px rgba(16,26,30,.05),0 12px 30px -16px rgba(16,26,30,.22);
  --radius:13px; --radius-sm:9px; --radius-xs:6px;
  --mono:ui-monospace,"SF Mono",SFMono-Regular,"Cascadia Mono",Menlo,Consolas,monospace;
  --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,"PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;
}
@media (prefers-color-scheme:dark){
  :root:not([data-theme="light"]){
    --ground:#0C1013; --ground-2:#10161A;
    --surface:#151B1F; --surface-2:#1C2429; --surface-3:#263036;
    --ink:#E3E9EC; --ink-2:#B0BCC3; --muted:#84919A;
    --rule:#222B31; --rule-2:#35434C;
    --accent:#4EA8B6; --accent-2:#7FC7D2; --accent-wash:#0F2E34;
    --pass:#4FAE81; --pass-2:#79CBA3;
    --warn:#C99A45; --warn-2:#E0B769;
    --fail:#DB7568; --fail-2:#EC9A8F;
    --skip:#78868E; --skip-2:#94A1A8;
    --pass-bg:#12291F; --warn-bg:#2B2213; --fail-bg:#2E1A18; --skip-bg:#1C2328;
    --shadow-1:0 1px 2px rgba(0,0,0,.4);
    --shadow-2:0 1px 2px rgba(0,0,0,.4),0 12px 30px -16px rgba(0,0,0,.75);
  }
}
:root[data-theme="dark"]{
  --ground:#0C1013; --ground-2:#10161A;
  --surface:#151B1F; --surface-2:#1C2429; --surface-3:#263036;
  --ink:#E3E9EC; --ink-2:#B0BCC3; --muted:#84919A;
  --rule:#222B31; --rule-2:#35434C;
  --accent:#4EA8B6; --accent-2:#7FC7D2; --accent-wash:#0F2E34;
  --pass:#4FAE81; --pass-2:#79CBA3;
  --warn:#C99A45; --warn-2:#E0B769;
  --fail:#DB7568; --fail-2:#EC9A8F;
  --skip:#78868E; --skip-2:#94A1A8;
  --pass-bg:#12291F; --warn-bg:#2B2213; --fail-bg:#2E1A18; --skip-bg:#1C2328;
  --shadow-1:0 1px 2px rgba(0,0,0,.4);
  --shadow-2:0 1px 2px rgba(0,0,0,.4),0 12px 30px -16px rgba(0,0,0,.75);
}

*{box-sizing:border-box}
html{scroll-behavior:smooth}
body{margin:0;color:var(--ink);font-family:var(--sans);font-size:15px;line-height:1.7;
     -webkit-font-smoothing:antialiased;
     background:var(--ground);
     background-image:radial-gradient(900px 380px at 78% -8%,var(--accent-wash),transparent 62%);
     background-repeat:no-repeat}
.wrap{max-width:1140px;margin:0 auto;padding:0 26px}
h1,h2,h3{font-family:var(--mono);font-weight:600;letter-spacing:-.012em;margin:0;text-wrap:balance}
p{text-wrap:pretty}
code{font-family:var(--mono)}

/* ── entrance animation ─────────────────────────────── */
.rise{opacity:0;transform:translateY(14px);animation:rise .55s cubic-bezier(.2,.7,.3,1) forwards}
@keyframes rise{to{opacity:1;transform:none}}
main>section:nth-of-type(1){animation-delay:.02s}
main>section:nth-of-type(2){animation-delay:.06s}
main>section:nth-of-type(3){animation-delay:.10s}
main>section:nth-of-type(4){animation-delay:.14s}
main>section:nth-of-type(5){animation-delay:.18s}
main>section:nth-of-type(6){animation-delay:.22s}
main>section:nth-of-type(7){animation-delay:.26s}
main>section:nth-of-type(n+8){animation-delay:.30s}
@media (prefers-reduced-motion:reduce){
  html{scroll-behavior:auto}
  .rise{animation:none;opacity:1;transform:none}
  .donut-ring,.bar-fill,.dist-bar,.tally-seg,.verdict-chip .dot{animation:none !important}
  .donut-ring{stroke-dashoffset:var(--final) !important}
  .bar-fill,.tally-seg{width:var(--w) !important}
  .dist-bar{height:var(--h) !important}
}

/* ── masthead ───────────────────────────────────────── */
.mast{border-bottom:1px solid var(--rule);
      background:linear-gradient(180deg,var(--surface),color-mix(in srgb,var(--surface) 84%,transparent))}
.mast .wrap{padding:26px 26px 22px}
.brand{display:flex;align-items:center;gap:11px;flex-wrap:wrap}
.brand .mark{width:22px;height:22px;border-radius:6px;flex:0 0 auto;
             background:linear-gradient(135deg,var(--accent-2),var(--accent));
             box-shadow:0 0 0 1px color-mix(in srgb,var(--accent) 30%,transparent) inset;
             display:grid;place-content:center;color:#fff;font-family:var(--mono);
             font-size:12px;font-weight:700;line-height:1}
.brand .name{font-family:var(--mono);font-size:15px;font-weight:600;color:var(--ink);letter-spacing:-.02em}
.brand .ver{font-family:var(--mono);font-size:11.5px;color:var(--muted);
            padding:2px 8px;border:1px solid var(--rule);border-radius:999px}
.brand .stamp{margin-left:auto;font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.target{margin-top:16px;display:grid;gap:10px 14px;
        grid-template-columns:repeat(auto-fit,minmax(215px,1fr))}
.target div{background:var(--surface-2);border:1px solid var(--rule);border-radius:var(--radius-xs);
            padding:7px 11px;min-width:0}
.target b{display:block;font-family:var(--mono);font-size:10px;letter-spacing:.12em;
          text-transform:uppercase;color:var(--muted);font-weight:600}
.target span{display:block;font-family:var(--mono);font-size:12.5px;color:var(--ink-2);
             overflow-wrap:anywhere;margin-top:1px}

/* ── section index ──────────────────────────────────── */
.toc{position:sticky;top:0;z-index:30;background:var(--ground);
     border-bottom:1px solid var(--rule);margin-top:30px}
.toc .wrap{display:flex;gap:2px;overflow-x:auto;padding:0 20px;scrollbar-width:thin}
.toc a{font-family:var(--mono);font-size:11.5px;letter-spacing:.02em;color:var(--muted);
       text-decoration:none;padding:12px 11px 10px;white-space:nowrap;
       border-bottom:2px solid transparent;transition:color .15s,border-color .15s}
.toc a:hover{color:var(--accent);border-bottom-color:var(--accent)}

/* ── hero ───────────────────────────────────────────── */
.hero{position:relative;overflow:hidden;margin:28px 0 0;
      display:grid;grid-template-columns:auto 1fr;gap:34px;align-items:center;
      background:var(--surface);border:1px solid var(--rule);border-radius:var(--radius);
      padding:28px 32px;box-shadow:var(--shadow-2)}
.hero:before{content:"";position:absolute;inset:0;pointer-events:none;
             background:radial-gradient(560px 220px at 8% 0%,var(--accent-wash),transparent 70%)}
.hero>*{position:relative}
@media (max-width:760px){.hero{grid-template-columns:1fr;gap:22px;padding:24px 22px}}
.donut{position:relative;width:170px;height:170px;flex:0 0 auto;margin:0 auto}
.donut svg{transform:rotate(-90deg);display:block}
.donut-track{fill:none;stroke:var(--surface-3);stroke-width:12}
.donut-ring{fill:none;stroke-width:12;stroke-linecap:round;
            stroke-dasharray:var(--circ);stroke-dashoffset:var(--circ);
            animation:draw 1.3s cubic-bezier(.25,.8,.3,1) .25s forwards}
@keyframes draw{to{stroke-dashoffset:var(--final)}}
.donut-label{position:absolute;inset:0;display:grid;place-content:center;text-align:center}
.donut-label .n{font-family:var(--mono);font-size:40px;font-weight:600;line-height:1;
                letter-spacing:-.03em;font-variant-numeric:tabular-nums}
.donut-label .u{font-size:10px;color:var(--muted);font-family:var(--mono);letter-spacing:.16em;
                text-transform:uppercase;margin-top:6px}

.verdict-chip{display:inline-flex;align-items:center;gap:9px;padding:6px 15px;border-radius:999px;
              font-weight:600;font-size:14.5px;border:1px solid transparent;line-height:1.45}
.verdict-chip .dot{width:8px;height:8px;border-radius:50%;background:currentColor;
                   animation:pulse 2.4s ease-in-out infinite}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.3}}
.v-good{background:var(--pass-bg);color:var(--pass);border-color:color-mix(in srgb,var(--pass) 30%,transparent)}
.v-mid{background:var(--accent-wash);color:var(--accent);border-color:color-mix(in srgb,var(--accent) 30%,transparent)}
.v-warn{background:var(--warn-bg);color:var(--warn);border-color:color-mix(in srgb,var(--warn) 30%,transparent)}
.v-bad{background:var(--fail-bg);color:var(--fail);border-color:color-mix(in srgb,var(--fail) 30%,transparent)}
.v-none{background:var(--skip-bg);color:var(--skip);border-color:var(--rule-2)}

.hero-body h1{font-size:clamp(23px,3.2vw,32px);margin:15px 0 8px;overflow-wrap:anywhere}
.hero-body>p{margin:0;color:var(--ink-2);max-width:64ch}
.axes{margin-top:20px;display:grid;gap:1px;background:var(--rule);border:1px solid var(--rule);
      border-radius:var(--radius-sm);overflow:hidden;
      grid-template-columns:repeat(auto-fit,minmax(155px,1fr))}
.axes div{background:var(--surface);padding:10px 14px;min-width:0}
.axes .k{font-family:var(--mono);font-size:10px;letter-spacing:.12em;text-transform:uppercase;
         color:var(--muted)}
.axes .v{font-size:13.5px;color:var(--ink);margin-top:2px;font-weight:500;overflow-wrap:anywhere}

/* probe tally meter */
.tally{margin-top:18px}
.tally-bar{display:flex;height:8px;border-radius:999px;overflow:hidden;background:var(--surface-3)}
.tally-seg{width:0;animation:grow 1s cubic-bezier(.22,.8,.3,1) .4s forwards}
.tally-keys{margin-top:9px;display:flex;flex-wrap:wrap;gap:6px 16px;
            font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.tally-keys span{display:inline-flex;align-items:center;gap:6px}
.tally-keys i{width:7px;height:7px;border-radius:2px;display:inline-block}
.tally-keys b{color:var(--ink);font-weight:600;font-variant-numeric:tabular-nums}

/* ── gate alert ─────────────────────────────────────── */
.gates{margin:26px 0 0;border:1px solid color-mix(in srgb,var(--fail) 42%,transparent);
       border-left:4px solid var(--fail);border-radius:var(--radius-sm);
       background:var(--fail-bg);overflow:hidden;box-shadow:var(--shadow-1)}
.gates header{padding:12px 18px;font-family:var(--mono);font-size:13px;font-weight:600;
              color:var(--fail);border-bottom:1px solid color-mix(in srgb,var(--fail) 25%,transparent);
              display:flex;align-items:center;gap:9px}
.gates ul{margin:0;padding:13px 20px 15px 38px}
.gates li{margin-bottom:7px;font-size:14px;color:var(--ink)}
.gates li:last-child{margin-bottom:0}
.gates li b{font-family:var(--mono);font-size:13px}
.gates li .src{color:var(--muted);font-size:12px;font-family:var(--mono)}

/* ── layout ─────────────────────────────────────────── */
main{counter-reset:sec}
section{padding:38px 0 4px;scroll-margin-top:52px;counter-increment:sec}
.sec-head{display:flex;align-items:baseline;gap:12px;margin-bottom:14px;flex-wrap:wrap}
.sec-head:before{content:counter(sec,decimal-leading-zero);font-family:var(--mono);font-size:10.5px;
                 font-weight:600;letter-spacing:.1em;color:var(--accent);align-self:center;
                 padding:3px 8px;border-radius:999px;background:var(--accent-wash);
                 border:1px solid color-mix(in srgb,var(--accent) 24%,transparent)}
.sec-head h2{font-size:18.5px}
.sec-head .hint{font-size:13px;color:var(--muted)}
.grid{display:grid;gap:14px}
/* 390px, not 330: a `.g2` section holding two cards plus a full-width one
   would otherwise fit three tracks and leave the first row half empty. */
.g2{grid-template-columns:repeat(auto-fit,minmax(min(100%,390px),1fr))}
.g4{grid-template-columns:repeat(auto-fit,minmax(180px,1fr))}
.span-all{grid-column:1/-1}
.card{background:var(--surface);border:1px solid var(--rule);border-radius:var(--radius);
      padding:18px 20px;box-shadow:var(--shadow-1);
      transition:box-shadow .2s ease,border-color .2s ease}
.card:hover{box-shadow:var(--shadow-2);border-color:var(--rule-2)}
.card h3{font-size:11px;letter-spacing:.11em;text-transform:uppercase;color:var(--muted);
         margin-bottom:12px}
.card h3+.bars,.card h3+.cmp,.card h3+.dl{margin-top:-2px}

/* ── stat tiles ─────────────────────────────────────── */
.stat{position:relative;overflow:hidden;padding-top:20px}
.stat:before{content:"";position:absolute;top:0;left:0;right:0;height:3px;
             background:var(--rule-2)}
.stat.good:before{background:linear-gradient(90deg,var(--pass),var(--pass-2))}
.stat.warn:before{background:linear-gradient(90deg,var(--warn),var(--warn-2))}
.stat.bad:before{background:linear-gradient(90deg,var(--fail),var(--fail-2))}
.stat .v{font-family:var(--mono);font-size:30px;font-weight:600;line-height:1.15;
         letter-spacing:-.025em;font-variant-numeric:tabular-nums}
.stat .s{font-size:12.5px;color:var(--muted);margin-top:5px;text-wrap:pretty}
.stat .v.good{color:var(--pass)} .stat .v.warn{color:var(--warn)} .stat .v.bad{color:var(--fail)}

/* ── bars ───────────────────────────────────────────── */
.bars{display:grid;gap:10px}
.bar-row{display:grid;grid-template-columns:clamp(96px,26%,180px) 1fr 54px;gap:12px;
         align-items:center;font-size:13px}
@media (max-width:520px){.bar-row{grid-template-columns:92px 1fr 46px;gap:9px}}
.bar-row .lbl{color:var(--ink-2);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bar-track{height:9px;border-radius:999px;background:var(--surface-3);overflow:hidden;
           box-shadow:inset 0 0 0 1px color-mix(in srgb,var(--rule-2) 45%,transparent)}
.bar-fill{height:100%;border-radius:999px;width:0;
          animation:grow 1s cubic-bezier(.22,.8,.3,1) .15s forwards}
@keyframes grow{to{width:var(--w)}}
.bar-row .num{font-family:var(--mono);font-size:12.5px;text-align:right;
              font-variant-numeric:tabular-nums;color:var(--ink-2)}

/* ── key/value list ─────────────────────────────────── */
.dl{display:grid}
.dl-row{display:flex;justify-content:space-between;align-items:baseline;gap:18px;
        padding:9px 0;border-bottom:1px dashed var(--rule)}
.dl-row:last-child{border-bottom:none;padding-bottom:2px}
.dl-row .k{color:var(--muted);font-size:12.5px;white-space:nowrap}
.dl-row .v{font-family:var(--mono);font-size:12.5px;text-align:right;
           font-variant-numeric:tabular-nums;color:var(--ink);overflow-wrap:anywhere}

/* ── billing comparison ─────────────────────────────── */
.cmp{display:grid;gap:14px}
.cmp-row .top{display:flex;justify-content:space-between;align-items:baseline;
              font-size:12.5px;margin-bottom:6px;gap:12px}
.cmp-row .top .k{color:var(--muted)}
.cmp-row .top .n{font-family:var(--mono);font-variant-numeric:tabular-nums;color:var(--ink)}
.ratio-note{margin-top:15px;padding:11px 14px;border-radius:var(--radius-sm);font-size:13.5px;
            line-height:1.6;border:1px solid transparent}
.ratio-ok{background:var(--pass-bg);color:var(--pass);border-color:color-mix(in srgb,var(--pass) 22%,transparent)}
.ratio-hi{background:var(--warn-bg);color:var(--warn);border-color:color-mix(in srgb,var(--warn) 22%,transparent)}
.ratio-bad{background:var(--fail-bg);color:var(--fail);border-color:color-mix(in srgb,var(--fail) 22%,transparent)}
.foot-note{margin-top:12px;font-size:12.5px;color:var(--muted);text-wrap:pretty}

/* ── distribution chart ─────────────────────────────── */
.dist{position:relative;display:flex;align-items:flex-end;gap:4px;height:112px;
      padding:10px 2px 0;border-bottom:1px solid var(--rule-2);
      background-image:repeating-linear-gradient(to bottom,var(--rule) 0 1px,transparent 1px 25%)}
/* stretch, not flex-end: a percentage bar height needs a resolved parent
   height, and an auto-height column silently collapses every bar to zero. */
.dist-col{flex:1;align-self:stretch;display:flex;flex-direction:column;justify-content:flex-end;
          min-width:4px;position:relative;z-index:1}
.dist-bar{width:100%;border-radius:3px 3px 0 0;height:0;
          background:linear-gradient(to top,var(--accent),var(--accent-2));
          animation:rise-bar .8s cubic-bezier(.22,.8,.3,1) forwards;opacity:.9}
.dist-col:hover .dist-bar{opacity:1}
@keyframes rise-bar{to{height:var(--h)}}
.dist-mark{position:absolute;left:0;right:0;border-top:1px dashed var(--fail);z-index:2;
           pointer-events:none}
.dist-mark span{position:absolute;right:0;top:-8px;font-family:var(--mono);font-size:9.5px;
                color:var(--fail);background:var(--surface);padding:0 4px;border-radius:3px;
                letter-spacing:.04em}
.dist-axis{display:flex;justify-content:space-between;font-family:var(--mono);font-size:10.5px;
           color:var(--muted);margin-top:7px}

/* ── hop chain ──────────────────────────────────────── */
.chain{list-style:none;display:flex;align-items:center;gap:9px;flex-wrap:wrap;
       margin:0 0 16px;padding:0}
.chain .hop{font-family:var(--mono);font-size:12px;padding:6px 12px;border-radius:999px;
     background:var(--accent-wash);color:var(--accent);
     border:1px solid color-mix(in srgb,var(--accent) 26%,transparent)}
.chain .hop.you{background:var(--surface-2);color:var(--ink-2);border-color:var(--rule-2)}
.chain .arrow{color:var(--rule-2);font-family:var(--mono);font-size:14px;line-height:1}

/* ── evidence lists ─────────────────────────────────── */
.ev{margin:0;padding-left:19px;font-size:13.5px;color:var(--ink-2)}
.ev li{margin-bottom:5px}
.ev li::marker{color:var(--rule-2)}

/* ── probe table ────────────────────────────────────── */
.tbl-wrap{overflow-x:auto;border:1px solid var(--rule);border-radius:var(--radius);
          background:var(--surface);box-shadow:var(--shadow-1)}
table{border-collapse:collapse;width:100%;min-width:680px;font-size:13.5px}
thead th{font-family:var(--mono);font-size:10px;letter-spacing:.11em;text-transform:uppercase;
         color:var(--muted);font-weight:600;text-align:left;padding:11px 15px;
         background:var(--surface-2);border-bottom:1px solid var(--rule-2);white-space:nowrap}
tbody td{padding:10px 15px;border-bottom:1px solid var(--rule);vertical-align:top}
tbody tr:last-child td{border-bottom:none}
tbody tr:not(.grp):hover td{background:color-mix(in srgb,var(--surface-2) 55%,transparent)}
tbody tr.grp td{background:var(--surface-2);padding:9px 15px;border-bottom:1px solid var(--rule-2)}
.grp-head{display:flex;align-items:baseline;gap:10px;flex-wrap:wrap}
.grp-head b{font-family:var(--mono);font-size:11.5px;letter-spacing:.06em;color:var(--ink-2);
            font-weight:600;text-transform:uppercase}
.grp-head span{font-size:12px;color:var(--muted)}
.grp-head em{margin-left:auto;font-style:normal;font-family:var(--mono);font-size:11px;
             color:var(--muted);font-variant-numeric:tabular-nums}
td.st{width:78px;white-space:nowrap}
td.pid{font-family:var(--mono);font-size:11.5px;color:var(--muted);white-space:nowrap}
td.plabel{font-weight:600;white-space:nowrap}
td.psum{color:var(--ink-2)}
td.pms{font-family:var(--mono);font-size:12px;text-align:right;color:var(--muted);
       white-space:nowrap;font-variant-numeric:tabular-nums}
.tag{display:inline-flex;align-items:center;gap:5px;font-family:var(--mono);font-size:11px;
     font-weight:600;padding:3px 9px;border-radius:999px;border:1px solid transparent}
.tag.pass{background:var(--pass-bg);color:var(--pass);border-color:color-mix(in srgb,var(--pass) 22%,transparent)}
.tag.warn{background:var(--warn-bg);color:var(--warn);border-color:color-mix(in srgb,var(--warn) 22%,transparent)}
.tag.fail{background:var(--fail-bg);color:var(--fail);border-color:color-mix(in srgb,var(--fail) 22%,transparent)}
.tag.skip{background:var(--skip-bg);color:var(--skip);border-color:var(--rule-2)}
.tag.err{background:var(--fail-bg);color:var(--fail);border-color:color-mix(in srgb,var(--fail) 22%,transparent)}
.neutral-note{font-weight:400;font-size:11px;color:var(--muted)}
details.detail{margin-top:7px}
details.detail summary{cursor:pointer;font-size:12px;color:var(--accent);font-family:var(--mono);
                       list-style:none;display:inline-block;padding:1px 0}
details.detail summary::-webkit-details-marker{display:none}
details.detail summary:before{content:"▸ "}
details.detail[open] summary:before{content:"▾ "}
details.detail summary:hover{text-decoration:underline}
.detail-body{margin-top:8px;padding:11px 14px;background:var(--surface-2);
             border-radius:var(--radius-xs);font-size:12.5px;color:var(--ink-2);
             border-left:2px solid var(--accent)}
.detail-body ul{margin:0 0 8px;padding-left:17px}
.detail-body pre{margin:7px 0 0;white-space:pre-wrap;overflow-wrap:anywhere;
                 font-family:var(--mono);font-size:11.5px;color:var(--ink);
                 background:var(--surface);padding:10px 12px;border-radius:var(--radius-xs);
                 border:1px solid var(--rule);max-height:230px;overflow:auto}
.kv{display:flex;flex-wrap:wrap;gap:5px 8px;margin-top:6px}
.kv code{font-size:11px;background:var(--surface);padding:2px 7px;border-radius:999px;
         border:1px solid var(--rule);color:var(--ink-2)}

/* ── trace / notes ──────────────────────────────────── */
ol.trace{margin:0;padding-left:24px;font-family:var(--mono);font-size:12.5px;color:var(--ink-2)}
ol.trace li{margin-bottom:6px;padding-left:2px}
ol.trace li::marker{color:var(--accent);font-size:11px}
.note{border-left:3px solid var(--accent);background:var(--surface-2);padding:14px 18px;
      border-radius:0 var(--radius-sm) var(--radius-sm) 0;font-size:13.5px;line-height:1.7;
      color:var(--ink-2)}
.note b{color:var(--ink)}
.note.warnbox{border-left-color:var(--warn)}
.limits{margin:0;padding:0;list-style:none;display:grid;gap:10px;
        grid-template-columns:repeat(auto-fit,minmax(330px,1fr))}
.limits li{background:var(--surface);border:1px solid var(--rule);border-radius:var(--radius-sm);
           padding:14px 17px;font-size:13px;color:var(--ink-2);line-height:1.65}
.limits li b{display:block;color:var(--ink);font-family:var(--mono);font-size:12.5px;
             margin-bottom:4px;letter-spacing:-.01em}

footer{padding:30px 0 52px;color:var(--muted);font-size:12px;font-family:var(--mono);
       border-top:1px solid var(--rule);margin-top:38px}
footer p{margin:4px 0;overflow-wrap:anywhere}
footer a{color:var(--accent);text-decoration:none}
footer a:hover{text-decoration:underline}
footer .disclaimer{margin-top:12px;font-family:var(--sans);font-size:12.5px;line-height:1.6;
                   max-width:76ch}

/* ── print ──────────────────────────────────────────── */
@media print{
  .toc{display:none}
  body{background:#fff;background-image:none;color:#000;font-size:11pt}
  .rise{animation:none;opacity:1;transform:none}
  .donut-ring{animation:none;stroke-dashoffset:var(--final)}
  .bar-fill,.tally-seg{animation:none;width:var(--w)}
  .dist-bar{animation:none;height:var(--h)}
  .verdict-chip .dot{animation:none}
  .card,.hero,.tbl-wrap,.limits li{box-shadow:none;break-inside:avoid}
  section{break-inside:avoid;padding-top:22px}
  details.detail[open] .detail-body{break-inside:avoid}
  details.detail:not([open]) summary{display:none}
}
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

/// The same three bands as [`score_colour`], as a gradient for filled bars.
fn score_gradient(pct: f64) -> &'static str {
    if pct >= 85.0 {
        "linear-gradient(90deg,var(--pass),var(--pass-2))"
    } else if pct >= 60.0 {
        "linear-gradient(90deg,var(--warn),var(--warn-2))"
    } else {
        "linear-gradient(90deg,var(--fail),var(--fail-2))"
    }
}

/// `.dist` is 112px tall with 10px of top padding, so the bars occupy the
/// lower 102px. Keep this in step with the stylesheet.
const DIST_PLOT_RATIO: f64 = 102.0 / 112.0;

fn stat_class(pct: f64) -> &'static str {
    if pct >= 85.0 {
        "good"
    } else if pct >= 60.0 {
        "warn"
    } else {
        "bad"
    }
}

/// Percentile of an unsorted sample, used for the reference lines on the
/// distribution charts.
fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() - 1) as f64 * p).round() as usize;
    v[idx]
}

pub fn render(rep: &Report) -> String {
    let mut h = String::with_capacity(96 * 1024);
    let l = rep.lang;

    let _ = write!(
        h,
        r#"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="report-language" content="{lang}">
<meta name="generator" content="llm-verify {ver}">
<title>llm-verify · {host}</title>
<style>{CSS}</style>
</head>
<body>
"#,
        lang = l.html_lang(),
        ver = esc(&rep.tool_version),
        host = esc(&rep.host),
    );

    masthead(&mut h, rep);
    let _ = write!(h, r#"<div class="wrap">"#);
    hero(&mut h, rep);
    let _ = write!(h, "</div>");
    toc(&mut h, rep);

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
    let _ = write!(h, "\n</body>\n</html>\n");
    h
}

/// Section anchors, in document order, with the short label the index uses.
/// Kept in one place so the index at the top and the sections below can never
/// drift apart. The labels are deliberately shorter than the section headings:
/// the index is one line and has to survive nine entries on a laptop.
fn sections(rep: &Report) -> Vec<(&'static str, &'static str)> {
    let l = rep.lang;
    let mut v = vec![
        ("numbers", ts!(l, "Key numbers", "关键指标")),
        ("groups", ts!(l, "Group scores", "分组得分")),
        ("identity", ts!(l, "Identity", "模型身份")),
        ("billing", ts!(l, "Billing", "计量计费")),
    ];
    if rep.perf.samples > 0 {
        v.push(("perf", ts!(l, "Performance", "性能")));
    }
    v.push(("channel", ts!(l, "Provenance", "渠道来源")));
    v.push(("probes", ts!(l, "Probes", "探针明细")));
    v.push(("verdict", ts!(l, "Verdict", "判定过程")));
    v.push(("limits", ts!(l, "Limits", "能力边界")));
    v
}

fn toc(h: &mut String, rep: &Report) {
    let _ = write!(h, r#"<nav class="toc"><div class="wrap">"#);
    for (id, label) in sections(rep) {
        let _ = write!(h, r##"<a href="#{id}">{}</a>"##, esc(label));
    }
    let _ = write!(h, "</div></nav>");
}

/// Open a numbered section. The number itself comes from a CSS counter, so
/// adding or removing a panel never needs a hand-maintained index.
fn sec_open(h: &mut String, id: &str, title: &str, hint: &str) {
    let _ = write!(
        h,
        r#"<section id="{id}" class="rise"><div class="sec-head"><h2>{}</h2>
<span class="hint">{}</span></div>"#,
        esc(title),
        esc(hint),
    );
}

fn masthead(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let _ = write!(
        h,
        r#"<header class="mast"><div class="wrap">
<div class="brand"><span class="mark">V</span><span class="name">llm-verify</span>
<span class="ver">v{ver}</span><span class="stamp">{started}</span></div>
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
    // r=65 ring on a 170px box; circumference drives the draw animation.
    const R: f64 = 65.0;
    let circ = 2.0 * std::f64::consts::PI * R;
    let final_offset = circ * (1.0 - (v.score / 100.0).clamp(0.0, 1.0));

    let _ = write!(
        h,
        r#"<div class="hero">
<div class="donut">
<svg width="170" height="170" viewBox="0 0 170 170" role="img" aria-label="{aria}">
<circle class="donut-track" cx="85" cy="85" r="{r}"></circle>
<circle class="donut-ring" cx="85" cy="85" r="{r}"
        style="--circ:{circ:.2};--final:{fin:.2};stroke:{col}"></circle>
</svg>
<div class="donut-label"><div class="n">{score:.0}</div><div class="u">{k_score}</div></div>
</div>
<div class="hero-body">
<span class="verdict-chip {vcss}"><span class="dot"></span>{vlabel}</span>
<h1>{host}</h1>
<p>{vdesc}</p>
<div class="axes">
<div><div class="k">{k_origin}</div><div class="v">{chan}</div></div>
<div><div class="k">{k_ident}</div><div class="v">{ident}</div></div>
<div><div class="k">{k_conf}</div><div class="v">{conf:.0}%</div></div>
<div><div class="k">{k_reqs}</div><div class="v">{reqs}</div></div>
</div>"#,
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
        k_score = esc(ts!(l, "of 100", "满分 100")),
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
        k_reqs = ts!(l, "Requests", "请求"),
        reqs = esc(&t!(
            l,
            "{} · {:.1}s",
            "{} 次 · {:.1}s",
            rep.request_count,
            rep.duration_ms as f64 / 1000.0
        )),
    );

    tally(h, rep);
    let _ = write!(h, "</div></div>");
}

/// One segmented bar for the whole probe run. The proportions carry more at a
/// glance than four separate counts do.
fn tally(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let total = rep.results.len();
    if total == 0 {
        return;
    }
    let bands = [
        (
            Status::Pass,
            "var(--pass)",
            ts!(l, "pass", "通过"),
            rep.count(Status::Pass),
        ),
        (
            Status::Warn,
            "var(--warn)",
            ts!(l, "warn", "警告"),
            rep.count(Status::Warn),
        ),
        (
            Status::Fail,
            "var(--fail)",
            ts!(l, "fail", "失败"),
            rep.count(Status::Fail),
        ),
        (
            Status::Skip,
            "var(--skip)",
            ts!(l, "skip", "跳过"),
            rep.count(Status::Skip) + rep.count(Status::Error),
        ),
    ];

    let _ = write!(h, r#"<div class="tally"><div class="tally-bar">"#);
    for (_, colour, label, n) in bands {
        if n == 0 {
            continue;
        }
        let _ = write!(
            h,
            r#"<span class="tally-seg" style="--w:{w:.2}%;background:{colour}" title="{label} {n}"></span>"#,
            w = n as f64 / total as f64 * 100.0,
            colour = colour,
            label = esc(label),
            n = n,
        );
    }
    let _ = write!(h, r#"</div><div class="tally-keys">"#);
    for (_, colour, label, n) in bands {
        let _ = write!(
            h,
            r#"<span><i style="background:{colour}"></i><b>{n}</b> {label}</span>"#,
            colour = colour,
            n = n,
            label = esc(label),
        );
    }
    let _ = write!(
        h,
        r#"<span>{}</span></div></div>"#,
        esc(&t!(l, "{} probes total", "共 {} 项探针", total))
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
            r#"<li><b>{}</b> — {}<br><span class="src">{}</span></li>"#,
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
    sec_open(
        h,
        "numbers",
        ts!(l, "Key numbers", "关键指标"),
        ts!(
            l,
            "The state of this endpoint at a glance",
            "一眼看懂这个端点当前的状态"
        ),
    );
    let _ = write!(h, r#"<div class="grid g4">"#);

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
        r#"<div class="card stat {cls}"><h3>{label}</h3><div class="v {cls}">{value}</div>
<div class="s">{sub}</div></div>"#,
        cls = cls,
        label = esc(label),
        value = esc(value),
        sub = esc(sub),
    );
}

fn group_scores(h: &mut String, rep: &Report) {
    let l = rep.lang;
    sec_open(
        h,
        "groups",
        ts!(l, "Scores by group", "分组得分"),
        ts!(
            l,
            "Locate which layer the problems are in",
            "问题出在哪一层，一眼定位"
        ),
    );
    let _ = write!(h, r#"<div class="card"><div class="bars">"#);
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
            col = score_gradient(*pct),
        );
    }
    let _ = write!(h, "</div></div></section>");
}

fn identity_panel(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let id = &rep.identity;
    sec_open(
        h,
        "identity",
        ts!(l, "Model identity", "模型身份"),
        ts!(
            l,
            "Is the model behind this the one that was sold?",
            "背后跑的是不是它声称的那个模型"
        ),
    );
    let _ = write!(h, r#"<div class="grid g2">"#);

    // Claim vs observation.
    let _ = write!(
        h,
        r#"<div class="card"><h3>{title}</h3>
<div class="dl">
<div class="dl-row"><span class="k">{k1}</span><span class="v">{claimed}</span></div>
<div class="dl-row"><span class="k">{k2}</span><span class="v">{cf} / {ct}</span></div>
<div class="dl-row"><span class="k">{k3}</span><span class="v">{of}</span></div>
<div class="dl-row"><span class="k">{k4}</span><span class="v">{et}</span></div>
<div class="dl-row"><span class="k">{k5}</span><span class="v">{basis}</span></div>
</div>
<div class="ratio-note {ncls}">{istatus}</div>{thin}
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
                r#"<div class="foot-note">{}</div>"#,
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
                "linear-gradient(90deg,var(--accent),var(--accent-2))"
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
            r#"<h3 style="margin-top:18px">{}</h3><div class="bars">"#,
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
                col = score_gradient(acc * 100.0),
            );
        }
        let _ = write!(h, "</div>");
    }
    let _ = write!(h, "</div>");

    if !id.evidence.is_empty() {
        let _ = write!(
            h,
            r#"<div class="card span-all"><h3>{}</h3><ul class="ev">"#,
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
    sec_open(
        h,
        "billing",
        ts!(l, "Metering & billing", "计量与计费"),
        ts!(
            l,
            "Are the token counts honest, or are you overcharged?",
            "计费数字可信吗，有没有多收钱"
        ),
    );
    let _ = write!(h, r#"<div class="grid g2">"#);

    // Billed vs honest bars, scaled to whichever is larger.
    let max = b.billed_input.max(b.honest_input).max(1) as f64;
    let _ = write!(
        h,
        r#"<div class="card"><h3>{title}</h3><div class="cmp">
<div class="cmp-row"><div class="top"><span class="k">{k1}</span><span class="n">{billed}</span></div>
<div class="bar-track"><div class="bar-fill" style="--w:{bw:.1}%;background:{bc}"></div></div></div>
<div class="cmp-row"><div class="top"><span class="k">{k2}</span><span class="n">{honest}</span></div>
<div class="bar-track"><div class="bar-fill" style="--w:{hw:.1}%;background:linear-gradient(90deg,var(--accent),var(--accent-2))"></div></div></div>
</div>
<div class="ratio-note {rcls}">{rtext}</div>
<div class="foot-note">{method}</div>
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
            "linear-gradient(90deg,var(--fail),var(--fail-2))"
        } else {
            "linear-gradient(90deg,var(--pass),var(--pass-2))"
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
            r#"<div class="dl">
<div class="dl-row"><span class="k">{k1}</span><span class="v">${bc:.6}</span></div>
<div class="dl-row"><span class="k">{k2}</span><span class="v">${hc:.6}</span></div>
<div class="dl-row"><span class="k">{k3}</span><span class="v">${d:.6}</span></div>
</div>
<div class="foot-note">{note}</div>"#,
            k1 = esc(ts!(l, "At the billed counts", "按计费数字")),
            bc = b.billed_cost_usd,
            k2 = esc(ts!(l, "At the measured counts", "按实测数字")),
            hc = b.honest_cost_usd,
            k3 = esc(ts!(l, "Difference", "差额")),
            d = b.billed_cost_usd - b.honest_cost_usd,
            // The source is a noun phrase ("built-in price table"), so it needs
            // a carrier sentence around it rather than being dropped in front
            // of a full stop.
            note = esc(&t!(
                l,
                "Priced from the {}. These figures cover only this run's few probe \
                 requests — they illustrate the ratio, they are not a monthly estimate.",
                "按{}折算。金额只是本次几个探测请求的量级，用于说明倍率，不是月账单预估。",
                b.pricing_source
            )),
        );
    } else {
        let _ = write!(
            h,
            r#"<div class="note warnbox">{}</div>"#,
            esc(&b.pricing_source)
        );
    }
    if !b.anomalies.is_empty() {
        let _ = write!(
            h,
            r#"<h3 style="margin-top:18px">{}</h3><ul class="ev">"#,
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
    sec_open(
        h,
        "perf",
        ts!(l, "Performance", "性能"),
        ts!(
            l,
            "Latency, throughput and jitter — also identity evidence",
            "首字延迟、吞吐与抖动，也是身份旁证"
        ),
    );
    let _ = write!(h, r#"<div class="grid g2">"#);

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
        r#"<div class="card span-all"><h3>{}</h3><div class="grid g4" style="gap:14px">"#,
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
            r#"<div><div style="font-size:10px;letter-spacing:.11em;text-transform:uppercase;color:var(--muted);font-family:var(--mono)">{}</div>
<div style="font-family:var(--mono);font-size:18px;font-variant-numeric:tabular-nums;letter-spacing:-.02em;margin-top:2px">{}</div></div>"#,
            esc(&label),
            esc(&value)
        );
    }
    if p.latency_cv > 0.5 {
        let _ = write!(
            h,
            r#"</div><div class="ratio-note ratio-hi">{}</div>"#,
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
    let p50 = percentile(values, 0.5);
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
            r#"<div class="dist-col" title="{tip}"><div class="dist-bar" style="--h:{pct:.1}%;animation-delay:{d:.2}s"></div></div>"#,
            tip = esc(&format!("#{} · {v:.0}{unit}", i + 1)),
            pct = pct,
            d = 0.3 + i as f64 * 0.05,
        );
    }
    // The median line turns a wall of bars into a readable distribution: how
    // far the tail sits above the typical sample is the whole point.
    let _ = write!(
        h,
        r#"<div class="dist-mark" style="bottom:{mark:.1}%"><span>P50 {p50:.0}{unit}</span></div>"#,
        // The plot area is the chart box minus its top padding, and a
        // percentage `bottom` resolves against the full box — hence the ratio.
        mark = (p50 / max * 100.0).clamp(4.0, 100.0) * DIST_PLOT_RATIO,
        p50 = p50,
        unit = unit,
    );
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
    sec_open(
        h,
        "channel",
        ts!(l, "Channel provenance", "渠道来源"),
        ts!(
            l,
            "What the request passes through before it reaches the model",
            "请求到达模型前经过了什么"
        ),
    );
    let _ = write!(
        h,
        r#"<div class="card"><ol class="chain"><li class="hop you">{}</li>"#,
        esc(ts!(l, "your request", "你的请求"))
    );
    if c.all_hops.is_empty() {
        let _ = write!(
            h,
            r#"<li class="arrow">→</li><li class="hop">{}</li>"#,
            esc(&c.display)
        );
    } else {
        for hop in &c.all_hops {
            let _ = write!(
                h,
                r#"<li class="arrow">→</li><li class="hop">{}</li>"#,
                esc(hop)
            );
        }
    }
    let _ = write!(
        h,
        r#"<li class="arrow">→</li><li class="hop you">{}</li></ol>"#,
        esc(ts!(l, "model", "模型"))
    );

    let _ = write!(
        h,
        r#"<div class="dl">
<div class="dl-row"><span class="k">{k1}</span><span class="v">{ident}</span></div>
<div class="dl-row"><span class="k">{k2}</span><span class="v">{chan}</span></div></div>
<div class="foot-note" style="color:var(--ink-2);font-size:13px">{desc}</div>"#,
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
            r#"<h3 style="margin-top:18px">{}</h3><ul class="ev">"#,
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
    sec_open(
        h,
        "probes",
        ts!(l, "Probe detail", "探针明细"),
        &t!(
            l,
            "{n} probes; expand any row for evidence and the raw response",
            "{n} 项，点开可看证据与原始响应",
            n = rep.results.len()
        ),
    );
    let _ = write!(
        h,
        r#"<div class="tbl-wrap"><table>
<thead><tr><th>{c1}</th><th>ID</th><th>{c2}</th><th>{c3}</th><th style="text-align:right">{c4}</th></tr></thead>
<tbody>"#,
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
        // A per-group tally on the group row saves scanning every line to see
        // where a group lost its points.
        let pass = rows.iter().filter(|r| r.status == Status::Pass).count();
        let _ = write!(
            h,
            r#"<tr class="grp"><td colspan="5"><div class="grp-head"><b>{}</b><span>{}</span>
<em>{}</em></div></td></tr>"#,
            esc(g.label(l)),
            esc(g.blurb(l)),
            esc(&t!(l, "{}/{} passed", "{}/{} 通过", pass, rows.len())),
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
                        r#"<br><span class="neutral-note">{}</span>"#,
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
    sec_open(
        h,
        "verdict",
        ts!(l, "How the verdict was reached", "判定过程"),
        ts!(
            l,
            "Every step, so you can check it yourself",
            "结论怎么来的，可以自己复核"
        ),
    );
    let _ = write!(
        h,
        r#"<div class="grid g2"><div class="card"><h3>{}</h3><ol class="trace">"#,
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
            r#"<h3 style="margin-top:18px">{title}</h3>
<div class="note warnbox" style="margin-bottom:11px;font-size:13px">
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
    sec_open(
        h,
        "limits",
        ts!(l, "What this cannot prove", "能力边界"),
        ts!(l, "The limits of this report", "这份报告不能证明什么"),
    );
    let _ = write!(
        h,
        r#"<div class="note"><b>{lead}</b> {lead2}</div>
<ul class="limits" style="margin-top:16px">"#,
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
        let _ = write!(h, "<li><b>{}</b>{}</li>", esc(head), body);
    }
    let _ = write!(h, "</ul></section>");
}

fn footer(h: &mut String, rep: &Report) {
    let l = rep.lang;
    let _ = write!(
        h,
        r##"<footer class="wrap">
<p>{line1}</p>
<p>{line2}</p>
<p class="disclaimer">{line3}</p>
<p><a href="#top">{top}</a></p>
</footer>"##,
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
        top = esc(ts!(l, "back to top ↑", "回到顶部 ↑")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Protocol;
    use std::collections::BTreeMap;

    /// A report with every panel populated, so the renderer is exercised on
    /// the shape a real run produces rather than on `Default::default()`.
    fn sample(lang: Lang) -> Report {
        let mut group_scores = BTreeMap::new();
        for (k, v) in [
            ("contract", 96.0),
            ("stream", 100.0),
            ("billing", 88.0),
            ("channel", 54.0),
            ("perf", 72.0),
            ("identity", 91.0),
            ("consistency", 100.0),
        ] {
            group_scores.insert(k.to_string(), v);
        }
        let mut tier_scores = BTreeMap::new();
        tier_scores.insert("large".into(), 0.82);
        tier_scores.insert("mid".into(), 0.61);
        tier_scores.insert("small".into(), 0.24);
        let mut acc = BTreeMap::new();
        acc.insert("easy".into(), 1.0);
        acc.insert("medium".into(), 0.83);
        acc.insert("hard".into(), 0.5);

        let results = vec![
            ProbeResult::new("contract.headers", "Response headers", Group::Contract)
                .pass("Vendor markers absent, shape otherwise correct")
                .metric("server", "cloudflare")
                .evidence("HTTP/2 200\nserver: cloudflare")
                .took(412),
            ProbeResult::new("stream.sse", "SSE framing", Group::Stream)
                .pass("Well-formed event stream")
                .took(980),
            ProbeResult::new("billing.recount", "Independent recount", Group::Billing)
                .warn("Billed 6% above the local estimate")
                .finding("ratio 1.06 across 3 rounds")
                .took(1_240),
            ProbeResult::new("channel.hops", "Relay signatures", Group::Channel)
                .fail("Two relay signatures on the path")
                .took(305),
            ProbeResult::new("perf.ttft", "First-token latency", Group::Perf)
                .warn("P95 well above P50")
                .took(2_100),
            ProbeResult::new("identity.tier", "Capability tier", Group::Identity)
                .pass("Flagship tier, margin 0.21")
                .neutral()
                .took(31_500),
            ProbeResult::new("consistency.repeat", "Repeat requests", Group::Consistency)
                .skip("Endpoint does not expose a seed")
                .took(120),
        ];

        Report {
            tool_version: "0.2.2".into(),
            lang,
            started_at: "2026-08-13 07:24:01".into(),
            finished_at: "2026-08-13 07:26:15".into(),
            duration_ms: 133_800,
            host: "openrouter.ai".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            protocol: Protocol::OpenAI,
            model: "gpt-5.6-sol".into(),
            claimed_model: "gpt-5.6-sol".into(),
            depth: "balanced".into(),
            request_count: 47,
            results,
            verdict: Verdict {
                authenticity: Authenticity::ThirdParty,
                channel: Channel::Proxy,
                score: 92.0,
                confidence: 0.86,
                hard_gate_hits: vec![],
                signals: vec!["No vendor headers on any response".into()],
                trace: vec![
                    "contract layer clean, identity confidence kept at full weight".into(),
                    "two relay signatures → origin = relay".into(),
                    "weighted score 92.0, no hard gate → relayed".into(),
                ],
                group_scores,
                coverage_gap: 0.05,
            },
            identity: Identity {
                claimed_model: "gpt-5.6-sol".into(),
                claimed_family: Some("gpt".into()),
                claimed_tier: Some("large".into()),
                observed_family: Some("gpt".into()),
                family_confidence: 0.78,
                estimated_tier: Some("large".into()),
                tier_confidence: 0.82,
                tier_severity: 0,
                status: IdentityStatus::Match,
                evidence: vec!["Self-identification consistent across 4 phrasings".into()],
                tier_scores,
                accuracy_by_difficulty: acc,
                tier_questions: 9,
                tier_margin: 0.21,
            },
            billing: BillingAudit {
                rounds: vec![],
                method: "local estimate".into(),
                billed_input: 1_284,
                billed_output: 902,
                honest_input: 1_211,
                honest_output: 902,
                input_ratio: 1.06,
                billed_cost_usd: 0.014_2,
                honest_cost_usd: 0.013_4,
                cost_ratio: 1.06,
                pricing_source: "built-in price table".into(),
                anomalies: vec![],
            },
            channel: ChannelSignature {
                key: "proxy".into(),
                display: "openrouter".into(),
                confidence: 0.91,
                tier: 1,
                evidence: vec!["x-openrouter-* headers present".into()],
                all_hops: vec!["cloudflare".into(), "openrouter".into()],
            },
            perf: PerfSummary {
                samples: 12,
                ttft_ms: vec![
                    820.0, 910.0, 780.0, 1_140.0, 860.0, 930.0, 1_020.0, 2_400.0, 870.0, 890.0,
                    950.0, 1_010.0,
                ],
                latency_ms: vec![
                    3_100.0, 3_400.0, 2_900.0, 4_200.0, 3_200.0, 3_500.0, 3_800.0, 7_100.0,
                    3_150.0, 3_300.0, 3_600.0, 3_750.0,
                ],
                tps: vec![48.0, 51.0, 46.0],
                ttft_p50: 915.0,
                ttft_p95: 2_180.0,
                latency_p50: 3_450.0,
                latency_p95: 6_400.0,
                tps_mean: 48.3,
                latency_cv: 0.31,
            },
            skipped: vec!["consistency.seed — endpoint does not expose a seed".into()],
        }
    }

    #[test]
    fn renders_a_complete_document() {
        let out = render(&sample(Lang::En));
        assert!(out.starts_with("<!doctype html>"));
        assert!(out.trim_end().ends_with("</html>"));
        assert!(out.contains(r#"<html lang="en">"#));
        assert!(out.contains("<title>llm-verify · openrouter.ai</title>"));
    }

    #[test]
    fn every_index_entry_has_a_section_to_land_on() {
        // The sticky index and the sections below are generated separately;
        // a dead anchor would be invisible until someone clicked it.
        for lang in [Lang::En, Lang::Zh] {
            let rep = sample(lang);
            let out = render(&rep);
            for (id, label) in sections(&rep) {
                assert!(
                    out.contains(&format!(r##"href="#{id}""##)),
                    "{id} missing from the index"
                );
                assert!(
                    out.contains(&format!(r#"<section id="{id}""#)),
                    "{id} has an index entry but no section"
                );
                assert!(!label.is_empty());
            }
        }
    }

    #[test]
    fn a_run_without_performance_samples_drops_the_section_and_its_anchor() {
        let mut rep = sample(Lang::En);
        rep.perf = PerfSummary::default();
        let out = render(&rep);
        assert!(!out.contains(r#"<section id="perf""#));
        assert!(!out.contains(r##"href="#perf""##));
    }

    #[test]
    fn hard_gates_are_rendered_above_everything_else() {
        let mut rep = sample(Lang::En);
        rep.verdict.hard_gate_hits = vec![GateHit {
            name: "silent fallback".into(),
            probe: "identity.echo".into(),
            reason: "the echoed model differs from the requested one".into(),
        }];
        let out = render(&rep);
        let gate = out.find("silent fallback").expect("gate must render");
        let first_section = out.find("<section").expect("sections must render");
        assert!(
            gate < first_section,
            "the gate alert belongs above the sections"
        );
    }

    #[test]
    fn hostile_strings_cannot_break_out_of_the_document() {
        let mut rep = sample(Lang::En);
        rep.host = "<script>alert(1)</script>".into();
        rep.results[0].summary = "</td></tr><script>x</script>".into();
        let out = render(&rep);
        assert!(!out.contains("<script>alert(1)</script>"));
        assert!(!out.contains("<script>x</script>"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn the_probe_table_carries_a_per_group_tally() {
        let out = render(&sample(Lang::En));
        // One pass out of one probe in the contract group.
        assert!(out.contains("1/1 passed"));
    }

    /// Not a test — a way to eyeball the design. Run with:
    /// `cargo test --release preview -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview() {
        for (lang, name) in [(Lang::En, "preview-en.html"), (Lang::Zh, "preview-zh.html")] {
            let path = std::path::Path::new("target").join(name);
            std::fs::write(&path, render(&sample(lang))).unwrap();
            println!("wrote {}", path.display());
        }
    }
}
