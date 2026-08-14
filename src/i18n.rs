// SPDX-License-Identifier: Apache-2.0
//! Bilingual output.
//!
//! The catalogue is deliberately *inline* rather than a separate resource
//! file: every message is written at the point it is used, with both languages
//! side by side, so a translation cannot silently drift away from the code
//! that produces it. `cargo build` fails if one half is missing.

/// Output language. English is the default because the project's public face
/// is English; Chinese is reachable explicitly or via the system locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        // Accept both bare tags and full locale strings such as `zh_CN.UTF-8`.
        if s.starts_with("zh") || s.starts_with("cmn") || s == "chinese" {
            return Some(Self::Zh);
        }
        if s.starts_with("en") || s == "english" {
            return Some(Self::En);
        }
        None
    }

    /// Short tag, used wherever the language must be stated as data rather
    /// than rendered — the report footer and the `--lang` echo.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }

    /// BCP 47 tag for the report's `lang` attribute.
    pub fn html_lang(&self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh-Hans",
        }
    }

    /// Resolve from, in order: an explicit flag, `LLM_VERIFY_LANG`, then the
    /// usual locale variables. Anything unrecognised falls through to English
    /// rather than guessing.
    pub fn resolve(explicit: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> Self {
        if let Some(v) = explicit {
            if let Some(l) = Self::parse(v) {
                return l;
            }
        }
        for key in ["LLM_VERIFY_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Some(v) = env(key) {
                // `C` and `POSIX` are "no locale set", not a language choice.
                if v.is_empty() || v.starts_with('C') || v.starts_with("POSIX") {
                    continue;
                }
                if let Some(l) = Self::parse(&v) {
                    return l;
                }
            }
        }
        Self::En
    }

    /// Production resolver reading the real process environment.
    pub fn from_env(explicit: Option<&str>) -> Self {
        Self::resolve(explicit, &|k| std::env::var(k).ok())
    }
}

/// Pick between an English and a Chinese message, applying format arguments to
/// whichever is selected.
///
/// ```ignore
/// t!(lang, "Endpoint reachable, {}ms", "端点可达，{}ms", ms)
/// ```
///
/// Both literals take the same arguments, so a mismatched placeholder count is
/// a compile error rather than a runtime surprise.
#[macro_export]
macro_rules! t {
    // Still `format!`, even with no trailing arguments: the messages use
    // inline captures such as `{host}` heavily, and `.to_string()` would emit
    // those braces literally instead of interpolating them. A literal brace in
    // a message must therefore be written `{{`, which `format!` enforces at
    // compile time.
    ($lang:expr, $en:literal, $zh:literal) => {
        match $lang {
            $crate::i18n::Lang::En => format!($en),
            $crate::i18n::Lang::Zh => format!($zh),
        }
    };
    ($lang:expr, $en:literal, $zh:literal, $($arg:tt)*) => {
        match $lang {
            $crate::i18n::Lang::En => format!($en, $($arg)*),
            $crate::i18n::Lang::Zh => format!($zh, $($arg)*),
        }
    };
}

