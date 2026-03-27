//! 軽量 groff/nroff 実装
//! 
//! manページの表示に必要な基本的なroff/manマクロをサポート。
//! 完全なgroff互換ではなく、manページ表示に特化した軽量実装。

use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::process;

use glob::glob;

#[derive(Debug, Clone, Copy, PartialEq)]
enum OutputDevice {
    Ascii,      // ASCII端末出力（nroff互換）
    Utf8,       // UTF-8端末出力
    Latin1,     // Latin-1出力
    Html,       // HTML出力
    Ps,         // PostScript出力（未実装）
}

#[derive(Debug)]
#[allow(dead_code)]
struct Config {
    /// 入力ファイル
    files: Vec<String>,
    /// 出力デバイス
    device: OutputDevice,
    /// マクロパッケージ（-m）
    macros: Vec<String>,
    /// 警告レベル（-w）
    warnings: bool,
    /// 標準出力
    stdout: bool,
    /// ページ幅
    line_length: usize,
    /// 互換モード（-C）
    compat_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            files: Vec::new(),
            device: OutputDevice::Utf8,
            macros: Vec::new(),
            warnings: false,
            stdout: true,
            line_length: 78,
            compat_mode: false,
        }
    }
}

/// フォーマッタ状態
#[allow(dead_code)]
struct Formatter {
    /// 出力バッファ
    output: Vec<String>,
    /// 現在の行バッファ
    current_line: String,
    /// 行の残り幅
    line_length: usize,
    /// 現在位置
    current_pos: usize,
    /// インデント
    indent: usize,
    /// フォント状態（B=ボールド, I=イタリック, R=ローマン）
    font: char,
    /// 前のフォント
    prev_font: char,
    /// セクション名
    section: String,
    /// 埋め（fill）モード
    fill_mode: bool,
    /// 調整（adjust）モード  
    adjust_mode: bool,
    /// 出力デバイス
    device: OutputDevice,
    /// マクロ定義
    macros: HashMap<String, Vec<String>>,
    /// 文字列定義
    strings: HashMap<String, String>,
    /// 数値レジスタ
    registers: HashMap<String, i32>,
    /// 条件スタック
    condition_stack: Vec<bool>,
    /// 一時インデント
    temp_indent: Option<usize>,
    /// no-space フラグ
    no_space: bool,
}

impl Formatter {
    fn new(device: OutputDevice, line_length: usize) -> Self {
        let mut strings = HashMap::new();
        // 定義済み文字列
        strings.insert("R".to_string(), "®".to_string());
        strings.insert("lq".to_string(), "\u{201C}".to_string()); // "
        strings.insert("rq".to_string(), "\u{201D}".to_string()); // "
        strings.insert("Tm".to_string(), "™".to_string());
        
        Formatter {
            output: Vec::new(),
            current_line: String::new(),
            line_length,
            current_pos: 0,
            indent: 0,
            font: 'R',
            prev_font: 'R',
            section: String::new(),
            fill_mode: true,
            adjust_mode: true,
            device,
            macros: HashMap::new(),
            strings,
            registers: HashMap::new(),
            condition_stack: Vec::new(),
            temp_indent: None,
            no_space: false,
        }
    }

    /// 出力を取得
    fn get_output(&self) -> String {
        self.output.join("\n")
    }

    /// 行を出力
    fn flush_line(&mut self) {
        if !self.current_line.is_empty() || !self.no_space {
            let indent_str = " ".repeat(self.indent);
            self.output.push(format!("{}{}", indent_str, self.current_line));
        }
        self.current_line.clear();
        self.current_pos = 0;
        self.temp_indent = None;
        self.no_space = false;
    }

    /// 空行を出力
    fn blank_line(&mut self) {
        self.flush_line();
        self.output.push(String::new());
    }

    /// テキストを追加
    fn add_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let effective_indent = self.temp_indent.unwrap_or(self.indent);
        let available = self.line_length.saturating_sub(effective_indent);

