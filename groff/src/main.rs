//! groff/nroff 実装（manページレンダリング向け）
//!
//! roff 言語のコア（レジスタ・文字列・マクロ定義・条件・式評価・エスケープ）と
//! man マクロパッケージを実装し、GNU groff -man -Tutf8 に近い出力を生成する。

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use glob::glob;

// ============================================================================
// 設定
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum Device {
    Ascii,
    Latin1,
    Utf8,
    Html,
}

#[derive(Debug)]
struct Config {
    files: Vec<String>,
    device: Device,
    /// SGR エスケープで装飾するか（-c / GROFF_NO_SGR で無効化）
    styled: bool,
    /// -r で設定するレジスタ（名前, 式文字列）
    cli_registers: Vec<(String, String)>,
    /// -d で設定する文字列
    cli_strings: Vec<(String, String)>,
    /// -z: 出力を抑制（解析のみ）
    suppress_output: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            files: Vec::new(),
            device: Device::Utf8,
            styled: true,
            cli_registers: Vec::new(),
            cli_strings: Vec::new(),
            suppress_output: false,
        }
    }
}

// ============================================================================
// 単位と式評価
// ============================================================================
// 内部単位は groff -Tutf8 と同じ基本単位 u（1文字 = 24u、1行 = 40u、1インチ = 240u）

const UNITS_PER_CHAR: f64 = 24.0;
const UNITS_PER_LINE: f64 = 40.0;

fn scale_factor(c: char) -> Option<f64> {
    match c {
        'u' => Some(1.0),
        'n' | 'm' => Some(UNITS_PER_CHAR),
        'v' => Some(UNITS_PER_LINE),
        'i' => Some(240.0),
        'c' => Some(240.0 / 2.54),
        'p' => Some(240.0 / 72.0),
        'P' => Some(240.0 / 6.0),
        's' | 'z' => Some(1.0),
        _ => None,
    }
}

/// troff 数式を評価して基本単位で返す。
/// troff と同様、演算子に優先順位はなく左から右へ評価する。
fn eval_expr(s: &str, default_scale: f64) -> Option<f64> {
    let chars: Vec<char> = s.trim().chars().collect();
    let mut pos = 0;
    let v = parse_expr(&chars, &mut pos, default_scale)?;
    Some(v)
}

fn parse_expr(chars: &[char], pos: &mut usize, ds: f64) -> Option<f64> {
    let mut left = parse_term(chars, pos, ds)?;
    loop {
        skip_spaces(chars, pos);
        if *pos >= chars.len() {
            break;
        }
        let op = chars[*pos];
        let two = |c: char| *pos + 1 < chars.len() && chars[*pos + 1] == c;
        match op {
            '+' => { *pos += 1; let r = parse_term(chars, pos, ds)?; left += r; }
            '-' => { *pos += 1; let r = parse_term(chars, pos, ds)?; left -= r; }
            '*' => { *pos += 1; let r = parse_term(chars, pos, ds)?; left *= r; }
            '/' => {
                *pos += 1;
                let r = parse_term(chars, pos, ds)?;
                if r == 0.0 { return None; }
                left /= r;
            }
            '%' => {
                *pos += 1;
                let r = parse_term(chars, pos, ds)?;
                if r == 0.0 { return None; }
                left = (left as i64 % r as i64) as f64;
            }
            '<' => {
                if two('=') { *pos += 2; let r = parse_term(chars, pos, ds)?; left = if left <= r { 1.0 } else { 0.0 }; }
                else { *pos += 1; let r = parse_term(chars, pos, ds)?; left = if left < r { 1.0 } else { 0.0 }; }
            }
            '>' => {
                if two('=') { *pos += 2; let r = parse_term(chars, pos, ds)?; left = if left >= r { 1.0 } else { 0.0 }; }
                else { *pos += 1; let r = parse_term(chars, pos, ds)?; left = if left > r { 1.0 } else { 0.0 }; }
            }
            '=' => {
                *pos += if two('=') { 2 } else { 1 };
                let r = parse_term(chars, pos, ds)?;
                left = if (left - r).abs() < 0.001 { 1.0 } else { 0.0 };
            }
            '&' => { *pos += 1; let r = parse_term(chars, pos, ds)?; left = if left > 0.0 && r > 0.0 { 1.0 } else { 0.0 }; }
            ':' => { *pos += 1; let r = parse_term(chars, pos, ds)?; left = if left > 0.0 || r > 0.0 { 1.0 } else { 0.0 }; }
            _ => break,
        }
    }
    Some(left)
}

fn parse_term(chars: &[char], pos: &mut usize, ds: f64) -> Option<f64> {
    skip_spaces(chars, pos);
    if *pos >= chars.len() {
        return None;
    }
    match chars[*pos] {
        '-' => { *pos += 1; Some(-parse_term(chars, pos, ds)?) }
        '+' => { *pos += 1; parse_term(chars, pos, ds) }
        '!' => { *pos += 1; let v = parse_term(chars, pos, ds)?; Some(if v > 0.0 { 0.0 } else { 1.0 }) }
        '(' => {
            *pos += 1;
            let v = parse_expr(chars, pos, ds)?;
            skip_spaces(chars, pos);
            if *pos < chars.len() && chars[*pos] == ')' { *pos += 1; }
            Some(v)
        }
        c if c.is_ascii_digit() || c == '.' => {
            let mut num = String::new();
            while *pos < chars.len() && (chars[*pos].is_ascii_digit() || chars[*pos] == '.') {
                num.push(chars[*pos]);
                *pos += 1;
            }
            let base: f64 = num.parse().ok()?;
            let scale = if *pos < chars.len() {
                if let Some(f) = scale_factor(chars[*pos]) { *pos += 1; f } else { ds }
            } else {
                ds
            };
            Some(base * scale)
        }
        _ => None,
    }
}

fn skip_spaces(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos] == ' ' {
        *pos += 1;
    }
}

// ============================================================================
// 文字幅
// ============================================================================

fn char_width(c: char) -> usize {
    // 内部マーカー（\& \% 由来）とゼロ幅スペースは幅 0
    if c == '\u{200B}' || c == '\u{E000}' || c == '\u{E001}' {
        return 0;
    }
    if is_wide_char(c) { 2 } else { 1 }
}

fn is_wide_char(c: char) -> bool {
    let cp = c as u32;
    (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0x9FFF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE10..=0xFE1F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0x20000..=0x2FFFF).contains(&cp)
}

/// SGR エスケープを除いた表示幅
fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' { in_escape = false; }
            continue;
        }
        if c == '\x1b' { in_escape = true; continue; }
        width += char_width(c);
    }
    width
}

// ============================================================================
// 特殊文字
// ============================================================================

fn special_char(name: &str, dev: Device) -> Option<String> {
    // \[uXXXX] 形式
    if let Some(hex) = name.strip_prefix('u') {
        if hex.len() >= 4 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                if let Some(c) = char::from_u32(cp) {
                    return Some(c.to_string());
                }
            }
        }
    }

    let unicode = match name {
        "em" => "\u{2014}", "en" => "\u{2013}", "hy" => "-",
        "bu" => "\u{2022}", "ci" => "\u{25CB}", "sq" => "\u{25A1}",
        "lq" => "\u{201C}", "rq" => "\u{201D}", "oq" => "\u{2018}", "cq" => "\u{2019}",
        "aq" => "'", "dq" => "\"", "Fo" => "\u{00AB}", "Fc" => "\u{00BB}",
        "fo" => "\u{2039}", "fc" => "\u{203A}",
        "co" => "\u{00A9}", "rg" => "\u{00AE}", "tm" => "\u{2122}",
        "rs" => "\\", "ti" => "~", "ha" => "^", "ga" => "`", "at" => "@", "sh" => "#",
        "Do" => "$", "sl" => "/", "lB" => "[", "rB" => "]", "lC" => "{", "rC" => "}",
        "pl" => "+", "mi" => "\u{2212}", "mu" => "\u{00D7}", "di" => "\u{00F7}",
        "eq" => "=", ">=" | "ge" => "\u{2265}", "<=" | "le" => "\u{2264}",
        "!=" => "\u{2260}", "==" => "\u{2261}", "+-" => "\u{00B1}",
        "if" => "\u{221E}", "de" => "\u{00B0}", "mc" => "\u{00B5}",
        "12" => "\u{00BD}", "14" => "\u{00BC}", "34" => "\u{00BE}",
        "la" => "\u{27E8}", "ra" => "\u{27E9}",
        "<-" => "\u{2190}", "->" => "\u{2192}",
        "ua" => "\u{2191}", "da" => "\u{2193}", "<>" => "\u{2194}",
        "lA" => "\u{21D0}", "rA" => "\u{21D2}", "uA" => "\u{21D1}",
        "dA" => "\u{21D3}", "hA" => "\u{21D4}",
        "sc" => "\u{00A7}", "ps" => "\u{00B6}", "dg" => "\u{2020}", "dd" => "\u{2021}",
        "OK" => "\u{2713}", "ss" => "\u{00DF}",
        "aa" => "\u{00B4}", "'" => "\u{00B4}", "`" => "`",
        "cent" | "ct" => "\u{00A2}", "Po" => "\u{00A3}", "Ye" => "\u{00A5}", "Eu" => "\u{20AC}",
        "no" | "tno" => "\u{00AC}", "%0" => "\u{2030}",
        "fm" => "\u{2032}", "sd" => "\u{2033}",
        "lh" => "\u{261C}", "rh" => "\u{261E}",
        "SP" => " ",
        _ => return None,
    };

    if dev == Device::Utf8 || dev == Device::Html {
        return Some(unicode.to_string());
    }

    // ASCII / Latin1 近似
    let ascii = match name {
        "em" => "--", "en" => "-", "bu" => "o", "ci" => "O", "sq" => "[]",
        "lq" | "rq" => "\"", "oq" => "`", "cq" => "'",
        "co" => "(C)", "rg" => "(R)", "tm" => "(tm)",
        "mi" => "-", "mu" => "x", "di" => "/",
        ">=" | "ge" => ">=", "<=" | "le" => "<=", "!=" => "!=", "==" => "==",
        "+-" => "+-", "if" => "inf", "de" => "deg", "mc" => "u",
        "12" => "1/2", "14" => "1/4", "34" => "3/4",
        "la" => "<", "ra" => ">",
        "<-" => "<-", "->" => "->", "ua" => "^", "da" => "v", "<>" => "<->",
        "lA" => "<=", "rA" => "=>", "uA" => "^", "dA" => "v", "hA" => "<=>",
        "dg" => "+", "dd" => "++", "OK" => "OK", "ss" => "ss",
        "aa" | "'" => "'", "fm" => "'", "sd" => "\"",
        "cent" | "ct" => "c", "Po" => "GBP", "Ye" => "JPY", "Eu" => "EUR",
        "no" | "tno" => "not", "sc" => "S:", "ps" => "P:", "%0" => "0/00",
        "lh" | "rh" => "=>",
        _ => {
            // Latin1 で表現できるものはそのまま
            if dev == Device::Latin1 && unicode.chars().all(|c| (c as u32) < 256) {
                return Some(unicode.to_string());
            }
            unicode
        }
    };
    Some(ascii.to_string())
}

fn default_section_name(sec: &str) -> &'static str {
    match sec.chars().next() {
        Some('1') => "General Commands Manual",
        Some('2') => "System Calls Manual",
        Some('3') => "Library Functions Manual",
        Some('4') => "Kernel Interfaces Manual",
        Some('5') => "File Formats Manual",
        Some('6') => "Games Manual",
        Some('7') => "Miscellaneous Information Manual",
        Some('8') => "System Manager's Manual",
        Some('9') => "Kernel Developer's Manual",
        _ => "",
    }
}

