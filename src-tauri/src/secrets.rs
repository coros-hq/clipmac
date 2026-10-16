//! Finds secrets (API keys, tokens, passwords, private keys…) in text.
//! Used on copied text directly and on OCR'd screenshot text.

use std::{ops::Range, sync::OnceLock};

use regex::Regex;

/// Text shown in place of a hidden secret. Fixed length so it doesn't leak the real one.
pub const MASK: &str = "••••••••";

#[derive(Debug, Clone)]
pub struct Finding {
    /// Byte range of the secret itself (e.g. just the value in `password: hunter2`).
    pub range: Range<usize>,
    pub label: &'static str,
}

struct Rule {
    label: &'static str,
    re: Regex,
    /// Capture group holding the secret (0 = whole match).
    group: usize,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |label, pattern: &str, group| Rule { label, re: Regex::new(pattern).expect("valid secret pattern"), group };
        vec![
            r("Private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----", 0),
            r("API key", r"\bsk-(?:ant-|proj-)?[A-Za-z0-9_\-]{20,}", 0),
            r("AWS key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", 0),
            r("GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})", 0),
            r("Slack token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}", 0),
            r("Google API key", r"\bAIza[0-9A-Za-z_\-]{30,}", 0),
            r("Stripe key", r"\b[sr]k_(?:live|test)_[A-Za-z0-9]{16,}", 0),
            r("npm token", r"\bnpm_[A-Za-z0-9]{30,}", 0),
            r("JWT", r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}", 0),
            r("Bearer token", r"(?i)\bbearer\s+([A-Za-z0-9._~+/\-]{20,}=*)", 1),
            r("Password", r#"(?i)\b(?:pass(?:word|wd|phrase)?|pwd)\b["']?\s*[:=]\s*["']?([^\s"',;]{3,})"#, 1),
            r(
                "Secret",
                r#"(?i)\b[a-z0-9_\-]*(?:secret|token|api[_\-\s]?key|access[_\-\s]?key|private[_\-\s]?key|auth[_\-]?key)\b["']?\s*[:=]\s*["']?([^\s"',;]{6,})"#,
                1,
            ),
            r("Credentials in URL", r"[a-z][a-z0-9+.\-]*://[^\s:/@]+:([^\s@/]{3,})@", 1),
        ]
    })
}

fn long_token() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9_\-+/=]{32,}").expect("valid token pattern"))
}

fn entropy(s: &str) -> f64 {
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    let n = s.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// A long, random-looking string that mixes cases and digits. Pure hex (git hashes) is skipped.
fn looks_random(tok: &str) -> bool {
    let has = |f: fn(&char) -> bool| tok.chars().any(|c| f(&c));
    has(char::is_ascii_digit)
        && has(char::is_ascii_uppercase)
        && has(char::is_ascii_lowercase)
        && !tok.chars().all(|c| c.is_ascii_hexdigit())
        && entropy(tok) >= 4.0
}

/// Cleans up OCR output before scanning: look-alike Cyrillic/Greek letters become Latin and
/// typographic dashes become `-`. Strictly one char for one char, so UTF-16 offsets (which
/// Vision uses to locate text) are unchanged.
pub fn normalize_ocr(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            'А' | 'Α' => 'A',
            'В' | 'Β' => 'B',
            'Е' | 'Ε' => 'E',
            'К' | 'Κ' => 'K',
            'М' | 'Μ' => 'M',
            'Н' | 'Η' => 'H',
            'О' | 'Ο' => 'O',
            'Р' | 'Ρ' => 'P',
            'С' => 'C',
            'Т' | 'Τ' => 'T',
            'Х' | 'Χ' => 'X',
            'Υ' => 'Y',
            'Ι' | 'І' => 'I',
            'Ζ' => 'Z',
            'а' => 'a',
            'е' => 'e',
            'о' | 'ο' => 'o',
            'р' => 'p',
            'с' => 'c',
            'х' => 'x',
            'у' => 'y',
            'і' => 'i',
            c => c,
        })
        .collect()
}