        if self.fill_mode {
            for word in text.split_whitespace() {
                let word_len = visible_width(word);
                
                if self.current_pos > 0 && self.current_pos + 1 + word_len > available {
                    self.flush_line();
                }
                
                if self.current_pos > 0 {
                    self.current_line.push(' ');
                    self.current_pos += 1;
                }
                
                self.current_line.push_str(word);
                self.current_pos += word_len;
            }
        } else {
            // no-fill モード
            self.current_line.push_str(text);
            self.current_pos += visible_width(text);
        }
    }

    /// フォントを適用したテキストを返す
    fn apply_font(&self, text: &str, font: char) -> String {
        match self.device {
            OutputDevice::Ascii | OutputDevice::Utf8 => {
                match font {
                    'B' => format!("\x1b[1m{}\x1b[0m", text),  // Bold
                    'I' => format!("\x1b[4m{}\x1b[0m", text),  // Underline (for italic)
                    _ => text.to_string(),
                }
            }
            OutputDevice::Html => {
                match font {
                    'B' => format!("<b>{}</b>", text),
                    'I' => format!("<i>{}</i>", text),
                    _ => text.to_string(),
                }
            }
            _ => text.to_string(),
        }
    }

    /// manマクロを処理
    fn process_man_macro(&mut self, cmd: &str, args: &[&str]) {
        match cmd {
            // セクションヘッダ
            "SH" => {
                self.flush_line();
                self.blank_line();
                self.section = args.join(" ");
                let header = self.apply_font(&self.section.clone(), 'B');
                self.output.push(header);
                self.indent = 0;
            }
            // サブセクションヘッダ
            "SS" => {
                self.flush_line();
                self.blank_line();
                let text = args.join(" ");
                let header = format!("  {}", self.apply_font(&text, 'B'));
                self.output.push(header);
                self.indent = 3;
            }
            // タイトル
            "TH" => {
                self.flush_line();
                if args.len() >= 2 {
                    let title = format!("{}({})", args[0], args[1]);
                    let header = self.apply_font(&title, 'B');
                    self.output.push(header);
                    self.blank_line();
                }
            }
            // パラグラフ
            "PP" | "P" | "LP" => {
                self.flush_line();
                self.blank_line();
                self.indent = 7;
            }
            // インデントパラグラフ
            "IP" => {
                self.flush_line();
                self.blank_line();
                if !args.is_empty() {
                    let tag = self.expand_escapes(args[0]);
                    let tag = self.apply_font(&tag, 'B');
                    self.output.push(format!("       {}", tag));
                }
                self.indent = 14;
            }
            // タグ付きパラグラフ
            "TP" => {
                self.flush_line();
                self.blank_line();
                self.indent = 7;
                self.temp_indent = Some(7);
            }
            // 相対インデント開始
            "RS" => {
                self.flush_line();
                let inc: usize = args.get(0)
                    .and_then(|s| parse_dimension(s))
                    .unwrap_or(7);
                self.indent += inc;
            }
            // 相対インデント終了
            "RE" => {
                self.flush_line();
                let dec: usize = args.get(0)
                    .and_then(|s| parse_dimension(s))
                    .unwrap_or(7);
                self.indent = self.indent.saturating_sub(dec);
            }
            // ボールドテキスト
            "B" => {
                let text = args.join(" ");
                let text = self.expand_escapes(&text);
                let formatted = self.apply_font(&text, 'B');
                self.add_text(&formatted);
            }
            // イタリックテキスト
            "I" => {
                let text = args.join(" ");
                let text = self.expand_escapes(&text);
                let formatted = self.apply_font(&text, 'I');
                self.add_text(&formatted);
            }
            // ボールド+ローマン交互
            "BR" => {
                let mut is_bold = true;
                for arg in args {
                    let text = self.expand_escapes(arg);
                    let formatted = self.apply_font(&text, if is_bold { 'B' } else { 'R' });
                    self.add_text(&formatted);
                    is_bold = !is_bold;
                }
            }
            // ローマン+ボールド交互
            "RB" => {
                let mut is_roman = true;
                for arg in args {
                    let text = self.expand_escapes(arg);
                    let formatted = self.apply_font(&text, if is_roman { 'R' } else { 'B' });
                    self.add_text(&formatted);
                    is_roman = !is_roman;
                }
            }
            // イタリック+ローマン交互
            "IR" => {
                let mut is_italic = true;
                for arg in args {
                    let text = self.expand_escapes(arg);
                    let formatted = self.apply_font(&text, if is_italic { 'I' } else { 'R' });
                    self.add_text(&formatted);
                    is_italic = !is_italic;
                }
            }
            // ローマン+イタリック交互
            "RI" => {
                let mut is_roman = true;
                for arg in args {
                    let text = self.expand_escapes(arg);
                    let formatted = self.apply_font(&text, if is_roman { 'R' } else { 'I' });
                    self.add_text(&formatted);
                    is_roman = !is_roman;
                }
            }
            // ボールド+イタリック交互
            "BI" => {
                let mut is_bold = true;
                for arg in args {
                    let text = self.expand_escapes(arg);
                    let formatted = self.apply_font(&text, if is_bold { 'B' } else { 'I' });
                    self.add_text(&formatted);
                    is_bold = !is_bold;
                }
            }
            // イタリック+ボールド交互
            "IB" => {
                let mut is_italic = true;
                for arg in args {
                    let text = self.expand_escapes(arg);
                    let formatted = self.apply_font(&text, if is_italic { 'I' } else { 'B' });
                    self.add_text(&formatted);
                    is_italic = !is_italic;
                }
            }
            // 小さいテキスト
            "SM" => {
                let text = args.join(" ");
                self.add_text(&self.expand_escapes(&text));
            }
            // 小さいボールド
            "SB" => {
                let text = args.join(" ");
                let text = self.expand_escapes(&text);
                let formatted = self.apply_font(&text, 'B');
                self.add_text(&formatted);
            }
            // 改行
            "br" => {
                self.flush_line();
            }
            // 空白
            "sp" => {
                self.flush_line();
                let count: usize = args.get(0)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                for _ in 0..count {
                    self.output.push(String::new());
                }
            }
            // no-space モード
            "ns" => {
                self.no_space = true;
            }
            // space モードに戻す
            "rs" => {
                self.no_space = false;
            }
            // 例の開始（固定幅）
            "EX" => {
                self.flush_line();
                self.fill_mode = false;
            }
            // 例の終了
            "EE" => {
                self.flush_line();
                self.fill_mode = true;
            }
            // nf (no fill)
            "nf" => {
                self.flush_line();
                self.fill_mode = false;
            }
            // fi (fill)
            "fi" => {
                self.flush_line();
                self.fill_mode = true;
            }
            // コメント
            "\\\"" | "\\#" => {
                // 無視
            }
            // URLリンク（man-ext）
            "UR" => {
                if !args.is_empty() {
                    self.add_text("<");
                    self.add_text(args[0]);
                }
            }
            "UE" => {
                self.add_text(">");
            }
            // メールリンク
            "MT" => {
                if !args.is_empty() {
                    self.add_text("<");
                    self.add_text(args[0]);
                }
            }
            "ME" => {
                self.add_text(">");
            }
            // シノプシス開始/終了
            "SY" => {
                self.flush_line();
                if !args.is_empty() {
                    let cmd = self.apply_font(args[0], 'B');
                    self.add_text(&cmd);
                }
                self.indent = 14;
            }
            "YS" => {
                self.flush_line();
                self.indent = 7;
            }
            // オプション
            "OP" => {
                let mut text = String::from("[");
                if args.len() >= 1 {
                    text.push_str(&self.apply_font(args[0], 'B'));
                }
                if args.len() >= 2 {
                    text.push(' ');
                    text.push_str(&self.apply_font(args[1], 'I'));
                }
                text.push(']');
                self.add_text(&text);
            }
            _ => {
                // 未知のマクロは無視
            }
        }
    }

    /// roffリクエストを処理
    fn process_request(&mut self, cmd: &str, args: &[&str]) {
        match cmd {
            // 文字列定義
            "ds" => {
                if args.len() >= 2 {
                    let name = args[0].to_string();
                    let value = args[1..].join(" ");
                    self.strings.insert(name, value);
                }
            }
            // 数値レジスタ
            "nr" => {
                if args.len() >= 2 {
                    let name = args[0].to_string();
                    if let Ok(val) = args[1].parse() {
                        self.registers.insert(name, val);
                    }
                }
            }
            // 行長
            "ll" => {
                if let Some(len) = args.get(0).and_then(|s| parse_dimension(s)) {
                    self.line_length = len;
                }
            }
            // インデント
            "in" => {
                if let Some(ind) = args.get(0).and_then(|s| parse_dimension(s)) {
                    self.indent = ind;
                }
            }
            // 一時インデント
            "ti" => {
                if let Some(ind) = args.get(0).and_then(|s| parse_dimension(s)) {
                    self.temp_indent = Some(ind);
                }
            }
            // 調整モード
            "ad" => {
                self.adjust_mode = true;
            }
            "na" => {
                self.adjust_mode = false;
            }
            // ページ
            "bp" => {
                self.flush_line();
                self.output.push("\n--- Page Break ---\n".to_string());
            }
            // 条件
            "if" | "ie" => {
                // 簡易条件処理
                if !args.is_empty() {
                    let cond = evaluate_condition(args[0], &self.registers);
                    self.condition_stack.push(cond);
                    if cond && args.len() > 1 {
                        let rest = args[1..].join(" ");
                        if rest.starts_with("\\{") {
                            // 複数行条件
                        } else {
                            self.process_line(&rest);
                        }
                    }
                }
            }
            "el" => {
                let cond = self.condition_stack.pop().unwrap_or(false);
                if !cond && !args.is_empty() {
                    let rest = args.join(" ");
                    self.process_line(&rest);
                }
            }
            // マクロ定義
            "de" => {
                // 簡略化：定義は無視
            }
            // ソースファイル読み込み
            "so" => {
                // 簡略化：無視
            }
            _ => {
                // 未知のリクエストは無視
            }
        }
    }

    /// エスケープシーケンスを展開
    fn expand_escapes(&self, text: &str) -> String {
        let mut result = String::new();
        let mut chars = text.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('f') => {
                        // フォント変更（無視）
                        if chars.peek() == Some(&'(') {
                            chars.next();
                            chars.next();
                            chars.next();
                        } else if chars.peek() == Some(&'[') {
                            while let Some(c) = chars.next() {
                                if c == ']' { break; }
                            }
                        } else {
                            chars.next();
                        }
                    }
                    Some('*') => {
                        // 文字列展開
                        let name = if chars.peek() == Some(&'(') {
                            chars.next();
                            let c1 = chars.next().unwrap_or(' ');
                            let c2 = chars.next().unwrap_or(' ');
                            format!("{}{}", c1, c2)
                        } else if chars.peek() == Some(&'[') {
                            chars.next();
                            let mut name = String::new();
                            while let Some(c) = chars.next() {
                                if c == ']' { break; }
                                name.push(c);
                            }
                            name
                        } else {
                            chars.next().map(|c| c.to_string()).unwrap_or_default()
                        };
                        if let Some(val) = self.strings.get(&name) {
                            result.push_str(val);
                        }
                    }
                    Some('(') => {
                        // 2文字特殊文字
                        let c1 = chars.next().unwrap_or(' ');
                        let c2 = chars.next().unwrap_or(' ');
                        let special = format!("{}{}", c1, c2);
                        result.push_str(&expand_special_char(&special));
                    }
                    Some('[') => {
                        // 名前付き特殊文字
                        let mut name = String::new();
                        while let Some(c) = chars.next() {
                            if c == ']' { break; }
                            name.push(c);
                        }
                        result.push_str(&expand_special_char(&name));
                    }
                    Some('e') => result.push('\\'),
                    Some('&') => {} // ゼロ幅文字
                    Some('-') => result.push('-'),
                    Some('~') => result.push(' '), // 改行禁止スペース
                    Some('^') => {} // 1/12 em スペース
                    Some('|') => {} // 1/6 em スペース
                    Some('0') => result.push(' '), // 数字幅スペース
                    Some('"') => break, // コメント開始
                    Some('#') => break, // コメント開始
                    Some('\\') => result.push('\\'),
                    Some(c) => result.push(c),
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// 1行を処理
    fn process_line(&mut self, line: &str) {
        let line = line.trim_end();
        
        if line.is_empty() {
            if self.fill_mode {
                self.flush_line();
                self.blank_line();
            } else {
                self.flush_line();
            }
            return;
        }

        // コマンド行
        if line.starts_with('.') || line.starts_with('\'') {
            let line = &line[1..];
            let parts: Vec<&str> = split_args(line);
            
            if parts.is_empty() {
                return;
            }

            let cmd = parts[0];
            let args: Vec<&str> = parts[1..].to_vec();

            // manマクロ
            if is_man_macro(cmd) {
                self.process_man_macro(cmd, &args);
            } else {
                self.process_request(cmd, &args);
            }
        } else {
            // テキスト行
            let expanded = self.expand_escapes(line);
            self.add_text(&expanded);
        }
    }

    /// ファイルを処理
    fn process<R: BufRead>(&mut self, reader: R) -> Result<(), String> {
        for line in reader.lines() {
            let line = line.map_err(|e| format!("読み込みエラー: {}", e))?;
            self.process_line(&line);
        }
        self.flush_line();
        Ok(())
    }
}