/// 現在日付 (年, 月, 日)（エポック秒から civil 変換）
fn today_ymd() -> (i64, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs / 86400;
    // Howard Hinnant の civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ============================================================================
// ハイフネーション（Knuth-Liang アルゴリズム）
// ============================================================================
// パターンデータは groff 1.23.0 収録の hyphen.en / hyphenex.en
// （Gerard D.C. Kuiken による米語パターン。著作権表示はデータファイル内に保持）

static HYPHEN_PATTERNS: &str = include_str!("hyphen_en.txt");
static HYPHEN_EXCEPTIONS: &str = include_str!("hyphenex_en.txt");

struct Hyphenator {
    patterns: HashMap<String, Vec<u8>>,
    exceptions: HashMap<String, Vec<usize>>,
    max_len: usize,
}

impl Hyphenator {
    fn instance() -> &'static Hyphenator {
        use std::sync::OnceLock;
        static H: OnceLock<Hyphenator> = OnceLock::new();
        H.get_or_init(Hyphenator::load)
    }

    fn load() -> Hyphenator {
        #[derive(PartialEq)]
        enum Mode {
            None,
            Patterns,
            Exceptions,
        }

        let mut patterns = HashMap::new();
        let mut exceptions = HashMap::new();
        let mut max_len = 0;

        // 両ファイルとも \patterns{...} と \hyphenation{...} を含み得る
        // （hyphen.en は末尾に「分綴しない語」の例外ブロックを持つ）
        for source in [HYPHEN_PATTERNS, HYPHEN_EXCEPTIONS] {
            let mut mode = Mode::None;
            for line in source.lines() {
                let line = line.split('%').next().unwrap_or("");
                for tok in line.split_whitespace() {
                    if tok.starts_with("\\patterns{") {
                        mode = Mode::Patterns;
                        continue;
                    }
                    if tok.starts_with("\\hyphenation{") {
                        mode = Mode::Exceptions;
                        continue;
                    }
                    if tok == "}" {
                        mode = Mode::None;
                        continue;
                    }
                    match mode {
                        Mode::Patterns => {
                            let mut letters = String::new();
                            let mut weights: Vec<u8> = vec![0];
                            for c in tok.chars() {
                                if let Some(d) = c.to_digit(10) {
                                    *weights.last_mut().unwrap() = d as u8;
                                } else {
                                    letters.push(c);
                                    weights.push(0);
                                }
                            }
                            max_len = max_len.max(letters.chars().count());
                            patterns.insert(letters, weights);
                        }
                        Mode::Exceptions => {
                            let mut word = String::new();
                            let mut breaks = Vec::new();
                            for c in tok.chars() {
                                if c == '-' {
                                    breaks.push(word.chars().count());
                                } else {
                                    word.push(c);
                                }
                            }
                            // ハイフンなしの例外 = 分綴禁止（空の分綴点リスト）
                            exceptions.insert(word, breaks);
                        }
                        Mode::None => {}
                    }
                }
            }
        }

        Hyphenator { patterns, exceptions, max_len }
    }

    /// 単語（小文字英字のみ）の分綴可能位置（文字数）を返す
    fn hyphenate(&self, word: &str) -> Vec<usize> {
        let wlen = word.chars().count();
        if wlen < 5 {
            return Vec::new();
        }
        if let Some(b) = self.exceptions.get(word) {
            return b.clone();
        }

        let marked: Vec<char> = std::iter::once('.')
            .chain(word.chars())
            .chain(std::iter::once('.'))
            .collect();
        let m = marked.len();
        let mut points = vec![0u8; m + 1];

        for i in 0..m {
            let mut sub = String::new();
            for j in i..m.min(i + self.max_len) {
                sub.push(marked[j]);
                if let Some(w) = self.patterns.get(&sub) {
                    for (k, &wt) in w.iter().enumerate() {
                        if points[i + k] < wt {
                            points[i + k] = wt;
                        }
                    }
                }
            }
        }

        // 左 2 文字・右 3 文字は分綴しない（TeX / groff デフォルト）
        (2..=wlen.saturating_sub(3))
            .filter(|&c| points[c + 1] % 2 == 1)
            .collect()
    }
}

/// ワードの分綴候補（テキスト内の文字位置と、ハイフン付加が必要か）を返す。
/// `allow_patterns` が偽のときは明示ハイフンでの分割のみ
/// （troff では .hy 0 でも明示ハイフンの改行は行われる）。
fn hyphen_break_points(text: &str, allow_patterns: bool) -> Vec<(usize, bool)> {
    // SGR エスケープ入りのワードは分綴しない（分割時のスタイル管理が複雑になるため）
    if text.contains('\x1b') {
        return Vec::new();
    }
    // \% マーカー付きの語は分綴しない
    if text.contains('\u{E001}') {
        return Vec::new();
    }

    let chars: Vec<char> = text.chars().collect();
    let first_alpha = match chars.iter().position(|c| c.is_alphabetic()) {
        Some(p) => p,
        None => return Vec::new(),
    };

    // 先頭の英字より前にハイフンがある語（--option 等）は
    // 明示ハイフンでは分割しない（パターン分綴のみ許可）
    let leading_dash = chars[..first_alpha].contains(&'-');

    let mut points = Vec::new();

    // 明示ハイフンでの分割（MS-Windows → MS-/Windows）。
    // 数字間や記号隣接のハイフンでは分割しない。.hy 0 でも有効（troff の挙動）。
    // 先頭が - の語（--option 等）では行わない。
    if !leading_dash {
        for k in first_alpha + 1..chars.len().saturating_sub(1) {
            if chars[k] == '-' && chars[k - 1].is_alphabetic() && chars[k + 1].is_alphabetic() {
                points.push((k + 1, false));
            }
        }
    }

    if allow_patterns {
        // 英字の連続区間ごとに Liang パターンを適用
        let mut j = 0;
        while j < chars.len() {
            if !chars[j].is_ascii_alphabetic() {
                j += 1;
                continue;
            }
            let rs = j;
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let run: String = chars[rs..j].iter().collect();
            let lower = run.to_ascii_lowercase();
            for p in Hyphenator::instance().hyphenate(&lower) {
                points.push((rs + p, true));
            }
        }
    }

    points.sort();
    points
}

/// 分綴点リストから space 幅に収まる最大の分割を行う。
/// 戻り値は (前半ワード, 残りワード, 残りに対する分綴点)。
fn split_at_points(
    w: &Word,
    points: &[(usize, bool)],
    space: usize,
    device: Device,
) -> Option<(Word, Word, Vec<(usize, bool)>)> {
    let mut best: Option<(usize, bool)> = None;
    for &(pos, add_hyphen) in points {
        let head_width: usize = w.text.chars().take(pos).map(char_width).sum::<usize>()
            + if add_hyphen { 1 } else { 0 };
        if head_width <= space && best.map(|(p, _)| pos > p).unwrap_or(true) {
            best = Some((pos, add_hyphen));
        }
    }
    let (pos, add_hyphen) = best?;

    let byte = w
        .text
        .char_indices()
        .nth(pos)
        .map(|(b, _)| b)
        .unwrap_or(w.text.len());
    let mut head_text = w.text[..byte].to_string();
    let rest_text = w.text[byte..].to_string();
    if rest_text.is_empty() {
        return None;
    }
    if add_hyphen {
        head_text.push(if device == Device::Utf8 { '\u{2010}' } else { '-' });
    }
    let head_width = head_text.chars().map(char_width).sum();
    let rest_width = rest_text.chars().map(char_width).sum();
    let rest_points: Vec<(usize, bool)> = points
        .iter()
        .filter(|&&(p, _)| p > pos)
        .map(|&(p, a)| (p - pos, a))
        .collect();
    Some((
        Word {
            text: head_text,
            width: head_width,
            gap: 0,
            sentence_end: false,
            adjustable: true,
        },
        Word {
            text: rest_text,
            width: rest_width,
            gap: 0,
            sentence_end: w.sentence_end,
            adjustable: true,
        },
        rest_points,
    ))
}

/// ワード末尾が文末（. ! ?）かどうか。
/// groff と同様、末尾の \&（ゼロ幅文字 = マーカー U+E000）は文末を打ち消す。
fn is_sentence_end(text: &str) -> bool {
    let plain = strip_sgr(text);
    if plain.ends_with('\u{E000}') {
        return false;
    }
    let trimmed = plain.trim_end_matches(['"', '\'', ')', ']', '*', '\u{2019}', '\u{201D}']);
    matches!(trimmed.chars().last(), Some('.') | Some('!') | Some('?'))
}

// ============================================================================
// 出力ワード
// ============================================================================

#[derive(Debug, Clone)]
struct Word {
    text: String,
    width: usize,
    /// 直前のワードとの間隔（0 = 密着）
    gap: usize,
    /// このワードで文が終わる（次のワードとの間隔を 2 にする）
    sentence_end: bool,
    /// 間隔を調整（justify）で伸ばしてよいか
    adjustable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Adjust {
    Left,
    Both,
    Center,
    Right,
}

// ============================================================================
// フォーマッタ本体
// ============================================================================

struct Troff {
    device: Device,
    styled: bool,

    // 出力
    out: Vec<String>,
    line: Vec<Word>,
    cur_width: usize,

    // 環境
    ll: usize,
    indent: usize,
    temp_indent: Option<usize>,
    fill: bool,
    adjust: Adjust,
    ce_count: usize,
    ul_count: usize,
    font: char,
    prev_font: char,
    line_spacing: usize,
    /// 文末の追加スペース数（.ss の第2引数で 0 にできる。デフォルト 1 = 計2つ）
    sentence_gap_extra: usize,
    /// これまでに両端揃えした行数（余白の配分方向を交互にするため）
    adjusted_lines: usize,
    /// ハイフネーション有効
    hy: bool,
    /// 直前の出力が .SH/.SS 見出しか（.HP の空行の癖の再現用）
    after_heading: bool,

    // 定義
    strings: HashMap<String, String>,
    macros: HashMap<String, Vec<String>>,
    registers: HashMap<String, i64>,
    reg_steps: HashMap<String, i64>,

    // man 状態
    base_indent: usize,
    prevail_indent: usize,
    rs_stack: Vec<usize>,
    pd: usize,
    th: Option<(String, String, String, String, String)>,
    tag_pending: Option<usize>,
    tag_words: Vec<Word>,
    next_line_font: Option<char>,
    pending_heading: Option<usize>,
    link_url: Option<String>,

    // 制御
    el_stack: Vec<bool>,
    no_space: bool,
    /// render 中に \c を見た（この行の末尾で次行と連結する）
    pending_connect: bool,
    /// 前の行が \c で終わった（この行の先頭を前の行に連結する）
    connect_active: bool,
    diverting: bool,
    exited: bool,

    // 入力コンテキスト（.so の相対パス解決用）
    file_dirs: Vec<PathBuf>,

    // タブストップ（文字位置）
    tab_stops: Vec<usize>,
}

struct LineStream {
    lines: Vec<String>,
    pos: usize,
}

impl LineStream {
    fn new(lines: Vec<String>) -> Self {
        LineStream { lines, pos: 0 }
    }
    fn next_line(&mut self) -> Option<String> {
        if self.pos >= self.lines.len() {
            return None;
        }
        let mut l = self.lines[self.pos].clone();
        self.pos += 1;
        // 行末の \（エスケープされた改行）は行継続
        while ends_with_continuation(&l) && self.pos < self.lines.len() {
            l.pop();
            l.push_str(&self.lines[self.pos]);
            self.pos += 1;
        }
        Some(l)
    }
}

/// 行末のバックスラッシュが奇数個（= エスケープされた改行）か
fn ends_with_continuation(s: &str) -> bool {
    let n = s.chars().rev().take_while(|&c| c == '\\').count();
    n % 2 == 1
}

#[derive(PartialEq)]
enum Flow {
    Normal,
    Return,
}

impl Troff {
    fn new(config: &Config) -> Self {
        let mut strings = HashMap::new();
        strings.insert("R".to_string(), "\\(rg".to_string());
        strings.insert("Tm".to_string(), "\\(tm".to_string());
        strings.insert("lq".to_string(), "\\(lq".to_string());
        strings.insert("rq".to_string(), "\\(rq".to_string());
        strings.insert("S".to_string(), String::new());
        strings.insert(
            ".T".to_string(),
            match config.device {
                Device::Ascii => "ascii",
                Device::Latin1 => "latin1",
                Device::Utf8 => "utf8",
                Device::Html => "html",
            }
            .to_string(),
        );
        for (name, val) in &config.cli_strings {
            strings.insert(name.clone(), val.clone());
        }

        let mut registers = HashMap::new();
        let (y, m, d) = today_ymd();
        registers.insert("yr".to_string(), y - 1900);
        registers.insert("mo".to_string(), m as i64);
        registers.insert("dy".to_string(), d as i64);
        registers.insert("%".to_string(), 1);
        registers.insert(".g".to_string(), 1);
        // groff の man パッケージが定義するハイフネーション制御レジスタ
        registers.insert("HY".to_string(), 1);

        let mut ll = 78usize;
        for (name, expr) in &config.cli_registers {
            if let Some(v) = eval_expr(expr, 1.0) {
                registers.insert(name.clone(), v as i64);
                if name == "LL" {
                    let chars = (v / UNITS_PER_CHAR).round() as usize;
                    if chars > 0 {
                        ll = chars;
                    }
                }
            }
        }

        Troff {
            device: config.device,
            styled: config.styled && config.device != Device::Html,
            out: Vec::new(),
            line: Vec::new(),
            cur_width: 0,
            ll,
            indent: 0,
            temp_indent: None,
            fill: true,
            adjust: Adjust::Both,
            ce_count: 0,
            ul_count: 0,
            font: 'R',
            prev_font: 'R',
            line_spacing: 1,
            sentence_gap_extra: 1,
            adjusted_lines: 0,
            hy: true,
            after_heading: false,
            strings,
            macros: HashMap::new(),
            registers,
            reg_steps: HashMap::new(),
            base_indent: 7,
            prevail_indent: 7,
            rs_stack: Vec::new(),
            pd: 1,
            th: None,
            tag_pending: None,
            tag_words: Vec::new(),
            next_line_font: None,
            pending_heading: None,
            link_url: None,
            el_stack: Vec::new(),
            no_space: false,
            pending_connect: false,
            connect_active: false,
            diverting: false,
            exited: false,
            file_dirs: Vec::new(),
            // groff のデフォルトタブ幅は 0.5 インチ（TTY では 5 桁）
            tab_stops: (1..=40).map(|i| i * 5).collect(),
        }
    }

    // ------------------------------------------------------------------
    // 出力エンジン
    // ------------------------------------------------------------------

    fn effective_indent(&self) -> usize {
        self.temp_indent.unwrap_or(self.indent)
    }

    fn available(&self) -> usize {
        self.ll.saturating_sub(self.effective_indent())
    }

    fn push_output(&mut self, s: String) {
        self.out.push(s);
        for _ in 1..self.line_spacing {
            self.out.push(String::new());
        }
    }