/// Like [`t!`] but yields a `&'static str`, for cases that must not allocate
/// or that feed APIs expecting a borrowed string.
#[macro_export]
macro_rules! ts {
    ($lang:expr, $en:literal, $zh:literal) => {
        match $lang {
            $crate::i18n::Lang::En => $en,
            $crate::i18n::Lang::Zh => $zh,
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn parses_bare_tags_and_full_locales() {
        assert_eq!(Lang::parse("zh"), Some(Lang::Zh));
        assert_eq!(Lang::parse("zh_CN.UTF-8"), Some(Lang::Zh));
        assert_eq!(Lang::parse("zh-Hant"), Some(Lang::Zh));
        assert_eq!(Lang::parse("EN"), Some(Lang::En));
        assert_eq!(Lang::parse("en_GB.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::parse("fr_FR"), None);
        assert_eq!(Lang::parse(""), None);
    }

    #[test]
    fn explicit_flag_wins_over_everything() {
        let env = env_from(&[("LANG", "zh_CN.UTF-8"), ("LLM_VERIFY_LANG", "zh")]);
        assert_eq!(Lang::resolve(Some("en"), &env), Lang::En);
    }

    #[test]
    fn dedicated_variable_beats_the_system_locale() {
        let env = env_from(&[("LLM_VERIFY_LANG", "en"), ("LANG", "zh_CN.UTF-8")]);
        assert_eq!(Lang::resolve(None, &env), Lang::En);
    }

    #[test]
    fn falls_back_to_the_system_locale() {
        let env = env_from(&[("LANG", "zh_CN.UTF-8")]);
        assert_eq!(Lang::resolve(None, &env), Lang::Zh);
        let env = env_from(&[("LC_ALL", "zh_TW")]);
        assert_eq!(Lang::resolve(None, &env), Lang::Zh);
    }

    #[test]
    fn defaults_to_english_when_nothing_is_set_or_understood() {
        assert_eq!(Lang::resolve(None, &env_from(&[])), Lang::En);
        // An unrecognised language must not become Chinese by accident.
        assert_eq!(
            Lang::resolve(None, &env_from(&[("LANG", "de_DE.UTF-8")])),
            Lang::En
        );
        // An unparseable explicit value falls through to the next source.
        let env = env_from(&[("LANG", "zh_CN.UTF-8")]);
        assert_eq!(Lang::resolve(Some("klingon"), &env), Lang::Zh);
    }

    #[test]
    fn the_c_locale_is_not_a_language_choice() {
        assert_eq!(Lang::resolve(None, &env_from(&[("LC_ALL", "C")])), Lang::En);
        assert_eq!(
            Lang::resolve(None, &env_from(&[("LANG", "POSIX")])),
            Lang::En
        );
        // ...and must not mask a real setting further down the list.
        let env = env_from(&[("LC_ALL", "C"), ("LANG", "zh_CN.UTF-8")]);
        assert_eq!(Lang::resolve(None, &env), Lang::Zh);
    }

    #[test]
    fn empty_values_are_skipped_rather_than_matched() {
        let env = env_from(&[("LLM_VERIFY_LANG", ""), ("LANG", "zh_CN.UTF-8")]);
        assert_eq!(Lang::resolve(None, &env), Lang::Zh);
    }

    #[test]
    fn t_macro_selects_and_formats() {
        assert_eq!(t!(Lang::En, "hello", "你好"), "hello");
        assert_eq!(t!(Lang::Zh, "hello", "你好"), "你好");
        assert_eq!(t!(Lang::En, "{} ms", "{} 毫秒", 42), "42 ms");
        assert_eq!(t!(Lang::Zh, "{} ms", "{} 毫秒", 42), "42 毫秒");
        // Named and positional arguments both work.
        assert_eq!(t!(Lang::En, "{a}/{b}", "{a} 比 {b}", a = 1, b = 2), "1/2");
    }

    #[test]
    fn ts_macro_borrows() {
        let s: &'static str = ts!(Lang::Zh, "left", "左");
        assert_eq!(s, "左");
    }

    #[test]
    fn html_lang_is_a_valid_bcp47_tag() {
        assert_eq!(Lang::En.html_lang(), "en");
        assert_eq!(Lang::Zh.html_lang(), "zh-Hans");
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    #[test]
    fn inline_named_capture_resolves_at_the_call_site() {
        // `t!` expands to `format!($literal, ...)` inside the macro body. If
        // macro hygiene stopped format!'s implicit capture from seeing the
        // caller's bindings, every `{name}` in a message would silently render
        // wrong — and there are many of them.
        let host = "api.example.com";
        let n = 7;
        assert_eq!(
            t!(Lang::En, "host {host} has {n}", "主机 {host} 有 {n}"),
            "host api.example.com has 7"
        );
        assert_eq!(
            t!(Lang::Zh, "host {host} has {n}", "主机 {host} 有 {n}"),
            "主机 api.example.com 有 7"
        );
    }

    #[test]
    fn inline_capture_works_alongside_explicit_args() {
        let name = "x";
        assert_eq!(t!(Lang::En, "{name}={}", "{name}={}", 42), "x=42");
    }
}

#[cfg(test)]
mod coverage_tests {
    /// Every user-facing string must exist in both languages.
    ///
    /// This is a source-level check because the failure it guards against is
    /// invisible at runtime in one language: `contract.rs` once shipped a full
    /// set of Chinese probe labels with no English half, and an English run
    /// silently printed Chinese for a third of its probes.
    ///
    /// The rule is structural rather than line-based, because `cargo fmt`
    /// freely splits a `t!` call across lines. A Chinese literal must be
    /// immediately preceded by one of:
    ///   `,` + an English literal — its `t!`/`ts!` partner;
    ///   `(`                      — a lookup-table entry carrying both halves;
    ///   `=>`                     — an explicit per-language match arm;
    ///   `=`                      — a named per-language constant.
    #[test]
    fn every_chinese_literal_has_an_english_partner() {
        let mut offenders = Vec::new();
        for file in source_files() {
            let src = std::fs::read_to_string(&file).unwrap();
            let body = strip_tests(&src);
            for (pos, lit) in string_literals(&body) {
                if !has_cjk(&lit) {
                    continue;
                }
                if !is_translated(&body, pos) {
                    offenders.push(format!(
                        "{}: {}",
                        file.file_name().unwrap().to_string_lossy(),
                        lit.chars().take(50).collect::<String>()
                    ));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "{} untranslated user-facing string(s):\n{}",
            offenders.len(),
            offenders.join("\n")
        );
    }

    /// A Chinese literal is legitimate only in one of four positions:
    ///
    ///   inside a `t!(` / `ts!(` call — its English half is the argument before;
    ///   inside a `const` lookup table — the entry's other field carries English;
    ///   after `=>` — an explicit per-language match arm;
    ///   after `=` — a named per-language constant such as `ZH_BODY`.
    ///
    /// Checking the *enclosing call* rather than merely the previous literal
    /// matters: `ProbeResult::new("jitter", "延迟抖动", G)` also has an English
    /// literal in front of it, and an earlier version of this test passed it.
    fn is_translated(src: &str, pos: usize) -> bool {
        let before = src[..pos].trim_end();
        if before.ends_with("=>") || before.ends_with('=') {
            return true;
        }
        match enclosing_open(src, pos) {
            Some((idx, b'(')) => {
                let head = src[..idx].trim_end();
                head.ends_with("t!") || head.ends_with("ts!") || inside_const_table(src, idx)
            }
            // Directly inside a `[...]` — a table row written without a tuple.
            Some((_, b'[')) => true,
            _ => false,
        }
    }

    /// Byte index and kind of the innermost unclosed delimiter before `pos`.
    fn enclosing_open(src: &str, pos: usize) -> Option<(usize, u8)> {
        let b = src.as_bytes();
        let mut depth = 0i32;
        let mut i = pos;
        while i > 0 {
            i -= 1;
            match b[i] {
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' => {
                    if depth == 0 {
                        return Some((i, b[i]));
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        None
    }

    /// Whether a tuple at `idx` sits inside a `const NAME: &[...] = &[` table,
    /// where each row carries both languages as separate fields.
    fn inside_const_table(src: &str, idx: usize) -> bool {
        matches!(enclosing_open(src, idx), Some((_, b'[')))
    }

    fn source_files() -> Vec<std::path::PathBuf> {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut out = Vec::new();
        for dir in [root.to_string(), format!("{root}/probes")] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("rs")
                    // i18n.rs holds this test's own sample strings.
                    && p.file_name().and_then(|x| x.to_str()) != Some("i18n.rs")
                {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Ideographs *and* CJK punctuation. The punctuation half matters: a
    /// separator literal such as `"、"` or `"："` carries no ideograph, so a
    /// bare `format!("{}：{}", ..)` used to slip past this check and print
    /// full-width punctuation into English reports.
    fn has_cjk(s: &str) -> bool {
        s.chars().any(|c| {
            ('\u{4e00}'..='\u{9fff}').contains(&c)      // ideographs
                || ('\u{3000}'..='\u{303f}').contains(&c) // 、。〈〉《》 …
                || ('\u{ff01}'..='\u{ff65}').contains(&c) // ：（）！？ …
        })
    }

    /// Drop `#[cfg(test)]` modules — fixtures are allowed to be monolingual.
    fn strip_tests(src: &str) -> String {
        match src.find("#[cfg(test)]") {
            Some(i) => src[..i].to_string(),
            None => src.to_string(),
        }
    }

    /// Byte offsets and contents of every string literal, skipping line
    /// comments and **raw** strings.
    ///
    /// Raw strings in this crate are HTML templates and the two per-language
    /// skill bodies — neither is ever a `t!` argument, and treating a raw
    /// string's opening `r#"` as a plain quote made the lookback see `r#`
    /// instead of the `=` that marks a named constant.
    fn string_literals(src: &str) -> Vec<(usize, String)> {
        let b = src.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            // Raw string: `r`, any number of `#`, then a quote.
            if b[i] == b'r' {
                let mut j = i + 1;
                let hash_start = j;
                while j < b.len() && b[j] == b'#' {
                    j += 1;
                }
                if j < b.len() && b[j] == b'"' {
                    let hashes = j - hash_start;
                    let close = format!("\"{}", "#".repeat(hashes));
                    i = match src[j + 1..].find(&close) {
                        Some(k) => j + 1 + k + close.len(),
                        None => b.len(),
                    };
                    continue;
                }
            }
            match b[i] {
                b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                }
                // A char literal, which may itself be a quote. `'"'` in
                // `trim_matches('"')` once opened a phantom string that ran to
                // the next quote hundreds of lines later, and every message in
                // between went unchecked — that is how an untranslated error
                // string survived in `main.rs`. Lifetimes (`'a`) are not char
                // literals, so only advance when a closing quote is really there.
                b'\'' => {
                    let mut j = i + 1;
                    if j < b.len() && b[j] == b'\\' {
                        j += 2;
                    } else {
                        // One UTF-8 scalar, however many bytes it occupies.
                        j += 1;
                        while j < b.len() && (b[j] & 0xC0) == 0x80 {
                            j += 1;
                        }
                    }
                    i = if j < b.len() && b[j] == b'\'' {
                        j + 1
                    } else {
                        i + 1
                    };
                }
                b'"' => {
                    let start = i;
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        i += if b[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                    if let Some(s) = src.get(start..i.min(src.len())) {
                        out.push((start, s.to_string()));
                    }
                }
                _ => i += 1,
            }
        }
        out
    }
}