/// 引数を分割（クォート対応）
fn split_args(line: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    let mut chars = line.char_indices().peekable();
    
    while let Some((i, c)) = chars.next() {
        if c == '"' {
            in_quote = !in_quote;
        } else if c.is_whitespace() && !in_quote {
            if i > start {
                let arg = line[start..i].trim_matches('"');
                if !arg.is_empty() {
                    result.push(arg);
                }
            }
            start = i + 1;
        }
    }
    
    if start < line.len() {
        let arg = line[start..].trim_matches('"');
        if !arg.is_empty() {
            result.push(arg);
        }
    }
    
    result
}

/// manマクロかどうか
fn is_man_macro(cmd: &str) -> bool {
    matches!(cmd, 
        "TH" | "SH" | "SS" | "PP" | "P" | "LP" | "IP" | "TP" |
        "RS" | "RE" | "B" | "I" | "BR" | "RB" | "IR" | "RI" |
        "BI" | "IB" | "SM" | "SB" | "sp" | "br" | "ns" | "rs" |
        "EX" | "EE" | "nf" | "fi" | "UR" | "UE" | "MT" | "ME" |
        "SY" | "YS" | "OP"
    )
}

/// 特殊文字を展開
fn expand_special_char(name: &str) -> String {
    match name {
        "em" | "\\-" => "—".to_string(),
        "en" => "–".to_string(),
        "hy" => "-".to_string(),
        "bu" => "•".to_string(),
        "sq" => "□".to_string(),
        "lq" => "\u{201C}".to_string(), // "
        "rq" => "\u{201D}".to_string(), // "
        "oq" => "\u{2018}".to_string(), // '
        "cq" => "\u{2019}".to_string(), // '
        "aq" => "'".to_string(),
        "dq" => "\"".to_string(),
        "Fo" => "«".to_string(),
        "Fc" => "»".to_string(),
        "co" => "©".to_string(),
        "rg" => "®".to_string(),
        "tm" => "™".to_string(),
        "rs" => "\\".to_string(),
        "ti" => "~".to_string(),
        "ha" => "^".to_string(),
        "ga" => "`".to_string(),
        "pl" => "+".to_string(),
        "mi" => "−".to_string(),
        "mu" => "×".to_string(),
        "di" => "÷".to_string(),
        "eq" => "=".to_string(),
        ">="|"ge" => "≥".to_string(),
        "<="|"le" => "≤".to_string(),
        "!=" => "≠".to_string(),
        "+-" => "±".to_string(),
        "if" => "∞".to_string(),
        "na" => "N/A".to_string(),
        "la" => "←".to_string(),
        "ra" => "→".to_string(),
        "ua" => "↑".to_string(),
        "da" => "↓".to_string(),
        _ => format!("[{}]", name),
    }
}