    /// 現在の行バッファを出力する
    fn flush(&mut self, justify: bool) {
        if self.line.is_empty() {
            self.temp_indent = None;
            return;
        }
        let ind = self.effective_indent();
        self.temp_indent = None;
        let words = std::mem::take(&mut self.line);
        self.cur_width = 0;

        let total_width: usize = words.iter().map(|w| w.width + w.gap).sum();

        let mut gaps: Vec<usize> = words.iter().map(|w| w.gap).collect();

        match self.adjust {
            Adjust::Both if justify && self.fill => {
                let avail = self.ll.saturating_sub(ind);
                if total_width < avail {
                    let mut extra = avail - total_width;
                    // タグ直後の固定間隔は伸縮させない（前後とも adjustable な間隔のみ）
                    let slots: Vec<usize> = (1..words.len())
                        .filter(|&i| words[i].adjustable && words[i - 1].adjustable && words[i].gap > 0)
                        .collect();
                    if !slots.is_empty() {
                        // 行ごとに配分方向を交互にして偏りを避ける（groff 互換）
                        let from_right = self.adjusted_lines % 2 == 1;
                        self.adjusted_lines += 1;
                        let n = slots.len();
                        let per = extra / n;
                        let mut rem = extra % n;
                        extra = 0;
                        let _ = extra;
                        for k in 0..n {
                            let idx = if from_right { slots[n - 1 - k] } else { slots[k] };
                            gaps[idx] += per + if rem > 0 { rem -= 1; 1 } else { 0 };
                        }
                    }
                }
            }
            _ => {}
        }

        let mut text = String::new();
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                text.push_str(&" ".repeat(gaps[i]));
            }
            text.push_str(&w.text);
        }

        // マーカーだけの行（\& 単独など）はインデントなしの空行として出力
        if total_width == 0 && text.chars().all(|c| c == '\u{E000}' || c == '\u{E001}') {
            self.push_output(String::new());
            self.no_space = false;
            self.after_heading = false;
            return;
        }