/// All secrets in `text`, sorted by position, overlapping ranges merged.
pub fn scan_text(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    for rule in rules() {
        for caps in rule.re.captures_iter(text) {
            if let Some(m) = caps.get(rule.group) {
                found.push(Finding {
                    range: m.range(),
                    label: rule.label,
                });
            }
        }
    }
    for m in long_token().find_iter(text) {
        if looks_random(m.as_str()) {
            found.push(Finding {
                range: m.range(),
                label: "Token",
            });
        }
    }

    found.sort_by_key(|f| (f.range.start, std::cmp::Reverse(f.range.end)));
    let mut merged: Vec<Finding> = Vec::new();
    for f in found {
        match merged.last_mut() {
            Some(last) if f.range.start < last.range.end => {
                last.range.end = last.range.end.max(f.range.end);
                if last.label == "Token" {
                    last.label = f.label; // a specific label beats the generic one
                }
            }
            _ => merged.push(f),
        }
    }
    merged
}

/// `text` with every secret replaced by [`MASK`].
pub fn mask_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for f in scan_text(text) {
        out.push_str(&text[at..f.range.start]);
        out.push_str(MASK);
        at = f.range.end;
    }
    out.push_str(&text[at..]);
    out
}

/// Distinct labels, in order of first appearance.
pub fn labels(findings: &[Finding]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in findings {
        if !out.iter().any(|l| l == f.label) {
            out.push(f.label.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(s: &str) -> String {
        mask_text(s)
    }

    #[test]
    fn password_value_only() {
        assert_eq!(
            masked("user: bob\npassword: hunter2"),
            format!("user: bob\npassword: {MASK}")
        );
    }

    #[test]
    fn env_style_keys() {
        assert_eq!(masked("API_KEY=abcd1234efgh"), format!("API_KEY={MASK}"));
        assert_eq!(
            masked("export STRIPE_SECRET=\"zzzzzz99\""),
            format!("export STRIPE_SECRET=\"{MASK}\"")
        );
    }

    #[test]
    fn well_known_formats() {
        // Built from pieces so secret scanners don't mistake these fixtures for real tokens.
        for s in [
            concat!("sk-", "ant-api03-AbCdEfGhIjKlMnOpQrStUv"),
            "AKIAIOSFODNN7EXAMPLE",
            concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("xo", "xb-123456789012-abcdefghij"),
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijk",
            concat!("-----BEGIN RSA ", "PRIVATE KEY-----"),
        ] {
            assert!(!scan_text(s).is_empty(), "missed: {s}");
        }
    }

    #[test]
    fn url_credentials_and_bearer() {
        assert_eq!(
            masked("postgres://admin:s3cr3tpw@db.local/app"),
            format!("postgres://admin:{MASK}@db.local/app")
        );
        assert!(masked("Authorization: Bearer abcdefghijklmnopqrstuvwx").contains(MASK));
    }

    #[test]
    fn random_token_but_not_hashes_or_prose() {
        assert!(!scan_text("Zk3pQ9vLmN8xRt2YbW7cHd4JfG6sAe1U").is_empty());
        assert!(scan_text("da39a3ee5e6b4b0d3255bfef95601890afd80709").is_empty()); // git sha
        assert!(
            scan_text("The quick brown fox jumps over the lazy dog, again and again.").is_empty()
        );
        assert!(
            scan_text("https://developer.apple.com/design/human-interface-guidelines/materials")
                .is_empty()
        );
    }

    #[test]
    fn ocr_lookalikes_are_normalized() {
        // Real OCR output from a screenshot: Cyrillic Р/А/Е/М and an em dash.
        let ocr = "0РENАI_АРI_KЕY=sk-proj—АbCdEfGhIjKlМn0pQrStUvWx1234";
        let clean = normalize_ocr(ocr);
        assert_eq!(clean.chars().count(), ocr.chars().count());
        assert_eq!(clean, "0PENAI_API_KEY=sk-proj-AbCdEfGhIjKlMn0pQrStUvWx1234");
        assert!(!scan_text(&clean).is_empty());
    }

    #[test]
    fn plain_text_is_untouched() {
        let s = "Meeting notes — ship the clipboard MVP by end of month";
        assert!(scan_text(s).is_empty());
        assert_eq!(masked(s), s);
    }
}