/// 寸法をパース
fn parse_dimension(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    
    // 単位を除去
    let num_str: String = s.chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    
    num_str.parse().ok()
}

/// 条件を評価
fn evaluate_condition(cond: &str, _registers: &HashMap<String, i32>) -> bool {
    let cond = cond.trim();
    
    // 出力デバイスチェック
    if cond == "n" || cond == "t" {
        return cond == "n"; // nroffモード
    }
    
    // 否定
    if cond.starts_with('!') {
        return !evaluate_condition(&cond[1..], _registers);
    }
    
    // 数値
    if let Ok(n) = cond.parse::<i32>() {
        return n != 0;
    }
    
    // デフォルトでtrue
    true
}

/// 可視幅を計算
fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        // CJK文字は幅2
        if is_wide_char(c) {
            width += 2;
        } else {
            width += 1;
        }
    }
    
    width
}

/// 幅広文字か判定
fn is_wide_char(c: char) -> bool {
    let cp = c as u32;
    // CJK Unified Ideographs and other wide characters
    (0x1100..=0x115F).contains(&cp) ||  // Hangul Jamo
    (0x2E80..=0x9FFF).contains(&cp) ||  // CJK
    (0xAC00..=0xD7A3).contains(&cp) ||  // Hangul Syllables
    (0xF900..=0xFAFF).contains(&cp) ||  // CJK Compatibility
    (0xFE10..=0xFE1F).contains(&cp) ||  // Vertical forms
    (0xFF00..=0xFF60).contains(&cp) ||  // Fullwidth forms
    (0x20000..=0x2FFFF).contains(&cp)   // CJK Extension B and beyond
}