        let lead = match self.adjust {
            Adjust::Center => ind + (self.ll.saturating_sub(ind).saturating_sub(total_width)) / 2,
            Adjust::Right => self.ll.saturating_sub(total_width),
            _ => ind,
        };
        self.push_output(format!("{}{}", " ".repeat(lead), text));
        self.no_space = false;
        self.after_heading = false;
    }

    fn center_flush(&mut self) {
        let old = self.adjust;
        self.adjust = Adjust::Center;
        self.flush(false);
        self.adjust = old;
    }

    /// n 行の空行を追加する
    fn vspace(&mut self, n: usize) {
        self.flush(false);
        if self.no_space {
            return;
        }
        for _ in 0..n {
            self.out.push(String::new());
        }
    }

    /// 段落マクロの垂直間隔。groff の an.tmac と同様、
    /// .sp 相当の空行を加算し、直後に no-space モードを立てる
    /// （連続する段落マクロの空行は 1 つに集約され、出力があると解除される）。
    fn para_space(&mut self) {
        self.flush(false);
        if !self.no_space && !self.out.is_empty() {
            for _ in 0..self.pd {
                self.out.push(String::new());
            }
        }
        self.no_space = true;
        self.after_heading = false;
    }

    /// 末尾がちょうど n 行の空行になるようにする（段落間隔用）
    #[allow(dead_code)]
    fn ensure_blank(&mut self, n: usize) {
        self.flush(false);
        if self.no_space || self.out.is_empty() {
            return;
        }
        let mut trailing = 0;
        while trailing < self.out.len() && self.out[self.out.len() - 1 - trailing].is_empty() {
            trailing += 1;
        }
        if trailing == self.out.len() {
            return; // 先頭に空行は作らない
        }
        for _ in trailing..n {
            self.out.push(String::new());
        }
        while {
            let len = self.out.len();
            len > 0 && {
                let mut t = 0;
                while t < len && self.out[len - 1 - t].is_empty() {
                    t += 1;
                }
                t > n && t < len
            }
        } {
            self.out.pop();
        }
    }

    /// ワード列を行バッファに流し込む（折り返し処理付き）
    fn emit_words(&mut self, words: Vec<Word>) {
        if words.is_empty() {
            return;
        }
        self.no_space = false;

        if self.tag_pending.is_some() {
            self.tag_words.extend(words);
            return;
        }

        let mut words = words;

        // \c による前の行との連結
        if self.connect_active && !self.line.is_empty() && !words.is_empty() {
            let first = words.remove(0);
            if let Some(last) = self.line.last_mut() {
                last.text.push_str(&first.text);
                last.width += first.width;
                last.sentence_end = first.sentence_end;
                self.cur_width += first.width;
            }
        }
        self.connect_active = false;

        if !self.fill {
            // no-fill: そのまま 1 行として出力（呼び出し側で 1 行単位）
            for w in words {
                let gap = if self.line.is_empty() { 0 } else { w.gap };
                self.cur_width += gap + w.width;
                self.line.push(Word { gap, ..w });
            }
            return;
        }

        for w in words {
            self.place_word(w);
        }
    }

    /// 1 ワードを行バッファに配置する（必要なら折り返し・分綴）。
    /// 分綴点は元の語に対して一度だけ計算し、分割後の残りにも引き継ぐ
    /// （su-per-sedes のような多段分割のため）。
    fn place_word(&mut self, mut w: Word) {
        let mut points: Option<Vec<(usize, bool)>> = None;
        loop {
            let gap = if self.line.is_empty() {
                0
            } else if self.line.last().map(|p| p.sentence_end).unwrap_or(false) {
                w.gap.max(1 + self.sentence_gap_extra)
            } else {
                w.gap
            };
            let avail = self.available();
            if self.cur_width + gap + w.width <= avail {
                w.gap = gap;
                self.cur_width += gap + w.width;
                self.line.push(w);
                return;
            }

            // 分綴を試みる
            let space = avail.saturating_sub(self.cur_width + gap);
            if self.fill && space >= 2 {
                let pts = points
                    .get_or_insert_with(|| hyphen_break_points(&w.text, self.hy));
                if let Some((mut head, rest, rest_points)) = split_at_points(&w, pts, space, self.device) {
                    head.gap = gap;
                    self.cur_width += gap + head.width;
                    self.line.push(head);
                    self.flush(true);
                    w = rest;
                    points = Some(rest_points);
                    continue;
                }
            }

            if self.line.is_empty() {
                // 行頭でも収まらない → そのまま置く
                w.gap = 0;
                self.cur_width = w.width;
                self.line.push(w);
                return;
            }
            self.flush(true);
            w.gap = 0;
        }
    }

    // ------------------------------------------------------------------
    // フォントとスタイル
    // ------------------------------------------------------------------

    fn style_on(&self, font: char) -> &'static str {
        if !self.styled {
            return "";
        }
        match font {
            'B' => "\x1b[1m",
            'I' => "\x1b[4m",
            'X' => "\x1b[1m\x1b[4m", // BI
            _ => "",
        }
    }

    fn style_off(&self, font: char) -> &'static str {
        if !self.styled {
            return "";
        }
        match font {
            'B' | 'I' | 'X' => "\x1b[0m",
            _ => "",
        }
    }

    fn map_font_name(name: &str) -> char {
        let has_b = name.contains('B');
        let has_i = name.contains('I');
        match (has_b, has_i) {
            (true, true) => 'X',
            (true, false) => 'B',
            (false, true) => 'I',
            _ => match name {
                "1" => 'R',
                "2" => 'I',
                "3" => 'B',
                "4" => 'X',
                "P" => 'P',
                _ => 'R',
            },
        }
    }

    // ------------------------------------------------------------------
    // テキスト → ワード列（エスケープ処理）
    // ------------------------------------------------------------------

    /// テキスト行をワード列に変換する。フォント状態は self.font を引き継ぐ。
    #[allow(unused_assignments)] // end_word! マクロの最終呼び出しで未読の代入が残る
    fn render_words(&mut self, text: &str) -> Vec<Word> {
        let mut words = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;

        let mut cur = String::new();
        let mut cur_width = 0usize;
        let mut has_content = false;
        let mut style_open = false;
        let mut gap_next = 0usize;

        macro_rules! open_style {
            () => {
                if !style_open && self.font != 'R' {
                    cur.push_str(self.style_on(self.font));
                    style_open = true;
                }
            };
        }
        macro_rules! push_char {
            ($c:expr) => {{
                open_style!();
                cur.push($c);
                cur_width += char_width($c);
                has_content = true;
            }};
        }
        macro_rules! push_str_w {
            ($s:expr) => {{
                let s: &str = $s;
                if !s.is_empty() {
                    open_style!();
                    cur.push_str(s);
                    cur_width += s.chars().map(char_width).sum::<usize>();
                    has_content = true;
                }
            }};
        }
        macro_rules! end_word {
            ($gap:expr) => {{
                if style_open {
                    cur.push_str(self.style_off(self.font));
                    style_open = false;
                }
                if has_content || !cur.is_empty() {
                    words.push(Word {
                        text: std::mem::take(&mut cur),
                        width: cur_width,
                        gap: gap_next.max(1),
                        sentence_end: false,
                        adjustable: true,
                    });
                    gap_next = $gap;
                } else {
                    // 空ワード（\fI 単独等）の場合は間隔を累積する
                    cur.clear();
                    gap_next += $gap;
                }
                cur_width = 0;
                has_content = false;
            }};
        }

        while i < chars.len() {
            let c = chars[i];
            match c {
                ' ' => {
                    let mut n = 0;
                    while i < chars.len() && chars[i] == ' ' {
                        n += 1;
                        i += 1;
                    }
                    if self.fill {
                        end_word!(n);
                    } else {
                        // no-fill: 空白をそのまま保持
                        open_style!();
                        for _ in 0..n {
                            cur.push(' ');
                            cur_width += 1;
                        }
                        has_content = true;
                    }
                    continue;
                }
                '\t' => {
                    i += 1;
                    if self.fill {
                        end_word!(1);
                    } else {
                        // no-fill: タブストップまで空白
                        let pos = self.cur_width
                            + words.iter().map(|w| w.width + w.gap).sum::<usize>()
                            + cur_width;
                        let stop = self
                            .tab_stops
                            .iter()
                            .find(|&&s| s > pos)
                            .copied()
                            .unwrap_or(pos + 1);
                        for _ in pos..stop {
                            open_style!();
                            cur.push(' ');
                            cur_width += 1;
                        }
                        has_content = true;
                    }
                    continue;
                }
                '\\' => {
                    i += 1;
                    if i >= chars.len() {
                        break;
                    }
                    let e = chars[i];
                    i += 1;
                    match e {
                        'f' => {
                            let name = read_escape_name(&chars, &mut i);
                            let newf = if name == "P" {
                                self.prev_font
                            } else {
                                Self::map_font_name(&name)
                            };
                            if newf != self.font {
                                if style_open {
                                    cur.push_str(self.style_off(self.font));
                                    style_open = false;
                                }
                                self.prev_font = self.font;
                                self.font = newf;
                            }
                        }
                        '(' => {
                            if i + 1 < chars.len() {
                                let name: String = chars[i..i + 2].iter().collect();
                                i += 2;
                                if let Some(s) = special_char(&name, self.device) {
                                    push_str_w!(&s);
                                }
                            }
                        }
                        '[' => {
                            let mut name = String::new();
                            while i < chars.len() && chars[i] != ']' {
                                name.push(chars[i]);
                                i += 1;
                            }
                            if i < chars.len() { i += 1; }
                            if let Some(s) = special_char(&name, self.device) {
                                push_str_w!(&s);
                            }
                        }
                        'C' => {
                            // \C'name'
                            if i < chars.len() {
                                let delim = chars[i];
                                i += 1;
                                let mut name = String::new();
                                while i < chars.len() && chars[i] != delim {
                                    name.push(chars[i]);
                                    i += 1;
                                }
                                if i < chars.len() { i += 1; }
                                if let Some(s) = special_char(&name, self.device) {
                                    push_str_w!(&s);
                                }
                            }
                        }
                        'e' => push_char!('\\'),
                        '\\' => push_char!('\\'),
                        // \- はダッシュ記号（明示ハイフンと違い、ここでは改行しない）。
                        // 内部では U+2011 で保持し、出力時に '-' へ戻す。
                        '-' => push_char!('\u{2011}'),
                        '.' => push_char!('.'),
                        '\'' => push_str_w!(if self.device == Device::Utf8 { "\u{00B4}" } else { "'" }),
                        '`' => push_char!('`'),
                        // \& はゼロ幅の非表示文字。文末判定の打ち消しに使われるので
                        // マーカー文字として保持し、出力時に除去する
                        '&' | ')' => {
                            open_style!();
                            cur.push('\u{E000}');
                            has_content = true;
                        }
                        '~' | ' ' => {
                            // 改行しないスペース（ワード内に保持）
                            open_style!();
                            cur.push(' ');
                            cur_width += 1;
                            has_content = true;
                        }
                        '0' => {
                            open_style!();
                            cur.push(' ');
                            cur_width += 1;
                            has_content = true;
                        }
                        // \% は語の分綴を抑制するマーカー
                        '%' => {
                            open_style!();
                            cur.push('\u{E001}');
                        }
                        // \| \^ は極小スペース（nroff ではゼロ幅）。
                        // \& と同様に文末判定を打ち消すのでマーカーとして残す
                        '|' | '^' => {
                            open_style!();
                            cur.push('\u{E000}');
                        }
                        ':' | 'a' | 'd' | 'u' | 'r' | ',' | '/' => {}
                        'c' => {
                            self.pending_connect = true;
                            break;
                        }
                        't' => push_char!('\t'),
                        's' => consume_size_escape(&chars, &mut i),
                        'h' => {
                            // \h'式' → 水平移動（空白で近似）
                            if let Some(arg) = read_delim_arg(&chars, &mut i) {
                                let expanded = self.interpolate(&arg, None);
                                if let Some(units) = eval_expr(&expanded, UNITS_PER_CHAR) {
                                    let n = (units / UNITS_PER_CHAR).round() as i64;
                                    for _ in 0..n.max(0) {
                                        open_style!();
                                        cur.push(' ');
                                        cur_width += 1;
                                    }
                                    has_content = true;
                                }
                            }
                        }
                        'v' | 'x' | 'l' | 'L' | 'D' | 'b' | 'o' | 'X' | 'Z' => {
                            let _ = read_delim_arg(&chars, &mut i);
                        }
                        'k' => {
                            let _ = read_escape_name(&chars, &mut i);
                        }
                        'z' => {
                            if i < chars.len() {
                                open_style!();
                                cur.push(chars[i]);
                                has_content = true;
                                i += 1;
                            }
                        }
                        'p' => {
                            end_word!(1);
                            // 段落内改行（spread）
                            let taken = std::mem::take(&mut words);
                            self.emit_words(taken);
                            self.flush(true);
                        }
                        'm' | 'M' => {
                            // カラー指定: 引数を読み飛ばす
                            if i < chars.len() && (chars[i] == '[' || chars[i] == '(') {
                                let _ = read_escape_name(&chars, &mut i);
                            } else if i < chars.len() {
                                i += 1;
                            }
                        }
                        '"' | '#' => break,
                        other => push_char!(other),
                    }
                }
                _ => {
                    push_char!(c);
                    i += 1;
                }
            }
        }
        end_word!(0);

        // 文末判定（行末のワードが . ! ? で終わる場合、次との間隔を 2 に）
        if !self.pending_connect {
            if let Some(last) = words.last_mut() {
                if is_sentence_end(&last.text) {
                    last.sentence_end = true;
                }
            }
        }
        words
    }

    /// エスケープを処理した装飾なしのテキストを返す（ヘッダ/フッタ用）
    fn render_plain(&mut self, text: &str) -> String {
        let saved_styled = self.styled;
        let saved_font = self.font;
        self.styled = false;
        let words = self.render_words(text);
        self.styled = saved_styled;
        self.font = saved_font;
        let mut s = String::new();
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                s.push_str(&" ".repeat(w.gap.max(1)));
            }
            s.push_str(&w.text);
        }
        s
    }

    /// テキストの表示幅を計測する（\w 用）
    fn measure_text(&mut self, text: &str) -> usize {
        let saved_font = self.font;
        let saved_prev = self.prev_font;
        let saved_connect = self.pending_connect;
        let saved_tag = self.tag_pending.take();
        let words = self.render_words(text);
        self.font = saved_font;
        self.prev_font = saved_prev;
        self.pending_connect = saved_connect;
        self.tag_pending = saved_tag;
        let mut w = 0;
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                w += word.gap.max(1);
            }
            w += word.width;
        }
        w
    }

    // ------------------------------------------------------------------
    // 補間（文字列・レジスタ・マクロ引数・\w・コメント）
    // ------------------------------------------------------------------

    fn interpolate(&mut self, line: &str, margs: Option<&[String]>) -> String {
        // マクロ内では \\ を 1 段階解決してから処理する
        let work: Vec<char> = if margs.is_some() {
            let mut v = Vec::new();
            let cs: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < cs.len() {
                if cs[i] == '\\' && i + 1 < cs.len() && cs[i + 1] == '\\' {
                    v.push('\\');
                    i += 2;
                } else {
                    v.push(cs[i]);
                    i += 1;
                }
            }
            v
        } else {
            line.chars().collect()
        };

        let mut out = String::new();
        let mut i = 0;
        while i < work.len() {
            let c = work[i];
            if c != '\\' {
                out.push(c);
                i += 1;
                continue;
            }
            if i + 1 >= work.len() {
                out.push('\\');
                break;
            }
            let e = work[i + 1];
            match e {
                '$' => {
                    i += 2;
                    if i < work.len() {
                        let sel = work[i];
                        i += 1;
                        if let Some(args) = margs {
                            match sel {
                                '1'..='9' => {
                                    let idx = sel.to_digit(10).unwrap() as usize;
                                    if let Some(a) = args.get(idx - 1) {
                                        out.push_str(a);
                                    }
                                }
                                '0' => out.push_str("macro"),
                                '*' => out.push_str(&args.join(" ")),
                                '@' => {
                                    let quoted: Vec<String> =
                                        args.iter().map(|a| format!("\"{}\"", a)).collect();
                                    out.push_str(&quoted.join(" "));
                                }
                                '(' => {
                                    // \$(nn 2桁
                                    let mut num = String::new();
                                    if i < work.len() { num.push(work[i]); i += 1; }
                                    if i < work.len() { num.push(work[i]); i += 1; }
                                    if let Ok(n) = num.parse::<usize>() {
                                        if let Some(a) = args.get(n - 1) {
                                            out.push_str(a);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                '*' => {
                    i += 2;
                    let name = read_ref_name(&work, &mut i);
                    if let Some(val) = self.strings.get(&name).cloned() {
                        let expanded = self.interpolate(&val, None);
                        out.push_str(&expanded);
                    }
                }
                'n' => {
                    i += 2;
                    let mut sign = 0i64;
                    if i < work.len() && (work[i] == '+' || work[i] == '-') {
                        sign = if work[i] == '+' { 1 } else { -1 };
                        i += 1;
                    }
                    let name = read_ref_name(&work, &mut i);
                    let val = self.read_register(&name, sign);
                    out.push_str(&val.to_string());
                }
                'w' => {
                    i += 2;
                    if i < work.len() {
                        let delim = work[i];
                        i += 1;
                        let mut inner = String::new();
                        while i < work.len() && work[i] != delim {
                            inner.push(work[i]);
                            i += 1;
                        }
                        if i < work.len() { i += 1; }
                        let expanded = self.interpolate(&inner, margs);
                        let w = self.measure_text(&expanded);
                        out.push_str(&((w as f64 * UNITS_PER_CHAR) as i64).to_string());
                    }
                }
                '"' => break, // コメント
                '#' => break,
                _ => {
                    out.push('\\');
                    out.push(e);
                    i += 2;
                }
            }
        }
        out
    }

    fn read_register(&mut self, name: &str, sign: i64) -> i64 {
        // 組み込みレジスタ
        match name {
            ".l" => return (self.ll as f64 * UNITS_PER_CHAR) as i64,
            ".i" => return (self.indent as f64 * UNITS_PER_CHAR) as i64,
            ".v" => return UNITS_PER_LINE as i64,
            ".H" => return UNITS_PER_CHAR as i64,
            ".V" => return UNITS_PER_LINE as i64,
            ".u" => return if self.fill { 1 } else { 0 },
            ".$" => return 0, // マクロ外では 0（マクロ内は interpolate 前に展開済み想定）
            _ => {}
        }
        if sign != 0 {
            let step = self.reg_steps.get(name).copied().unwrap_or(1);
            let v = self.registers.get(name).copied().unwrap_or(0) + sign * step;
            self.registers.insert(name.to_string(), v);
            v
        } else {
            self.registers.get(name).copied().unwrap_or(0)
        }
    }

    // ------------------------------------------------------------------
    // 実行
    // ------------------------------------------------------------------

    fn run_lines(&mut self, lines: Vec<String>, margs: &mut Vec<String>, in_macro: bool, depth: usize) -> Flow {
        if depth > 64 {
            return Flow::Normal;
        }
        let mut stream = LineStream::new(lines);
        while let Some(raw) = stream.next_line() {
            if self.exited {
                return Flow::Normal;
            }
            let flow = self.process_one(&raw, &mut stream, margs, in_macro, depth);
            if flow == Flow::Return {
                return Flow::Return;
            }
        }
        Flow::Normal
    }

    fn process_one(
        &mut self,
        raw: &str,
        stream: &mut LineStream,
        margs: &mut Vec<String>,
        in_macro: bool,
        depth: usize,
    ) -> Flow {
        // 引数個数レジスタ .$ をマクロ内で使えるよう先に展開
        let raw = if in_macro {
            raw.replace("\\n(.$", &margs.len().to_string())
                .replace("\\n[.$]", &margs.len().to_string())
        } else {
            raw.to_string()
        };

        let line = self.interpolate(&raw, if in_macro { Some(margs) } else { None });

        if self.diverting {
            if line.starts_with(".di") || line.starts_with(".da") {
                self.diverting = false;
            }
            return Flow::Normal;
        }

        if let Some(rest) = line.strip_prefix('.').or_else(|| line.strip_prefix('\'')) {
            return self.control(rest, stream, margs, in_macro, depth);
        }

        self.text_line(&line);
        Flow::Normal
    }

    fn text_line(&mut self, line: &str) {
        let line = line.trim_end();

        if line.is_empty() {
            // 空行は .sp 1 相当（no-space モード中は抑制される）
            self.vspace(1);
            return;
        }

        // 見出し（.SH/.SS の次行形式）
        if let Some(col) = self.pending_heading.take() {
            self.emit_heading(line, col);
            self.no_space = true;
            return;
        }

        // 行頭スペースはブレークして字下げ（fill モードのみ。
        // no-fill ではタブや空白をそのまま保持する）
        let mut content = line;
        if self.fill {
            let leading = line.len() - line.trim_start_matches(' ').len();
            if leading > 0 {
                self.flush(false);
                self.temp_indent = Some(self.indent + leading);
                content = &line[leading..];
            }
        }

        // .B / .I 等の「次行に適用」フォント
        let forced = self.next_line_font.take();
        let saved = self.font;
        if let Some(f) = forced {
            self.font = f;
        }
        if self.ul_count > 0 {
            self.font = 'I';
            self.ul_count -= 1;
        }

        let words = self.render_words(content);

        if forced.is_some() {
            self.font = saved;
            self.font = 'R';
        }

        // \c を見たらこの行の後で連結（render が立てたフラグを引き継ぐ）
        let connect_after = std::mem::take(&mut self.pending_connect);

        if self.ce_count > 0 && self.tag_pending.is_none() {
            self.flush(false);
            self.line = words;
            self.center_flush();
            self.ce_count -= 1;
            return;
        }

        self.emit_words(words);

        if connect_after {
            self.connect_active = true;
        }

        if !self.fill {
            self.flush(false);
        }

        // .TP のタグ行が揃った
        if self.tag_pending.is_some() && !self.tag_words.is_empty() {
            self.finalize_tag();
        }
    }

    fn emit_heading(&mut self, text: &str, col: usize) {
        self.flush(false);
        let saved_font = self.font;
        self.font = 'B';
        let words = self.render_words(text);
        self.font = saved_font;
        let mut s = String::new();
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                s.push_str(&" ".repeat(w.gap.max(1)));
            }
            s.push_str(&w.text);
        }
        self.push_output(format!("{}{}", " ".repeat(col), s));
        self.no_space = false;
    }

    /// .TP / .IP のタグを配置する
    fn finalize_tag(&mut self) {
        let ind = self.tag_pending.take().unwrap_or(self.prevail_indent);
        let words = std::mem::take(&mut self.tag_words);
        let base = self.base_indent;
        let body_indent = base + ind;

        let mut tagw = 0;
        let mut text = String::new();
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                let g = w.gap.max(1);
                text.push_str(&" ".repeat(g));
                tagw += g;
            }
            text.push_str(&w.text);
            tagw += w.width;
        }

        self.indent = body_indent;
        if tagw + 1 <= ind {
            // タグがインデント幅に収まる → 本文は同じ行から
            // （本文先頭ワードの gap 1 と合わせてちょうど ind になるよう詰める）
            let pad = ind - tagw - 1;
            self.temp_indent = Some(base);
            self.line = vec![Word {
                text: format!("{}{}", text, " ".repeat(pad)),
                width: ind - 1,
                gap: 0,
                sentence_end: false,
                adjustable: false,
            }];
            self.cur_width = ind - 1;
        } else {
            // 収まらない → タグを独立した行として出力
            // （長ければ base 幅で折り返す。GNU 同様、両端揃えはしない）
            let saved_indent = self.indent;
            let saved_adjust = self.adjust;
            self.indent = base;
            self.temp_indent = None;
            self.adjust = Adjust::Left;
            for w in words {
                self.place_word(w);
            }
            self.flush(false);
            self.adjust = saved_adjust;
            self.indent = saved_indent;
            self.no_space = false;
        }
    }

    // ------------------------------------------------------------------
    // 制御行
    // ------------------------------------------------------------------

    fn control(
        &mut self,
        rest: &str,
        stream: &mut LineStream,
        margs: &mut Vec<String>,
        in_macro: bool,
        depth: usize,
    ) -> Flow {
        let rest = rest.trim_start();
        if rest.is_empty() || rest.starts_with('\\') && rest[1..].starts_with('"') {
            return Flow::Normal;
        }
        // \} の除去（ブロック終端の残骸）
        if rest == "\\}" {
            return Flow::Normal;
        }

        // リクエスト名は空白または \（ブロック開始 .el\{ 等）まで
        let (name, tail) = match rest.find(|c: char| c == ' ' || c == '\t' || c == '\\') {
            Some(p) if rest[p..].starts_with('\\') && rest[p..].starts_with("\\{") => {
                (&rest[..p], &rest[p..])
            }
            Some(p) if !rest[p..].starts_with('\\') => (&rest[..p], rest[p..].trim_start()),
            Some(p) => (&rest[..p], rest[p..].trim_start()),
            None => (rest, ""),
        };
        let name = name.trim_end_matches("\\}");

        // ユーザ定義マクロ
        if let Some(body) = self.macros.get(name).cloned() {
            let mut args = split_args(tail);
            self.run_lines(body, &mut args, true, depth + 1);
            return Flow::Normal;
        }

        // man マクロ
        if is_man_macro(name) {
            self.man_macro(name, tail, stream, margs, in_macro, depth);
            return Flow::Normal;
        }

        self.request(name, tail, stream, margs, in_macro, depth)
    }

    fn request(
        &mut self,
        name: &str,
        tail: &str,
        stream: &mut LineStream,
        margs: &mut Vec<String>,
        in_macro: bool,
        depth: usize,
    ) -> Flow {
        let args = split_args(tail);
        match name {
            "br" => self.flush(false),
            "sp" => {
                // groff の垂直丸めは「半分ちょうどは切り捨て」（.sp .5 → 0 行）
                let n = if tail.is_empty() {
                    1
                } else {
                    eval_expr(tail, UNITS_PER_LINE)
                        .map(|u| ((u / UNITS_PER_LINE) - 0.5).ceil() as i64)
                        .unwrap_or(1)
                        .max(0) as usize
                };
                self.vspace(n);
            }
            "ll" => self.set_length_reg(tail, |t, v| t.ll = v, |t| t.ll, 78),
            "in" => {
                self.flush(false);
                let cur = self.indent;
                self.set_length_reg(tail, |t, v| t.indent = v, |t| t.indent, 0);
                if tail.is_empty() {
                    self.indent = cur; // .in 単独は直前値へ（簡易: 維持）
                }
            }
            "ti" => {
                self.flush(false);
                if let Some(u) = eval_expr(tail, UNITS_PER_CHAR) {
                    let v = (u / UNITS_PER_CHAR).round().max(0.0) as usize;
                    self.temp_indent = Some(v);
                }
            }
            "fi" => { self.flush(false); self.fill = true; }
            "nf" => { self.flush(false); self.fill = false; }
            "ad" => {
                self.adjust = match args.first().map(String::as_str) {
                    Some("l") => Adjust::Left,
                    Some("r") => Adjust::Right,
                    Some("c") => Adjust::Center,
                    _ => Adjust::Both,
                };
            }
            "na" => self.adjust = Adjust::Left,
            "ce" => {
                self.flush(false);
                self.ce_count = args
                    .first()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
            }
            "ul" | "cu" => {
                self.ul_count = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "ft" => {
                let f = args.first().map(String::as_str).unwrap_or("P");
                let newf = if f == "P" { self.prev_font } else { Self::map_font_name(f) };
                self.prev_font = self.font;
                self.font = newf;
            }
            "ds" | "as" => {
                if let Some(p) = tail.find(|c: char| c == ' ' || c == '\t') {
                    let key = tail[..p].to_string();
                    let mut val = tail[p + 1..].to_string();
                    if val.starts_with('"') {
                        val = val[1..].to_string();
                    }
                    if name == "as" {
                        let old = self.strings.get(&key).cloned().unwrap_or_default();
                        self.strings.insert(key, format!("{}{}", old, val));
                    } else {
                        self.strings.insert(key, val);
                    }
                } else if !tail.is_empty() {
                    self.strings.insert(tail.to_string(), String::new());
                }
            }
            "nr" => {
                if args.len() >= 2 {
                    let key = args[0].clone();
                    let vs = &args[1];
                    let (rel, expr) = if let Some(r) = vs.strip_prefix('+') {
                        (1i64, r)
                    } else if let Some(r) = vs.strip_prefix('-') {
                        (-1i64, r)
                    } else {
                        (0, vs.as_str())
                    };
                    if let Some(u) = eval_expr(expr, 1.0) {
                        let v = u as i64;
                        let new = if rel != 0 {
                            self.registers.get(&key).copied().unwrap_or(0) + rel * v
                        } else {
                            v
                        };
                        self.registers.insert(key.clone(), new);
                    }
                    if let Some(step) = args.get(2).and_then(|s| s.parse().ok()) {
                        self.reg_steps.insert(args[0].clone(), step);
                    }
                }
            }
            "rr" => {
                for a in &args {
                    self.registers.remove(a);
                }
            }
            "rm" => {
                for a in &args {
                    self.macros.remove(a);
                    self.strings.remove(a);
                }
            }
            "rn" | "als" => {
                if args.len() >= 2 {
                    if let Some(m) = self.macros.get(&args[0]).cloned() {
                        self.macros.insert(args[1].clone(), m);
                    } else if let Some(s) = self.strings.get(&args[0]).cloned() {
                        self.strings.insert(args[1].clone(), s);
                    }
                    if name == "rn" {
                        self.macros.remove(&args[0]);
                        self.strings.remove(&args[0]);
                    }
                }
            }
            "de" | "de1" | "am" | "am1" => {
                let mname = args.first().cloned().unwrap_or_default();
                let end = args.get(1).cloned().unwrap_or_else(|| ".".repeat(2));
                let end_dot = format!(".{}", end);
                let mut body = Vec::new();
                while let Some(l) = stream.next_line() {
                    let t = l.trim_end();
                    if t == ".." || t == end_dot || t.trim_start() == end_dot {
                        break;
                    }
                    body.push(l);
                }
                if mname.is_empty() {
                    return Flow::Normal;
                }
                if name.starts_with("am") {
                    self.macros.entry(mname).or_default().extend(body);
                } else {
                    self.macros.insert(mname, body);
                }
            }
            "ig" => {
                let end = args.first().cloned().unwrap_or_default();
                let end_dot = if end.is_empty() { "..".to_string() } else { format!(".{}", end) };
                while let Some(l) = stream.next_line() {
                    let t = l.trim_end();
                    if t == ".." || t == end_dot {
                        break;
                    }
                }
            }
            "if" | "ie" => {
                let (cond, body) = self.parse_condition(tail);
                if name == "ie" {
                    self.el_stack.push(!cond);
                }
                self.exec_cond_body(cond, body, stream, margs, in_macro, depth);
            }
            "el" => {
                let cond = self.el_stack.pop().unwrap_or(false);
                self.exec_cond_body(cond, tail.to_string(), stream, margs, in_macro, depth);
            }
            "while" => {
                // 未対応: ブロックを読み飛ばす
                if tail.contains("\\{") {
                    let start = tail.find("\\{").map(|p| &tail[p + 2..]).unwrap_or("");
                    let _ = collect_block(start, stream);
                }
            }
            "so" => {
                if !tail.is_empty() {
                    self.source_file(tail, depth);
                }
            }
            "mso" => {} // マクロパッケージは組み込み
            "shift" => {
                let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
                for _ in 0..n.min(margs.len()) {
                    margs.remove(0);
                }
            }
            "return" => return Flow::Return,
            "nop" => {
                let l = tail.to_string();
                return self.process_one(&l, stream, margs, in_macro, depth);
            }
            "do" => {
                // groff 拡張モードでリクエストを実行（互換モード無効なのでそのまま実行）
                let l = format!(".{}", tail);
                return self.exec_interpolated_line(&l, stream, margs, in_macro, depth);
            }
            "ex" => self.exited = true,
            "ab" => {
                if !tail.is_empty() {
                    eprintln!("{}", tail);
                }
                self.exited = true;
            }
            "tm" => eprintln!("{}", tail),
            "ns" => self.no_space = true,
            "rs" => self.no_space = false,
            "ta" => {
                let mut stops = Vec::new();
                let mut last = 0usize;
                for a in &args {
                    let (rel, expr) = if let Some(r) = a.strip_prefix('+') {
                        (true, r)
                    } else {
                        (false, a.as_str())
                    };
                    // 末尾の T/L/R/C 揃え指定は無視
                    let expr = expr.trim_end_matches(['R', 'L', 'C', 'T']);
                    if let Some(u) = eval_expr(expr, UNITS_PER_CHAR) {
                        let v = (u / UNITS_PER_CHAR).round() as usize;
                        let stop = if rel { last + v } else { v };
                        stops.push(stop);
                        last = stop;
                    }
                }
                if !stops.is_empty() {
                    // 最後の間隔で繰り返し
                    let step = if stops.len() >= 2 {
                        stops[stops.len() - 1] - stops[stops.len() - 2]
                    } else {
                        stops[0].max(1)
                    };
                    let mut cur = last;
                    while cur < 400 {
                        cur += step.max(1);
                        stops.push(cur);
                    }
                    self.tab_stops = stops;
                }
            }
            "ls" => {
                self.line_spacing = args.first().and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
            }
            "di" | "da" => {
                if !args.is_empty() {
                    self.diverting = true;
                }
            }
            "hy" => {
                self.hy = args
                    .first()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|n| n != 0)
                    .unwrap_or(true);
            }
            "nh" => self.hy = false,
            "ss" => {
                // .ss ワード間 [文末追加分]（単位 1/12 スペース）。第2引数 0 で文末の
                // 追加スペースを無効化（util-linux 系ページが使用）
                if let Some(m) = args.get(1).and_then(|s| s.parse::<i64>().ok()) {
                    self.sentence_gap_extra = if m <= 0 { 0 } else { 1 };
                }
            }
            "bp" | "ne" | "pl" | "po" | "pn" | "wh" | "ch" | "it" | "vs" | "ps"
            | "cs" | "bd" | "hw" | "hc" | "hla" | "hlm" | "hym" | "hys" | "kern" | "fam"
            | "ec" | "eo" | "cc" | "c2" | "lf" | "cf" | "fl" | "os" | "mk" | "rt" | "sv"
            | "cp" | "tr" | "char" | "fchar" | "fp" | "ftr" | "nx" | "pi" | "af"
            | "aln" | "cflags" | "shc" | "sy" | "open" | "opena" | "close" | "write"
            | "writem" | "ev" | "evc" | "gcolor" | "fcolor" | "defcolor" | "nm" | "nn"
            | "pm" | "pev" | "spreadwarn" | "warn" | "blm" | "em" | "itc" | "lsm"
            | "linetabs" | "vpt" | "warnscale" | "sizes" | "stringdown" | "stringup"
            | "substring" | "length" | "index" | "chop" | "pso" | "PSPIC" => {}
            _ => {
                // 未知のリクエストは無視（groff も未定義名は無視する）
            }
        }
        Flow::Normal
    }

    fn set_length_reg(
        &mut self,
        tail: &str,
        set: impl Fn(&mut Self, usize),
        get: impl Fn(&Self) -> usize,
        default: usize,
    ) {
        if tail.is_empty() {
            set(self, default);
            return;
        }
        let (rel, expr) = if let Some(r) = tail.strip_prefix('+') {
            (1i64, r)
        } else if let Some(r) = tail.strip_prefix('-') {
            (-1i64, r)
        } else {
            (0, tail)
        };
        if let Some(u) = eval_expr(expr, UNITS_PER_CHAR) {
            let v = (u / UNITS_PER_CHAR).round() as i64;
            let cur = get(self) as i64;
            let newv = if rel != 0 { cur + rel * v } else { v };
            set(self, newv.max(0) as usize);
        }
    }

    // ------------------------------------------------------------------
    // 条件
    // ------------------------------------------------------------------

    /// 条件部と本体を分離して評価する
    fn parse_condition(&mut self, tail: &str) -> (bool, String) {
        let tail = tail.trim_start();
        let chars: Vec<char> = tail.chars().collect();
        let mut i = 0;
        let mut neg = false;
        while i < chars.len() && chars[i] == '!' {
            neg = !neg;
            i += 1;
        }
        if i >= chars.len() {
            return (false, String::new());
        }

        let (mut cond, rest_idx) = match chars[i] {
            'n' if next_is_space(&chars, i + 1) => (true, i + 1),
            't' | 'v' | 'e' if next_is_space(&chars, i + 1) => (false, i + 1),
            'o' if next_is_space(&chars, i + 1) => (true, i + 1),
            'c' if next_is_space_or(&chars, i + 1, ' ') => {
                // c 文字 → その文字が存在するか（常に真とみなす）
                let mut j = i + 1;
                while j < chars.len() && chars[j] == ' ' { j += 1; }
                // 文字（またはエスケープ）を読み飛ばす
                if j < chars.len() && chars[j] == '\\' {
                    j += 1;
                    if j < chars.len() && (chars[j] == '(') { j += 3; } else { j += 1; }
                } else if j < chars.len() {
                    j += 1;
                }
                (true, j)
            }
            'd' | 'r' | 'm' | 'F' | 'S' => {
                let kind = chars[i];
                let mut j = i + 1;
                while j < chars.len() && chars[j] == ' ' { j += 1; }
                let mut name = String::new();
                while j < chars.len() && chars[j] != ' ' && chars[j] != '\t' {
                    name.push(chars[j]);
                    j += 1;
                }
                let c = match kind {
                    'd' => self.strings.contains_key(&name) || self.macros.contains_key(&name),
                    'r' => self.registers.contains_key(&name),
                    _ => false,
                };
                (c, j)
            }
            '\'' | '"' | '`' | '|' | '@' | '^' => {
                // 文字列比較 'a'b'
                let delim = chars[i];
                let mut j = i + 1;
                let mut a = String::new();
                while j < chars.len() && chars[j] != delim {
                    a.push(chars[j]);
                    j += 1;
                }
                j += 1;
                let mut b = String::new();
                while j < chars.len() && chars[j] != delim {
                    b.push(chars[j]);
                    j += 1;
                }
                if j < chars.len() { j += 1; }
                (a == b, j)
            }
            _ => {
                // 数値式
                let expr_str: String = chars[i..].iter().collect();
                let cvec: Vec<char> = expr_str.chars().collect();
                let mut p = 0;
                let v = parse_expr(&cvec, &mut p, 1.0).unwrap_or(0.0);
                (v > 0.0, i + p)
            }
        };
        if neg {
            cond = !cond;
        }
        let body: String = chars[rest_idx.min(chars.len())..].iter().collect();
        (cond, body.trim_start().to_string())
    }

    fn exec_cond_body(
        &mut self,
        cond: bool,
        body: String,
        stream: &mut LineStream,
        margs: &mut Vec<String>,
        in_macro: bool,
        depth: usize,
    ) {
        if let Some(after) = body.strip_prefix("\\{") {
            // 複数行ブロック。最初のチャンク（条件行の残り）は補間済み、
            // ストリームから集めた行は未補間なので実行時に補間する。
            let (block, from_chunk) = collect_block(after, stream);
            if cond {
                let mut stream2 = LineStream::new(block);
                let mut idx = 0;
                while let Some(l) = stream2.next_line() {
                    if self.exited {
                        break;
                    }
                    let flow = if idx < from_chunk {
                        self.exec_interpolated_line(&l, &mut stream2, margs, in_macro, depth)
                    } else {
                        self.process_one(&l, &mut stream2, margs, in_macro, depth)
                    };
                    idx += 1;
                    if flow == Flow::Return {
                        break;
                    }
                }
            }
        } else if cond && !body.is_empty() {
            let mut dummy = LineStream::new(Vec::new());
            let _ = self.exec_interpolated_line(&body, &mut dummy, margs, in_macro, depth);
        }
    }

    /// すでに補間済みの行を処理する（条件本体・ブロック用）
    fn exec_interpolated_line(
        &mut self,
        line: &str,
        stream: &mut LineStream,
        margs: &mut Vec<String>,
        in_macro: bool,
        depth: usize,
    ) -> Flow {
        if self.diverting {
            if line.starts_with(".di") || line.starts_with(".da") {
                self.diverting = false;
            }
            return Flow::Normal;
        }
        if let Some(rest) = line.strip_prefix('.').or_else(|| line.strip_prefix('\'')) {
            return self.control(rest, stream, margs, in_macro, depth);
        }
        self.text_line(line);
        Flow::Normal
    }

    // ------------------------------------------------------------------
    // .so
    // ------------------------------------------------------------------

    fn source_file(&mut self, path: &str, depth: usize) {
        let candidates: Vec<PathBuf> = {
            let mut v = vec![PathBuf::from(path)];
            if let Some(dir) = self.file_dirs.last() {
                v.push(dir.join(path));
                if let Some(parent) = dir.parent() {
                    v.push(parent.join(path));
                }
            }
            v
        };
        for cand in candidates {
            if cand.is_file() {
                if let Ok(content) = fs::read_to_string(&cand) {
                    self.file_dirs
                        .push(cand.parent().map(Path::to_path_buf).unwrap_or_default());
                    let lines: Vec<String> = content.lines().map(str::to_string).collect();
                    let mut noargs = Vec::new();
                    self.run_lines(lines, &mut noargs, false, depth + 1);
                    self.file_dirs.pop();
                    return;
                }
            }
        }
        eprintln!("groff: .so '{}' を読み込めません", path);
    }

    // ------------------------------------------------------------------
    // man マクロ
    // ------------------------------------------------------------------

    fn man_macro(
        &mut self,
        name: &str,
        tail: &str,
        _stream: &mut LineStream,
        _margs: &mut Vec<String>,
        _in_macro: bool,
        _depth: usize,
    ) {
        let args = split_args(tail);
        match name {
            "TH" => {
                self.flush(false);
                let title = args.first().cloned().unwrap_or_default();
                let sec = args.get(1).cloned().unwrap_or_default();
                let date = args.get(2).cloned().unwrap_or_default();
                let source = args.get(3).cloned().unwrap_or_default();
                // デフォルトのマニュアル名は第5引数が「無い」ときだけ
                // （明示的な空文字列 "" は空のまま）
                let manual = args
                    .get(4)
                    .cloned()
                    .unwrap_or_else(|| default_section_name(&sec).to_string());
                // LL レジスタ反映
                if let Some(&llu) = self.registers.get("LL") {
                    let chars = (llu as f64 / UNITS_PER_CHAR).round() as usize;
                    if chars > 0 {
                        self.ll = chars;
                    }
                }
                let title = self.render_plain(&title);
                let sec = self.render_plain(&sec);
                let date = self.render_plain(&date);
                let source = self.render_plain(&source);
                let manual = self.render_plain(&manual);
                let t = format!("{}({})", title, sec);
                let header = three_part(&t, &manual, &t, self.ll);
                self.push_output(header);
                self.th = Some((title, sec, date, source, manual));
                self.base_indent = 7;
                self.indent = 7;
                // ヘッダ直後の空行はここで出し、後続の段落間隔は抑制する
                self.out.push(String::new());
                self.no_space = true;
                self.after_heading = true;
            }
            "SH" => {
                self.rs_stack.clear();
                self.base_indent = 7;
                self.prevail_indent = 7;
                self.para_space();
                self.fill = true;
                self.font = 'R';
                if args.is_empty() {
                    self.pending_heading = Some(0);
                } else {
                    let text = args.join(" ");
                    self.emit_heading(&text, 0);
                }
                self.indent = 7;
                self.temp_indent = None;
                // 見出し直後の段落マクロは空行を追加しない（an.tmac の .ns 相当）
                self.no_space = true;
                self.after_heading = true;
            }
            "SS" => {
                self.rs_stack.clear();
                self.base_indent = 7;
                self.prevail_indent = 7;
                self.para_space();
                self.fill = true;
                self.font = 'R';
                if args.is_empty() {
                    self.pending_heading = Some(3);
                } else {
                    let text = args.join(" ");
                    self.emit_heading(&text, 3);
                }
                self.indent = 7;
                self.temp_indent = None;
                self.no_space = true;
                self.after_heading = true;
            }
            "PP" | "P" | "LP" => {
                self.para_space();
                self.prevail_indent = 7;
                self.indent = self.base_indent;
                self.temp_indent = None;
                self.fill = true;
            }
            "PD" => {
                self.pd = if args.is_empty() {
                    1
                } else {
                    eval_expr(&args[0], UNITS_PER_LINE)
                        .map(|u| (u / UNITS_PER_LINE).round() as i64)
                        .unwrap_or(1)
                        .max(0) as usize
                };
            }
            "IP" => {
                self.para_space();
                // GNU an.tmac の挙動: .IP/.TP は調整モードを両端揃えに戻す
                self.adjust = Adjust::Both;
                let ind = args
                    .get(1)
                    .and_then(|s| eval_expr(s, UNITS_PER_CHAR))
                    .map(|u| (u / UNITS_PER_CHAR).round().max(0.0) as usize);
                if let Some(v) = ind {
                    self.prevail_indent = v;
                }
                let ind = self.prevail_indent;
                let tag = args.first().cloned().unwrap_or_default();
                if tag.is_empty() {
                    self.indent = self.base_indent + ind;
                    self.temp_indent = None;
                } else {
                    self.tag_pending = Some(ind);
                    self.tag_words.clear();
                    // タグ先頭のスペースは保持する（.IP " 1." 等）
                    let lead = tag.len() - tag.trim_start_matches(' ').len();
                    let mut words = self.render_words(tag.trim_start_matches(' '));
                    if lead > 0 {
                        if let Some(first) = words.first_mut() {
                            first.text = format!("{}{}", " ".repeat(lead), first.text);
                            first.width += lead;
                        }
                    }
                    self.font = 'R';
                    self.tag_words = words;
                    self.finalize_tag();
                }
            }
            "TP" | "TQ" => {
                self.adjust = Adjust::Both;
                if name == "TP" {
                    self.para_space();
                } else {
                    self.flush(false);
                }
                if let Some(v) = args
                    .first()
                    .and_then(|s| eval_expr(s, UNITS_PER_CHAR))
                    .map(|u| (u / UNITS_PER_CHAR).round().max(0.0) as usize)
                {
                    self.prevail_indent = v;
                }
                self.tag_pending = Some(self.prevail_indent);
                self.tag_words.clear();
            }
            "HP" => {
                // GNU groff の挙動: 見出し直後の .HP は空行を 2 行入れる
                if self.no_space && self.after_heading {
                    self.flush(false);
                    self.no_space = false;
                    self.after_heading = false;
                    self.out.push(String::new());
                    self.out.push(String::new());
                } else {
                    self.para_space();
                }
                if let Some(v) = args
                    .first()
                    .and_then(|s| eval_expr(s, UNITS_PER_CHAR))
                    .map(|u| (u / UNITS_PER_CHAR).round().max(0.0) as usize)
                {
                    self.prevail_indent = v;
                }
                self.temp_indent = Some(self.base_indent);
                self.indent = self.base_indent + self.prevail_indent;
            }
            "RS" => {
                self.flush(false);
                let ind = args
                    .first()
                    .and_then(|s| eval_expr(s, UNITS_PER_CHAR))
                    .map(|u| (u / UNITS_PER_CHAR).round().max(0.0) as usize)
                    .unwrap_or(self.prevail_indent);
                self.rs_stack.push(ind);
                self.base_indent += ind;
                self.indent = self.base_indent;
            }
            "RE" => {
                self.flush(false);
                if let Some(level) = args.first().and_then(|s| s.parse::<usize>().ok()) {
                    while self.rs_stack.len() + 1 > level.max(1) {
                        if let Some(ind) = self.rs_stack.pop() {
                            self.base_indent = self.base_indent.saturating_sub(ind);
                        } else {
                            break;
                        }
                    }
                } else if let Some(ind) = self.rs_stack.pop() {
                    self.base_indent = self.base_indent.saturating_sub(ind);
                }
                if self.base_indent < 7 {
                    self.base_indent = 7;
                }
                self.indent = self.base_indent;
            }
            "B" | "I" | "SM" | "SB" => {
                let font = match name {
                    "B" | "SB" => 'B',
                    "I" => 'I',
                    _ => 'R',
                };
                if args.is_empty() {
                    self.next_line_font = Some(font);
                } else {
                    let text = args.join(" ");
                    let saved = self.font;
                    self.font = font;
                    let words = self.render_words(&text);
                    self.font = saved;
                    self.font = 'R';
                    self.emit_or_tag(words);
                }
            }
            "BR" | "RB" | "IR" | "RI" | "BI" | "IB" => {
                let fonts: [char; 2] = match name {
                    "BR" => ['B', 'R'],
                    "RB" => ['R', 'B'],
                    "IR" => ['I', 'R'],
                    "RI" => ['R', 'I'],
                    "BI" => ['B', 'I'],
                    "IB" => ['I', 'B'],
                    _ => ['R', 'R'],
                };
                if args.is_empty() {
                    return;
                }
                // 引数を交互フォントで連結する。引数境界は密着、
                // 引用符内の前後スペース（例 ", "）は語の区切りとして保持する。
                let mut result: Vec<Word> = Vec::new();
                let mut attach = false; // 前の語に密着するか
                for (i, a) in args.iter().enumerate() {
                    // 引用符由来の（エスケープされていない）前後スペースのみ区切りとして扱う
                    let lead_space = a.starts_with(' ');
                    let trimmed_start = a.trim_start_matches(' ');
                    let mut plain_trail = 0;
                    {
                        let b: Vec<char> = trimmed_start.chars().collect();
                        let mut e = b.len();
                        while e > 0 && b[e - 1] == ' ' && !(e >= 2 && b[e - 2] == '\\') {
                            e -= 1;
                            plain_trail += 1;
                        }
                    }
                    let trail_space = plain_trail > 0;
                    let trimmed: String = {
                        let cs: Vec<char> = trimmed_start.chars().collect();
                        cs[..cs.len() - plain_trail].iter().collect()
                    };
                    let trimmed = trimmed.as_str();
                    if trimmed.is_empty() {
                        attach = false;
                        continue;
                    }
                    let f = fonts[i % 2];
                    let saved = self.font;
                    self.font = f;
                    let mut ws = self.render_words(trimmed);
                    self.font = saved;
                    // 文末判定は emit_or_tag が最終ワードに対して行う
                    for w in &mut ws {
                        w.sentence_end = false;
                    }
                    for (j, w) in ws.into_iter().enumerate() {
                        if j == 0 && attach && !lead_space && !result.is_empty() {
                            let last = result.last_mut().unwrap();
                            last.text.push_str(&w.text);
                            last.width += w.width;
                        } else {
                            result.push(Word { gap: 1, ..w });
                        }
                    }
                    attach = !trail_space;
                }
                self.font = 'R';
                self.emit_or_tag(result);
            }
            "EX" => {
                self.flush(false);
                self.fill = false;
            }
            "EE" => {
                self.flush(false);
                self.fill = true;
            }
            "UR" | "MT" => {
                self.link_url = args.first().cloned();
            }
            "UE" | "ME" => {
                if let Some(url) = self.link_url.take() {
                    let (lb, rb) = if self.device == Device::Utf8 {
                        ("\u{27E8}", "\u{27E9}")
                    } else {
                        ("<", ">")
                    };
                    let mut text = format!("{}{}{}", lb, url, rb);
                    let mut width = visible_width(&text);
                    // 末尾引数（句読点）は密着
                    if let Some(t) = args.first() {
                        text.push_str(t);
                        width += visible_width(t);
                    }
                    self.emit_or_tag(vec![Word {
                        text,
                        width,
                        gap: 1,
                        sentence_end: false,
                        adjustable: true,
                    }]);
                }
            }
            "SY" => {
                self.para_space();
                let cmd = args.first().cloned().unwrap_or_default();
                let saved = self.font;
                self.font = 'B';
                let words = self.render_words(&cmd);
                self.font = saved;
                self.font = 'R';
                let mut text = String::new();
                let mut width = 0;
                for (j, w) in words.iter().enumerate() {
                    if j > 0 {
                        text.push_str(" ");
                        width += 1;
                    }
                    text.push_str(&w.text);
                    width += w.width;
                }
                let body_indent = self.base_indent + width + 1;
                self.indent = body_indent;
                self.temp_indent = Some(self.base_indent);
                self.line = vec![Word {
                    text,
                    width,
                    gap: 0,
                    sentence_end: false,
                    adjustable: false,
                }];
                self.cur_width = width;
            }
            "YS" => {
                self.flush(false);
                self.indent = self.base_indent;
                self.temp_indent = None;
            }
            "OP" => {
                let mut text = String::new();
                let mut width = 0;
                text.push('[');
                width += 1;
                if let Some(opt) = args.first() {
                    let saved = self.font;
                    self.font = 'B';
                    let ws = self.render_words(opt);
                    self.font = saved;
                    for w in &ws {
                        text.push_str(&w.text);
                        width += w.width;
                    }
                }
                if let Some(arg) = args.get(1) {
                    text.push(' ');
                    width += 1;
                    let saved = self.font;
                    self.font = 'I';
                    let ws = self.render_words(arg);
                    self.font = saved;
                    for w in &ws {
                        text.push_str(&w.text);
                        width += w.width;
                    }
                }
                self.font = 'R';
                text.push(']');
                width += 1;
                self.emit_or_tag(vec![Word {
                    text,
                    width,
                    gap: 1,
                    sentence_end: false,
                    adjustable: true,
                }]);
            }
            "DT" => self.tab_stops = (1..=20).map(|i| i * 8).collect(),
            "AT" | "UC" => {}
            _ => {}
        }
    }

    /// ワードを通常出力または TP タグとして送る。
    /// マクロ行の末尾は入力行末なので文末判定も行い、\c の連結も伝播させる。
    fn emit_or_tag(&mut self, mut words: Vec<Word>) {
        let connect_after = std::mem::take(&mut self.pending_connect);
        if !connect_after {
            if let Some(last) = words.last_mut() {
                if is_sentence_end(&last.text) {
                    last.sentence_end = true;
                }
            }
        }
        if self.tag_pending.is_some() {
            self.tag_words.extend(words);
            if connect_after {
                self.connect_active = true;
            } else {
                self.finalize_tag_if_line_done();
            }
        } else {
            self.emit_words(words);
            if connect_after {
                self.connect_active = true;
            }
        }
    }

    fn finalize_tag_if_line_done(&mut self) {
        // man の .TP タグは 1 入力行なので、テキストを出したマクロの直後に確定する
        if self.tag_pending.is_some() && !self.tag_words.is_empty() {
            self.finalize_tag();
        }
    }

    // ------------------------------------------------------------------
    // 完了処理
    // ------------------------------------------------------------------

    fn finish(&mut self) -> String {
        self.flush(false);
        if let Some((title, sec, date, source, _)) = self.th.clone() {
            // 本文と区切る空行（本文由来の末尾空行は保持する）
            self.out.push(String::new());
            let t = format!("{}({})", title, sec);
            self.out.push(three_part(&source, &date, &t, self.ll));
        } else {
            while self.out.last().map(|l| l.is_empty()).unwrap_or(false) {
                self.out.pop();
            }
        }
        let mut s = self.out.join("\n");
        s.push('\n');
        // 改行しないハイフン（\- 由来）を通常のハイフンに戻し、内部マーカーを除去
        s = s.replace('\u{2011}', "-");
        s = s.replace('\u{E000}', "");
        s.replace('\u{E001}', "")
    }
}

// ============================================================================
// ヘルパー
// ============================================================================

fn next_is_space(chars: &[char], i: usize) -> bool {
    i >= chars.len() || chars[i] == ' ' || chars[i] == '\t'
}

fn next_is_space_or(chars: &[char], i: usize, _c: char) -> bool {
    i < chars.len()
}

fn strip_sgr(s: &str) -> String {
    let mut out = String::new();
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' { in_esc = false; }
            continue;
        }
        if c == '\x1b' { in_esc = true; continue; }
        out.push(c);
    }
    out
}

