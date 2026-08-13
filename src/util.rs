// SPDX-License-Identifier: Apache-2.0
//! Small self-contained helpers. Deliberately dependency-free: every one of
//! these would otherwise pull in a crate (chrono, rand, tiktoken) that costs
//! more binary size than the handful of lines it replaces.

use std::time::{SystemTime, UNIX_EPOCH};

// ── time ───────────────────────────────────────────────────────────────────

/// Unix milliseconds. Used for durations and PRNG seeding.
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// RFC 3339 UTC timestamp, e.g. `2026-08-13T09:41:07Z`.
///
/// Implements the civil-from-days algorithm rather than pulling in chrono,
/// which would add ~300KB and a time-zone database we never consult.
pub fn iso8601_utc() -> String {
    let secs = (now_ms() / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Howard Hinnant's `civil_from_days`, shifted to a March-based year so the
/// leap day lands at the end and the month arithmetic stays branch-free.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Compact local-ish stamp for filenames: `20260813-094107`.
pub fn file_stamp() -> String {
    iso8601_utc()
        .replace(['-', ':'], "")
        .replace('T', "-")
        .replace('Z', "")
}

// ── PRNG ───────────────────────────────────────────────────────────────────

/// xorshift64*. Seeded per run so probe payloads differ every time — a
/// provider cannot pre-cache answers to canaries it has not seen.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let seed = now_ms() as u64 ^ 0x9E37_79B9_7F4A_7C15;
        Self(if seed == 0 { 0xDEAD_BEEF } else { seed })
    }

    /// Deterministic construction. Only tests need reproducible sequences;
    /// a real run must vary so providers cannot pre-cache probe payloads.
    #[cfg(test)]
    pub fn from_seed(seed: u64) -> Self {
        Self(if seed == 0 { 0xDEAD_BEEF } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[lo, hi]`. Returns `lo` when the range is empty or inverted.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo + 1) as u64;
        lo + (self.next_u64() % span) as i64
    }

    /// Uppercase hex token, used for canary markers.
    pub fn hex(&mut self, len: usize) -> String {
        const HEX: &[u8] = b"0123456789ABCDEF";
        (0..len)
            .map(|_| HEX[(self.next_u64() % 16) as usize] as char)
            .collect()
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new()
    }
}

// ── token estimation ───────────────────────────────────────────────────────

/// Heuristic token count, used only when the endpoint offers no authoritative
/// `count_tokens` route.
///
/// This is intentionally *not* a real BPE: embedding tiktoken's rank tables
/// would add several megabytes to the binary. The heuristic is calibrated for
/// the signal we actually need — inflation detection, where a genuine hit is
/// 10x to 1000x over baseline, not 10%. Anything derived from this value is
/// reported as an estimate, and the audit never claims exact billing fraud on
/// an estimate alone.
pub fn estimate_tokens(text: &str) -> u32 {
    let mut cjk = 0usize; // CJK ideographs & kana: roughly 1 token each
    let mut other = 0usize; // latin/punctuation bytes: roughly 4 chars per token
    for ch in text.chars() {
        let c = ch as u32;
        let is_cjk = (0x3040..=0x30FF).contains(&c)      // kana
            || (0x3400..=0x4DBF).contains(&c)            // CJK ext A
            || (0x4E00..=0x9FFF).contains(&c)            // CJK unified
            || (0xAC00..=0xD7AF).contains(&c)            // hangul
            || (0xF900..=0xFAFF).contains(&c); // compatibility
        if is_cjk {
            cjk += 1;
        } else {
            other += ch.len_utf8();
        }
    }
    // The +2 approximates the per-message role/delimiter overhead that every
    // chat format adds around the content.
    (cjk + other.div_ceil(4) + 2) as u32
}

// ── formatting ─────────────────────────────────────────────────────────────

/// Truncate on a char boundary, appending an ellipsis when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Escape for embedding text inside an HTML element or attribute.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Pad to a fixed *display* width, counting CJK as two columns so mixed-script
/// table rows stay aligned in a terminal. Truncates rather than overflowing.
pub fn pad_display(s: &str, width: usize) -> String {
    let mut used = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let w = if (ch as u32) >= 0x1100 && !ch.is_ascii() {
            2
        } else {
            1
        };
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

/// Percentile by nearest-rank over an already-sorted slice.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

pub fn stddev(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let m = mean(xs);
    (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_783), (2024, 3, 1)); // day after leap day
    }

    #[test]
    fn iso8601_has_expected_shape() {
        let s = iso8601_utc();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }

    #[test]
    fn rng_is_deterministic_for_a_seed_and_covers_range() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut r = Rng::from_seed(7);
        for _ in 0..500 {
            let v = r.range(3, 9);
            assert!((3..=9).contains(&v));
        }
        assert_eq!(r.range(5, 5), 5);
        assert_eq!(r.range(9, 2), 9, "inverted range collapses to lo");
    }

    #[test]
    fn estimate_tokens_separates_cjk_from_latin() {
        // Tiny prompts must stay tiny — this is what inflation detection keys on.
        assert!(estimate_tokens("Say OK") < 10);
        // CJK costs about one token per character, so it must exceed a
        // naive bytes/4 count of the same string.
        let cjk = "你好世界你好世界";
        assert!(estimate_tokens(cjk) >= 8, "{}", estimate_tokens(cjk));
        assert_eq!(estimate_tokens(""), 2);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("你好世界", 2), "你好…");
    }

    #[test]
    fn html_escape_covers_all_five_entities() {
        assert_eq!(
            html_escape(r#"<a href="x">&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn pad_display_counts_cjk_as_two_columns() {
        assert_eq!(pad_display("协议契约", 10), "协议契约  ");
        assert_eq!(pad_display("abc", 5), "abc  ");
        // Truncation must not split a character.
        assert_eq!(pad_display("协议契约检测", 5).chars().count(), 3);
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&xs, 50.0), 3.0);
        assert_eq!(percentile(&xs, 100.0), 5.0);
        assert_eq!(percentile(&xs, 0.0), 1.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn stddev_needs_two_samples() {
        assert_eq!(stddev(&[5.0]), 0.0);
        assert!((stddev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]) - 2.138).abs() < 0.01);
    }
}