fn print_help() {
    eprintln!(
        r#"使用法: groff [オプション]... [ファイル]...
       nroff [オプション]... [ファイル]...

roff入力ファイルを整形して出力します。
この実装はmanページ表示に特化した軽量版です。

オプション:
  -T DEVICE          出力デバイスを指定
                     ascii  ASCII端末
                     utf8   UTF-8端末（デフォルト）
                     latin1 Latin-1
                     html   HTML
  -m NAME            マクロパッケージを読み込む（man, mandoc）
  -t                 tbl プリプロセッサを実行（無視）
  -e                 eqn プリプロセッサを実行（無視）
  -p                 pic プリプロセッサを実行（無視）
  -c                 カラー出力を無効化
  -C                 互換モード
  -w WARNING         警告を有効化
      --help         このヘルプを表示
      --version      バージョン情報を表示

サポートするマクロ:
  man マクロ:
    .TH  タイトルヘッダ
    .SH  セクションヘッダ
    .SS  サブセクション
    .PP  パラグラフ
    .IP  インデントパラグラフ
    .TP  タグ付きパラグラフ
    .B   ボールド
    .I   イタリック
    .BR  ボールド/ローマン交互
    等

例:
  groff -man -Tutf8 ls.1          manページを表示
  nroff -man ls.1                 nroff互換モード
  groff -Thtml -man ls.1          HTML出力

globパターン対応:
  groff *.1                       複数ファイルを処理
"#
    );
}