/// \f や \* の名前部分（c / (xx / [xxx]）を読む
fn read_escape_name(chars: &[char], i: &mut usize) -> String {
    if *i >= chars.len() {
        return String::new();
    }
    match chars[*i] {
        '(' => {
            *i += 1;
            let mut s = String::new();
            for _ in 0..2 {
                if *i < chars.len() {
                    s.push(chars[*i]);
                    *i += 1;
                }
            }
            s
        }
        '[' => {
            *i += 1;
            let mut s = String::new();
            while *i < chars.len() && chars[*i] != ']' {
                s.push(chars[*i]);
                *i += 1;
            }
            if *i < chars.len() { *i += 1; }
            s
        }
        c => {
            *i += 1;
            c.to_string()
        }
    }
}

fn read_ref_name(chars: &[char], i: &mut usize) -> String {
    read_escape_name(chars, i)
}

/// \h'...' 形式の引数を読む
fn read_delim_arg(chars: &[char], i: &mut usize) -> Option<String> {
    if *i >= chars.len() {
        return None;
    }
    let delim = chars[*i];
    *i += 1;
    let mut s = String::new();
    while *i < chars.len() && chars[*i] != delim {
        s.push(chars[*i]);
        *i += 1;
    }
    if *i < chars.len() { *i += 1; }
    Some(s)
}

/// \s サイズエスケープを読み飛ばす
fn consume_size_escape(chars: &[char], i: &mut usize) {
    if *i >= chars.len() {
        return;
    }
    if chars[*i] == '+' || chars[*i] == '-' {
        *i += 1;
    }
    if *i >= chars.len() {
        return;
    }
    match chars[*i] {
        '(' => { *i += 3; }
        '[' => {
            while *i < chars.len() && chars[*i] != ']' { *i += 1; }
            if *i < chars.len() { *i += 1; }
        }
        '\'' => {
            *i += 1;
            while *i < chars.len() && chars[*i] != '\'' { *i += 1; }
            if *i < chars.len() { *i += 1; }
        }
        c if c.is_ascii_digit() => {
            *i += 1;
            // 1-3 で始まる 2 桁サイズ
            if ('1'..='3').contains(&c) && *i < chars.len() && chars[*i].is_ascii_digit() {
                *i += 1;
            }
        }
        _ => {}
    }
}

/// 複数行の \{ ... \} ブロックを収集する。
/// 戻り値は (行リスト, 先頭チャンク由来の行数)。
fn collect_block(first: &str, stream: &mut LineStream) -> (Vec<String>, usize) {
    let mut depth = 1;
    let mut lines = Vec::new();
    let mut from_chunk = 0usize;
    let mut is_first = true;

    let mut current = first.to_string();

    loop {
        // current から \{ / \} を探す
        let chars: Vec<char> = current.chars().collect();
        let mut content = String::new();
        let mut i = 0;
        let mut closed = false;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                if chars[i + 1] == '{' {
                    depth += 1;
                    content.push('\\');
                    content.push('{');
                    i += 2;
                    continue;
                }
                if chars[i + 1] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        closed = true;
                        break;
                    }
                    content.push('\\');
                    content.push('}');
                    i += 2;
                    continue;
                }
            }
            content.push(chars[i]);
            i += 1;
        }
        let trimmed = content.trim_end();
        if !trimmed.is_empty() && trimmed != "." {
            lines.push(content.clone());
            if is_first {
                from_chunk += 1;
            }
        }
        is_first = false;
        if closed {
            break;
        }
        match stream.next_line() {
            Some(l) => current = l,
            None => break,
        }
    }
    (lines, from_chunk)
}