fn print_version() {
    eprintln!("groff (Rust軽量実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
    eprintln!();
    eprintln!("この実装はmanページ表示に特化した軽量版です。");
    eprintln!("完全なgroff互換ではありません。");
}

/// glob展開
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if pattern == "-" {
        return Ok(vec!["-".to_string()]);
    }

    // WindowsでもPOSIX準拠の挙動に近づけるため、パス区切りをスラッシュに統一
    let normalized_pattern = pattern.replace('\\', "/");

    if normalized_pattern.contains('*') || normalized_pattern.contains('?') || normalized_pattern.contains('[') {
        let paths: Vec<String> = glob(&normalized_pattern)
            .map_err(|e| format!("globパターンエラー: {}", e))?
            .filter_map(Result::ok)
            .map(|p| p.to_string_lossy().to_string().replace('\\', "/"))  // 結果もスラッシュに統一
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

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().collect();
    let mut config = Config::default();
    
    // プログラム名をチェック（nroffとして呼ばれた場合）
    if let Some(name) = args.get(0) {
        if name.contains("nroff") {
            config.device = OutputDevice::Ascii;
        }
    }
    
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-T" {
            i += 1;
            if i >= args.len() {
                return Err("-T にはデバイス名が必要です".to_string());
            }
            config.device = match args[i].as_str() {
                "ascii" => OutputDevice::Ascii,
                "utf8" | "utf-8" => OutputDevice::Utf8,
                "latin1" => OutputDevice::Latin1,
                "html" => OutputDevice::Html,
                "ps" => OutputDevice::Ps,
                _ => OutputDevice::Utf8,
            };
        } else if arg.starts_with("-T") {
            config.device = match &arg[2..] {
                "ascii" => OutputDevice::Ascii,
                "utf8" | "utf-8" => OutputDevice::Utf8,
                "latin1" => OutputDevice::Latin1,
                "html" => OutputDevice::Html,
                "ps" => OutputDevice::Ps,
                _ => OutputDevice::Utf8,
            };
        } else if arg == "-m" {
            i += 1;
            if i >= args.len() {
                return Err("-m にはマクロ名が必要です".to_string());
            }
            config.macros.push(args[i].clone());
        } else if arg.starts_with("-m") {
            config.macros.push(arg[2..].to_string());
        } else if arg == "-C" {
            config.compat_mode = true;
        } else if arg == "-c" {
            // カラー無効（ASCII出力に切替）
        } else if arg == "-w" {
            config.warnings = true;
            i += 1; // 警告タイプをスキップ
        } else if arg == "-t" || arg == "-e" || arg == "-p" || arg == "-s" {
            // プリプロセッサオプション（無視）
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                let expanded = expand_glob(&args[j])?;
                config.files.extend(expanded);
            }
            break;
        } else if arg.starts_with('-') && arg != "-" {
            // 未知のオプションは無視
        } else {
            let expanded = expand_glob(arg)?;
            config.files.extend(expanded);
        }

        i += 1;
    }

    if config.files.is_empty() {
        config.files.push("-".to_string());
    }

    Ok(config)
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("groff: {}", e);
            eprintln!("詳しくは 'groff --help' を参照してください");
            process::exit(1);
        }
    };

    let mut formatter = Formatter::new(config.device, config.line_length);

    for file_path in &config.files {
        let result: Result<(), String> = if file_path == "-" {
            let stdin = io::stdin();
            formatter.process(stdin.lock())
        } else {
            match File::open(file_path) {
                Ok(file) => formatter.process(BufReader::new(file)),
                Err(e) => Err(format!("groff: '{}': {}", file_path, e)),
            }
        };

        if let Err(e) = result {
            eprintln!("{}", e);
            process::exit(1);
        }
    }

    let output = formatter.get_output();
    print!("{}", output);
    io::stdout().flush().ok();
}