/// 引数を分割（クォート対応、"" は " のリテラル）
fn split_args(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut arg = String::new();
        if chars[i] == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '"' {
                    if i + 1 < chars.len() && chars[i + 1] == '"' {
                        arg.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                arg.push(chars[i]);
                i += 1;
            }
            result.push(arg);
        } else {
            while i < chars.len() && chars[i] != ' ' && chars[i] != '\t' {
                // \（エスケープ）は次の文字ごと引数に含める（\  は改行しないスペース）
                if chars[i] == '\\' && i + 1 < chars.len() {
                    arg.push(chars[i]);
                    arg.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                arg.push(chars[i]);
                i += 1;
            }
            result.push(arg);
        }
    }
    result
}

fn is_man_macro(cmd: &str) -> bool {
    matches!(
        cmd,
        "TH" | "SH" | "SS" | "PP" | "P" | "LP" | "IP" | "TP" | "TQ" | "HP" | "PD"
            | "RS" | "RE" | "B" | "I" | "BR" | "RB" | "IR" | "RI" | "BI" | "IB"
            | "SM" | "SB" | "EX" | "EE" | "UR" | "UE" | "MT" | "ME" | "SY" | "YS"
            | "OP" | "DT" | "AT" | "UC"
    )
}

/// 左・中央・右の 3 分割行を作る（man のヘッダ/フッタ）
fn three_part(left: &str, center: &str, right: &str, width: usize) -> String {
    let lw = visible_width(left);
    let cw = visible_width(center);
    let rw = visible_width(right);
    let mut line = String::from(left);
    // 中央（groff と同じく切り上げ）
    let center_start = (width.saturating_sub(cw) + 1) / 2;
    let pad1 = center_start.saturating_sub(lw).max(1);
    line.push_str(&" ".repeat(pad1));
    line.push_str(center);
    // 右
    let cur = lw + pad1 + cw;
    let pad2 = width.saturating_sub(cur + rw).max(1);
    line.push_str(&" ".repeat(pad2));
    line.push_str(right);
    line
}

// ============================================================================
// コマンドライン
// ============================================================================

fn print_help() {
    println!(
        r#"使用法: groff [オプション]... [ファイル]...
       nroff [オプション]... [ファイル]...

roff 入力ファイルを整形して端末向けに出力します。
roff 言語のコア（レジスタ・文字列・マクロ定義・条件・式評価）と
man マクロパッケージを実装しています。

オプション:
  -T DEVICE       出力デバイス: utf8 (デフォルト), ascii, latin1, html
  -m NAME         マクロパッケージ（man は組み込み。an/man/mandoc を受理）
  -r REG=EXPR     数値レジスタを設定（例: -rLL=72n で行長 72 桁）
  -d STR=VAL      文字列を定義
  -c              SGR エスケープ（太字/下線）を無効化
  -a              テキスト近似出力（ascii 相当）
  -z              整形のみ行い出力を抑制
  -C              互換モード（受理のみ）
  -k, -t, -e, -p, -s, -R   プリプロセッサ指定（受理のみ）
  -w TYPE, -W TYPE         警告制御（受理のみ）
      --help      このヘルプを表示
  -v, --version   バージョン情報を表示

サポートする roff リクエスト:
  .br .sp .ll .in .ti .fi .nf .ad .na .ce .ft .ds .as .nr .rr .rm .rn .als
  .de .am .ig .if .ie .el .so .shift .return .nop .ex .ab .tm .ns .rs .ta .ls
  条件の \{{ ... \}} ブロック、数値式（単位 n m i v u c p 付き）に対応

サポートする man マクロ:
  .TH .SH .SS .PP/.P/.LP .IP .TP .TQ .HP .PD .RS .RE
  .B .I .BR .RB .IR .RI .BI .IB .SM .SB .EX .EE
  .UR/.UE .MT/.ME .SY/.YS .OP .DT

エスケープ:
  \fB \fI \fR \fP（フォント）, \*x \*(xx \*[xxx]（文字列）,
  \nx \n(xx \n[xxx]（レジスタ）, \(xx \[xxx]（特殊文字）, \w'...'（幅）,
  \h'...'（水平移動）, \c（行継続）, \& \- \e \~ \0 ほか

例:
  groff -man -Tutf8 ls.1          man ページを表示
  groff -man -rLL=72n ls.1        行長 72 桁で表示
  nroff -man ls.1 | less -R       ページャで表示"#
    );
}

fn print_version() {
    println!("groff (Rust実装) 1.0.0");
    println!("roff コア + man マクロパッケージ対応");
}

fn expand_glob_pattern(pattern: &str) -> Result<Vec<String>, String> {
    if pattern == "-" {
        return Ok(vec!["-".to_string()]);
    }
    let normalized = pattern.replace('\\', "/");
    if normalized.contains('*') || normalized.contains('?') || normalized.contains('[') {
        let paths: Vec<String> = glob(&normalized)
            .map_err(|e| format!("glob パターンエラー: {}", e))?
            .filter_map(Result::ok)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        if paths.is_empty() {
            Err(format!(
                "groff: '{}': そのようなファイルやディレクトリはありません",
                pattern
            ))
        } else {
            Ok(paths)
        }
    } else {
        Ok(vec![pattern.to_string()])
    }
}

fn parse_cli() -> Result<Config, String> {
    let args: Vec<String> = env::args().collect();
    let mut config = Config::default();

    if let Some(name) = args.first() {
        let lower = name.to_lowercase();
        if lower.contains("nroff") {
            config.device = Device::Utf8;
        }
    }
    if env::var("GROFF_NO_SGR").is_ok() {
        config.styled = false;
    }

    let parse_device = |s: &str| match s {
        "ascii" => Device::Ascii,
        "latin1" => Device::Latin1,
        "utf8" | "utf-8" => Device::Utf8,
        "html" => Device::Html,
        _ => Device::Utf8,
    };

    let mut i = 1;
    while i < args.len() {
        let arg = args[i].clone();
        match arg.as_str() {
            "--help" => {
                print_help();
                process::exit(0);
            }
            "--version" | "-v" => {
                print_version();
                process::exit(0);
            }
            "-T" => {
                i += 1;
                let v = args.get(i).ok_or("-T にはデバイス名が必要です")?;
                config.device = parse_device(v);
            }
            "-m" => {
                i += 1;
                // マクロパッケージ名は受理（man は組み込み）
                let _ = args.get(i).ok_or("-m にはマクロ名が必要です")?;
            }
            "-r" => {
                i += 1;
                let v = args.get(i).ok_or("-r にはレジスタ指定が必要です")?;
                if let Some((k, val)) = v.split_once('=') {
                    config.cli_registers.push((k.to_string(), val.to_string()));
                }
            }
            "-d" => {
                i += 1;
                let v = args.get(i).ok_or("-d には文字列指定が必要です")?;
                if let Some((k, val)) = v.split_once('=') {
                    config.cli_strings.push((k.to_string(), val.to_string()));
                }
            }
            "-c" => config.styled = false,
            "-a" => {
                config.device = Device::Ascii;
                config.styled = false;
            }
            "-z" => config.suppress_output = true,
            "-C" | "-k" | "-t" | "-e" | "-p" | "-s" | "-R" | "-E" | "-b" | "-i" | "-U"
            | "-S" => {}
            "-w" | "-W" | "-n" | "-o" | "-F" | "-M" | "-I" | "-P" | "-f" | "-K" | "-L" => {
                i += 1; // 引数を読み飛ばす
            }
            "--" => {
                for j in (i + 1)..args.len() {
                    config.files.extend(expand_glob_pattern(&args[j])?);
                }
                break;
            }
            s if s.starts_with("-T") => config.device = parse_device(&s[2..]),
            s if s.starts_with("-m") => {}
            s if s.starts_with("-r") => {
                let v = &s[2..];
                if let Some((k, val)) = v.split_once('=') {
                    config.cli_registers.push((k.to_string(), val.to_string()));
                } else if v.len() > 1 {
                    // -rN1 旧形式（1文字レジスタ）
                    let (k, val) = v.split_at(1);
                    config.cli_registers.push((k.to_string(), val.to_string()));
                }
            }
            s if s.starts_with("-d") => {
                let v = &s[2..];
                if let Some((k, val)) = v.split_once('=') {
                    config.cli_strings.push((k.to_string(), val.to_string()));
                } else if v.len() > 1 {
                    let (k, val) = v.split_at(1);
                    config.cli_strings.push((k.to_string(), val.to_string()));
                }
            }
            s if s.starts_with("-w") || s.starts_with("-W") || s.starts_with("-n")
                || s.starts_with("-o") || s.starts_with("-F") || s.starts_with("-M")
                || s.starts_with("-I") || s.starts_with("-P") || s.starts_with("-K")
                || s.starts_with("-L") => {}
            s if s.starts_with('-') && s != "-" => {
                // 未知のオプションは無視
            }
            _ => {
                config.files.extend(expand_glob_pattern(&arg)?);
            }
        }
        i += 1;
    }

    if config.files.is_empty() {
        config.files.push("-".to_string());
    }

    Ok(config)
}

fn main() {
    let config = match parse_cli() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("groff: {}", e);
            eprintln!("詳しくは 'groff --help' を参照してください");
            process::exit(1);
        }
    };

    let mut troff = Troff::new(&config);
    let mut had_error = false;

    for file_path in &config.files {
        let content = if file_path == "-" {
            let mut buf = String::new();
            if io::stdin().read_to_string(&mut buf).is_err() {
                eprintln!("groff: 標準入力を読み込めません");
                had_error = true;
                continue;
            }
            buf
        } else {
            match fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("groff: '{}': {}", file_path, e);
                    had_error = true;
                    continue;
                }
            }
        };

        troff
            .file_dirs
            .push(Path::new(file_path).parent().map(Path::to_path_buf).unwrap_or_default());
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        let mut noargs = Vec::new();
        troff.run_lines(lines, &mut noargs, false, 0);
        troff.file_dirs.pop();
    }

    let output = troff.finish();
    if !config.suppress_output {
        print!("{}", output);
        io::stdout().flush().ok();
    }
    if had_error {
        process::exit(2);
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn render(input: &str) -> String {
        render_with(input, |_| {})
    }

    fn render_with(input: &str, tweak: impl Fn(&mut Config)) -> String {
        let mut config = Config {
            styled: false,
            ..Default::default()
        };
        tweak(&mut config);
        let mut troff = Troff::new(&config);
        let lines: Vec<String> = input.lines().map(str::to_string).collect();
        let mut noargs = Vec::new();
        troff.run_lines(lines, &mut noargs, false, 0);
        troff.finish()
    }

    #[test]
    fn th_produces_header_and_footer() {
        let out = render(".TH LS 1 \"2026-01-01\" \"coreutils\" \"User Commands\"\n.SH NAME\nls - list");
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("LS(1)"));
        assert!(lines[0].contains("User Commands"));
        assert!(lines[0].ends_with("LS(1)"));
        let last = lines.last().unwrap();
        assert!(last.starts_with("coreutils"));
        assert!(last.contains("2026-01-01"));
        assert!(last.ends_with("LS(1)"));
    }

    #[test]
    fn sh_heading_at_column_zero_and_body_indented() {
        let out = render(".TH X 1\n.SH DESCRIPTION\nhello world");
        assert!(out.contains("\nDESCRIPTION\n"));
        assert!(out.contains("\n       hello world"));
    }

    #[test]
    fn tp_short_tag_shares_line_with_body() {
        let out = render(".TH X 1\n.SH OPT\n.TP\n-a\ndo all things");
        // タグ -a は 7 桁のインデント幅に収まるので本文と同じ行
        assert!(out.contains("       -a     do all things"), "output was:\n{}", out);
    }

    #[test]
    fn tp_long_tag_gets_own_line() {
        let out = render(".TH X 1\n.SH OPT\n.TP\n--very-long-option\ndescription here");
        assert!(out.contains("       --very-long-option\n              description here"),
            "output was:\n{}", out);
    }

    #[test]
    fn ip_with_tag_and_indent() {
        let out = render(".TH X 1\n.SH L\n.IP \\(bu 3\nitem text");
        assert!(out.contains("       \u{2022}  item text"), "output was:\n{}", out);
    }

    #[test]
    fn br_concatenates_alternating_args() {
        let out = render(".TH X 1\n.SH S\n.BR ls (1)\nrest");
        assert!(out.contains("ls(1)"), "output was:\n{}", out);
    }

    #[test]
    fn inline_font_escapes_produce_sgr() {
        let mut config = Config::default();
        config.styled = true;
        let mut troff = Troff::new(&config);
        let lines = vec![".TH X 1".to_string(), ".SH S".to_string(), "a \\fBbold\\fR b".to_string()];
        let mut noargs = Vec::new();
        troff.run_lines(lines, &mut noargs, false, 0);
        let out = troff.finish();
        assert!(out.contains("\x1b[1mbold\x1b[0m"), "output was:\n{:?}", out);
    }

    #[test]
    fn de_macro_definition_and_arguments() {
        let out = render(".de GR\nHello \\\\$1!\n..\n.GR world");
        assert!(out.contains("Hello world!"), "output was:\n{}", out);
    }

    #[test]
    fn string_and_register_interpolation() {
        let out = render(".ds foo BAR\n.nr x 42\nvalue \\*[foo] \\n[x] end");
        assert!(out.contains("value BAR 42 end"), "output was:\n{}", out);
    }

    #[test]
    fn if_condition_n_is_true_t_is_false() {
        let out = render(".if n YES-N\n.if t YES-T\ndone");
        assert!(out.contains("YES-N"));
        assert!(!out.contains("YES-T"));
    }

    #[test]
    fn ie_el_with_blocks() {
        let out = render(".ie n \\{\nline-n\n.\\}\n.el \\{\nline-t\n.\\}\nend");
        assert!(out.contains("line-n"), "output was:\n{}", out);
        assert!(!out.contains("line-t"));
    }

    #[test]
    fn numeric_comparison_condition() {
        let out = render(".nr x 5\n.if \\n[x]>3 BIG\n.if \\n[x]>9 HUGE\nend");
        assert!(out.contains("BIG"));
        assert!(!out.contains("HUGE"));
    }

    #[test]
    fn string_comparison_condition() {
        let out = render(".ds a foo\n.if '\\*[a]'foo' SAME\n.if '\\*[a]'bar' DIFF\nend");
        assert!(out.contains("SAME"));
        assert!(!out.contains("DIFF"));
    }

    #[test]
    fn width_escape_in_expressions() {
        // \w'abc' = 3 文字 = 72 単位 → 3n と等しい
        let out = render(".if \\w'abc'=3n W3\nend");
        assert!(out.contains("W3"), "output was:\n{}", out);
    }

    #[test]
    fn fill_and_wrap_at_line_length() {
        let long = "word ".repeat(30);
        let out = render_with(&format!(".TH X 1\n.SH S\n{}", long), |c| {
            c.cli_registers.push(("LL".to_string(), "40n".to_string()));
        });
        for line in out.lines() {
            assert!(visible_width(line) <= 40, "line too long: {:?}", line);
        }
    }

    #[test]
    fn justification_pads_to_right_margin() {
        let text = "aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm nnn ooo ppp qqq rrr";
        let out = render(&format!(".TH X 1\n.SH S\n{}\n{}", text, text));
        let body: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("       ") && !l.trim().is_empty())
            .collect();
        // 最終行以外は右端まで埋まる
        assert!(body.len() >= 2);
        assert_eq!(visible_width(body[0]), 78, "line was: {:?}", body[0]);
    }

    #[test]
    fn sentence_end_gets_two_spaces() {
        let out = render(".TH X 1\n.SH S\nFirst sentence.\nSecond one.");
        assert!(out.contains("sentence.  Second"), "output was:\n{}", out);
    }

    #[test]
    fn nf_preserves_layout() {
        let out = render(".TH X 1\n.SH S\n.nf\nkeep   spacing\n  indented\n.fi\nafter");
        assert!(out.contains("keep   spacing"));
        assert!(out.contains("  indented"));
    }

    #[test]
    fn rs_re_nest_indent() {
        let out = render(".TH X 1\n.SH S\ntop\n.RS\nnested\n.RE\nback");
        assert!(out.contains("\n       top"), "output was:\n{}", out);
        assert!(out.contains("\n              nested"), "output was:\n{}", out);
        assert!(out.contains("\n       back"), "output was:\n{}", out);
    }

    #[test]
    fn pd_zero_removes_paragraph_gap() {
        let out = render(".TH X 1\n.SH S\n.PD 0\n.TP\na\ntext1\n.TP\nb\ntext2\n.PD");
        assert!(!out.contains("text1\n\n"), "output was:\n{}", out);
    }

    #[test]
    fn ce_centers_lines() {
        let out = render(".TH X 1\n.SH S\n.ce 1\nmid");
        let line = out.lines().find(|l| l.contains("mid")).unwrap();
        let lead = line.len() - line.trim_start().len();
        assert!(lead > 30, "not centered: {:?}", line);
    }

    #[test]
    fn connect_escape_joins_lines() {
        let out = render(".TH X 1\n.SH S\nfoo\\c\nbar");
        assert!(out.contains("foobar"), "output was:\n{}", out);
    }

    #[test]
    fn special_chars_map_to_unicode() {
        let out = render(".TH X 1\n.SH S\na \\(em b \\(bu c");
        assert!(out.contains("\u{2014}"));
        assert!(out.contains("\u{2022}"));
    }

    #[test]
    fn ur_ue_renders_url_in_brackets() {
        let out = render(".TH X 1\n.SH S\nSee\n.UR https://example.com\n.UE .\nend");
        assert!(out.contains("\u{27E8}https://example.com\u{27E9}."), "output was:\n{}", out);
    }

    #[test]
    fn header_center_defaults_from_section() {
        let out = render(".TH FOO 5\ntext");
        assert!(out.lines().next().unwrap().contains("File Formats Manual"));
    }

    #[test]
    fn ig_ignores_block() {
        let out = render(".TH X 1\n.SH S\n.ig\nhidden text\n..\nvisible");
        assert!(!out.contains("hidden"));
        assert!(out.contains("visible"));
    }

    #[test]
    fn split_args_quotes() {
        assert_eq!(split_args("a \"b c\" d"), vec!["a", "b c", "d"]);
        assert_eq!(split_args("\"x \"\"y\"\" z\""), vec!["x \"y\" z"]);
    }

    #[test]
    fn eval_expr_units_and_operators() {
        assert_eq!(eval_expr("3n", 1.0), Some(72.0));
        assert_eq!(eval_expr("1i", 1.0), Some(240.0));
        // 優先順位なしの左結合: 1+2*3 = 9
        assert_eq!(eval_expr("1+2*3", 1.0), Some(9.0));
        assert_eq!(eval_expr("(1+2)*3", 1.0), Some(9.0));
    }
}
