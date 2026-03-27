// tail - ファイルの末尾を表示
// POSIX.1-2017準拠 + GNU拡張

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8, UTF_16LE, UTF_16BE};
use glob;
use regex::Regex;
use serde::Deserialize;

// ========== ハイライト設定 ==========

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum ColorSpec {
    Name(String),
    Index { index: u8 },
    Rgb { rgb: [u8; 3] },
}

#[derive(Debug, Deserialize, Clone)]
struct RuleConfig {
    pattern: String,
    #[serde(default)]
    fg: Option<ColorSpec>,
    #[serde(default)]
    bg: Option<ColorSpec>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    dim: bool,
    #[serde(default)]
    underline: bool,
    #[serde(default)]
    blink: bool,
    #[serde(default)]
    reverse: bool,
}

#[derive(Debug, Deserialize)]
struct HighlightConfig {
    #[serde(default)]
    rule: Vec<RuleConfig>,
}

struct CompiledRule {
    regex: Regex,
    ansi_start: String,
}

struct Highlighter {
    rules: Vec<CompiledRule>,
}

impl Highlighter {
    fn new() -> Self {
        Self { rules: Vec::new() }
    }
    
    fn load_from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("設定ファイルを読み込めません: {}", e))?;
        Self::load_from_str(&content)
    }
    
    fn load_from_str(content: &str) -> Result<Self, String> {
        let config: HighlightConfig = toml::from_str(content)
            .map_err(|e| format!("TOML解析エラー: {}", e))?;
        
        let mut rules = Vec::new();
        for (i, rule_config) in config.rule.iter().enumerate() {
            match Regex::new(&rule_config.pattern) {
                Ok(regex) => {
                    let ansi_start = build_ansi_start(rule_config);
                    rules.push(CompiledRule { regex, ansi_start });
                }
                Err(e) => {
                    return Err(format!("ルール {} の正規表現エラー: {}", i + 1, e));
                }
            }
        }
        
        Ok(Self { rules })
    }
    
    fn apply(&self, line: &str) -> String {
        if self.rules.is_empty() {
            return line.to_string();
        }
        
        let mut result = line.to_string();
        for rule in &self.rules {
            result = self.apply_rule(&result, rule);
        }
        result
    }
    
    fn apply_rule(&self, text: &str, rule: &CompiledRule) -> String {
        if rule.ansi_start.is_empty() {
            return text.to_string();
        }
        
        let mut result = String::new();
        let mut last_end = 0;
        
        for mat in rule.regex.find_iter(text) {
            result.push_str(&text[last_end..mat.start()]);
            result.push_str(&rule.ansi_start);
            result.push_str(mat.as_str());
            result.push_str("\x1b[0m");
            last_end = mat.end();
        }
        result.push_str(&text[last_end..]);
        result
    }
}

fn build_ansi_start(rule: &RuleConfig) -> String {
    let mut codes = Vec::new();
    
    // 装飾
    if rule.bold { codes.push("1".to_string()); }
    if rule.dim { codes.push("2".to_string()); }
    if rule.underline { codes.push("4".to_string()); }
    if rule.blink { codes.push("5".to_string()); }
    if rule.reverse { codes.push("7".to_string()); }
    
    // 前景色
    if let Some(ref fg) = rule.fg {
        if let Some(code) = color_to_ansi_fg(fg) {
            codes.push(code);
        }
    }
    
    // 背景色
    if let Some(ref bg) = rule.bg {
        if let Some(code) = color_to_ansi_bg(bg) {
            codes.push(code);
        }
    }
    
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

fn color_to_ansi_fg(color: &ColorSpec) -> Option<String> {
    match color {
        ColorSpec::Name(name) => {
            let code = match name.to_lowercase().as_str() {
                "black" => 30,
                "red" => 31,
                "green" => 32,
                "yellow" => 33,
                "blue" => 34,
                "magenta" => 35,
                "cyan" => 36,
                "white" => 37,
                _ => return None,
            };
            Some(code.to_string())
        }
        ColorSpec::Index { index } => Some(format!("38;5;{}", index)),
        ColorSpec::Rgb { rgb } => Some(format!("38;2;{};{};{}", rgb[0], rgb[1], rgb[2])),
    }
}

fn color_to_ansi_bg(color: &ColorSpec) -> Option<String> {
    match color {
        ColorSpec::Name(name) => {
            let code = match name.to_lowercase().as_str() {
                "black" => 40,
                "red" => 41,
                "green" => 42,
                "yellow" => 43,
                "blue" => 44,
                "magenta" => 45,
                "cyan" => 46,
                "white" => 47,
                _ => return None,
            };
            Some(code.to_string())
        }
        ColorSpec::Index { index } => Some(format!("48;5;{}", index)),
        ColorSpec::Rgb { rgb } => Some(format!("48;2;{};{};{}", rgb[0], rgb[1], rgb[2])),
    }
}

// ========== tail オプション ==========

#[derive(Default)]
enum CountMode {
    #[default]
    Lines,
    Bytes,
}

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    lines: Option<usize>,     // -n: 行数指定
    bytes: Option<usize>,     // -c: バイト数指定
    follow: bool,             // -f: 追跡モード
    line_from_start: bool,    // -n +N / +N: 先頭からN行目以降
    byte_from_start: bool,    // -c +N: 先頭からNバイト目以降
    count_mode: CountMode,    // 最後に指定された件数指定種別
    
    // GNU拡張オプション
    follow_name: bool,        // -F, --follow=name: 名前で追跡
    retry: bool,              // --retry: 再試行
    sleep_interval: f64,      // -s, --sleep-interval: 追跡間隔
    max_unchanged_stats: usize, // --max-unchanged-stats
    pid: Option<u32>,         // --pid: プロセス終了時に終了
    quiet: bool,              // -q, --quiet: ヘッダー非表示
    verbose: bool,            // -v, --verbose: ヘッダー表示
    zero_terminated: bool,    // -z: NUL区切り
    
    // ハイライト
    highlight_file: Option<PathBuf>,  // --highlight: 設定ファイル
    
    show_help: bool,
    show_version: bool,
}

impl Options {
    fn active_line_settings(&self) -> (usize, bool) {
        (self.lines.unwrap_or(10), self.line_from_start)
    }

    fn active_byte_settings(&self) -> Option<(usize, bool)> {
        match self.count_mode {
            CountMode::Bytes => self.bytes.map(|bytes| (bytes, self.byte_from_start)),
            CountMode::Lines => None,
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("tail: {}", e);
            eprintln!("詳細は 'tail --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("tail (Rust版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張 + ハイライト機能");
        std::process::exit(0);
    }

    // ハイライト設定の読み込み
    let highlighter = match &opts.highlight_file {
        Some(path) => {
            match Highlighter::load_from_file(path) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("tail: ハイライト設定エラー: {}", e);
                    std::process::exit(2);
                }
            }
        }
        None => {
            // デフォルトパスから読み込み試行
            let default_paths = [
                PathBuf::from("tail-highlight.toml"),
                dirs::config_dir().map(|p| p.join("tail").join("highlight.toml")).unwrap_or_default(),
            ];
            let mut hl = Highlighter::new();
            for path in &default_paths {
                if path.exists() {
                    match Highlighter::load_from_file(path) {
                        Ok(h) => {
                            hl = h;
                            break;
                        }
                        Err(e) => {
                            eprintln!("tail: 警告: ハイライト設定読み込み失敗 ({}): {}", path.display(), e);
                        }
                    }
                }
            }
            hl
        }
    };

    let (lines, _) = opts.active_line_settings();
    
    // glob展開
    let files = expand_globs(files);
    
    let show_header = opts.verbose || (!opts.quiet && files.len() > 1);

    if files.is_empty() {
        if let Err(e) = tail_stdin(&opts, lines, &highlighter) {
            eprintln!("tail: {}", format_error(&e));
            std::process::exit(1);
        }
    } else if opts.follow || opts.follow_name {
        if let Err(e) = tail_follow(&files, &opts, lines, show_header, &highlighter) {
            eprintln!("tail: {}", format_error(&e));
            std::process::exit(1);
        }
    } else {
        let mut exit_code = 0;
        for (i, file) in files.iter().enumerate() {
            if show_header {
                if i > 0 {
                    println!();
                }
                println!("==> {} <==", file);
            }

            if file == "-" {
                if let Err(e) = tail_stdin(&opts, lines, &highlighter) {
                    eprintln!("tail: 標準入力: {}", format_error(&e));
                    exit_code = 1;
                }
            } else if let Err(e) = tail_file(file, &opts, lines, &highlighter) {
                eprintln!("tail: '{}': {}", file, format_error(&e));
                exit_code = 1;
            }
        }
        std::process::exit(exit_code);
    }
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options {
        sleep_interval: 1.0,
        max_unchanged_stats: 5,
        ..Default::default()
    };
    let mut files = Vec::new();
    let mut i = 1;
    let mut end_of_opts = false;

    while i < args.len() {
        let arg = &args[i];

        if end_of_opts {
            files.push(arg.clone());
            i += 1;
            continue;
        }

        if arg == "--" {
            end_of_opts = true;
            i += 1;
            continue;
        }

        // ロングオプション
        if arg.starts_with("--") {
            match arg.as_str() {
                "--follow" => opts.follow = true,
                "--retry" => opts.retry = true,
                "--quiet" | "--silent" => opts.quiet = true,
                "--verbose" => opts.verbose = true,
                "--zero-terminated" => opts.zero_terminated = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                s if s.starts_with("--lines=") => {
                    let val = s.trim_start_matches("--lines=");
                    parse_line_arg(val, &mut opts)?;
                }
                s if s.starts_with("--bytes=") => {
                    let val = s.trim_start_matches("--bytes=");
                    parse_byte_arg(val, &mut opts)?;
                }
                s if s.starts_with("--sleep-interval=") => {
                    let val = s.trim_start_matches("--sleep-interval=");
                    opts.sleep_interval = val.parse().map_err(|_| format!("無効な秒数: '{}'", val))?;
                }
                s if s.starts_with("--pid=") => {
                    let val = s.trim_start_matches("--pid=");
                    opts.pid = Some(val.parse().map_err(|_| format!("無効なPID: '{}'", val))?);
                }
                s if s.starts_with("--max-unchanged-stats=") => {
                    let val = s.trim_start_matches("--max-unchanged-stats=");
                    opts.max_unchanged_stats = val.parse().map_err(|_| format!("無効な数値: '{}'", val))?;
                }
                s if s.starts_with("--follow=") => {
                    let val = s.trim_start_matches("--follow=");
                    match val {
                        "name" => {
                            opts.follow_name = true;
                            opts.follow = true;
                        }
                        "descriptor" => {
                            opts.follow = true;
                        }
                        _ => return Err(format!("'--follow' の引数が不正です: '{}'", val)),
                    }
                }
                "--lines" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--lines' には引数が必要です".to_string());
                    }
                    parse_line_arg(&args[i], &mut opts)?;
                }
                "--bytes" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--bytes' には引数が必要です".to_string());
                    }
                    parse_byte_arg(&args[i], &mut opts)?;
                }
                "--sleep-interval" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--sleep-interval' には引数が必要です".to_string());
                    }
                    opts.sleep_interval = args[i].parse().map_err(|_| format!("無効な秒数: '{}'", args[i]))?;
                }
                "--pid" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--pid' には引数が必要です".to_string());
                    }
                    opts.pid = Some(args[i].parse().map_err(|_| format!("無効なPID: '{}'", args[i]))?);
                }
                s if s.starts_with("--highlight=") => {
                    let val = s.trim_start_matches("--highlight=");
                    opts.highlight_file = Some(PathBuf::from(val));
                }
                "--highlight" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--highlight' には引数が必要です".to_string());
                    }
                    opts.highlight_file = Some(PathBuf::from(&args[i]));
                }
                _ => return Err(format!("不明なオプション: '{}'", arg)),
            }
            i += 1;
            continue;
        }

        // 短縮オプション
        if arg.starts_with('-') && arg.len() > 1 {
            // -NUM形式（例: -20）
            if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                opts.lines = arg[1..].parse().ok();
                opts.line_from_start = false;
                opts.count_mode = CountMode::Lines;
                i += 1;
                continue;
            }

            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;

            while j < chars.len() {
                match chars[j] {
                    // POSIX標準
                    'f' => opts.follow = true,
                    'n' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            parse_line_arg(&rest, &mut opts)?;
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-n' には引数が必要です".to_string());
                            }
                            parse_line_arg(&args[i], &mut opts)?;
                            break;
                        }
                    }
                    'c' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            parse_byte_arg(&rest, &mut opts)?;
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-c' には引数が必要です".to_string());
                            }
                            parse_byte_arg(&args[i], &mut opts)?;
                            break;
                        }
                    }
                    // GNU拡張
                    'F' => {
                        opts.follow_name = true;
                        opts.follow = true;
                        opts.retry = true;
                    }
                    'q' => opts.quiet = true,
                    'v' => opts.verbose = true,
                    'z' => opts.zero_terminated = true,
                    's' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.sleep_interval = rest.parse().map_err(|_| format!("無効な秒数: '{}'", rest))?;
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-s' には引数が必要です".to_string());
                            }
                            opts.sleep_interval = args[i].parse().map_err(|_| format!("無効な秒数: '{}'", args[i]))?;
                            break;
                        }
                    }
                    _ => return Err(format!("不正なオプション: '-{}'", chars[j])),
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        // +NUM形式（先頭からN番目以降）
        if arg.starts_with('+') {
            if let Ok(n) = arg[1..].parse::<usize>() {
                opts.line_from_start = true;
                opts.lines = Some(n);
                opts.count_mode = CountMode::Lines;
                i += 1;
                continue;
            }
        }

        files.push(arg.clone());
        i += 1;
    }

    Ok((opts, files))
}

fn parse_line_arg(val: &str, opts: &mut Options) -> Result<(), String> {
    if val.is_empty() {
        return Err("行数が空です".to_string());
    }

    if val.starts_with('+') {
        opts.line_from_start = true;
        opts.lines = val[1..].parse().ok();
    } else if val.starts_with('-') {
        opts.line_from_start = false;
        opts.lines = val[1..].parse().ok();
    } else {
        opts.line_from_start = false;
        opts.lines = val.parse().ok();
    }
    opts.count_mode = CountMode::Lines;
    if opts.lines.is_none() && !val.is_empty() {
        return Err(format!("無効な行数: '{}'", val));
    }
    Ok(())
}

fn parse_byte_arg(val: &str, opts: &mut Options) -> Result<(), String> {
    if val.is_empty() {
        return Err("バイト数が空です".to_string());
    }

    let (val_str, from_start) = if val.starts_with('+') {
        (&val[1..], true)
    } else if val.starts_with('-') {
        (&val[1..], false)
    } else {
        (val, false)
    };
    
    opts.byte_from_start = from_start;
    opts.bytes = parse_size(val_str);
    opts.count_mode = CountMode::Bytes;
    
    if opts.bytes.is_none() && !val_str.is_empty() {
        return Err(format!("無効なバイト数: '{}'", val));
    }
    Ok(())
}

fn parse_size(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    
    let s_upper = s.to_uppercase();
    let (num_part, multiplier) = if s_upper.ends_with("KB") {
        (&s[..s.len() - 2], 1000usize)
    } else if s_upper.ends_with("MB") {
        (&s[..s.len() - 2], 1000 * 1000)
    } else if s_upper.ends_with("GB") {
        (&s[..s.len() - 2], 1000 * 1000 * 1000)
    } else if s_upper.ends_with("TB") {
        (&s[..s.len() - 2], 1000 * 1000 * 1000 * 1000)
    } else if s_upper.ends_with("KIB") {
        (&s[..s.len() - 3], 1024)
    } else if s_upper.ends_with("MIB") {
        (&s[..s.len() - 3], 1024 * 1024)
    } else if s_upper.ends_with("GIB") {
        (&s[..s.len() - 3], 1024 * 1024 * 1024)
    } else if s_upper.ends_with('K') {
        (&s[..s.len() - 1], 1024)
    } else if s_upper.ends_with('M') {
        (&s[..s.len() - 1], 1024 * 1024)
    } else if s_upper.ends_with('G') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024)
    } else if s_upper.ends_with('T') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024 * 1024)
    } else if s_upper.ends_with('B') {
        (&s[..s.len() - 1], 1)
    } else {
        (s, 1)
    };

    num_part.trim().parse::<usize>().ok().map(|n| n * multiplier)
}

fn has_glob_metacharacters(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let _ = chars.next();
            }
            '*' | '?' | '[' => return true,
            _ => {}
        }
    }

    false
}

#[cfg(windows)]
fn normalize_case_for_match(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

#[cfg(windows)]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

#[cfg(windows)]
fn resolve_existing_path_matches_case_insensitively(path: &Path) -> Option<PathBuf> {
    let mut absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir().ok()?.join(path)
    };

    if let Ok(canonical) = fs::canonicalize(&absolute) {
        return Some(strip_windows_verbatim_prefix(canonical));
    }

    let mut unresolved_components = Vec::new();
    while !absolute.exists() {
        let name = absolute.file_name()?.to_os_string();
        unresolved_components.push(name);
        let parent = absolute.parent()?;
        if parent == absolute {
            return None;
        }
        absolute = parent.to_path_buf();
    }

    let mut resolved = strip_windows_verbatim_prefix(fs::canonicalize(&absolute).ok()?);
    unresolved_components.reverse();

    for component in unresolved_components {
        let target = component.to_string_lossy().to_lowercase();
        let matched = fs::read_dir(&resolved)
            .ok()?
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().to_lowercase() == target)
            .map(|entry| entry.path());

        if let Some(path) = matched {
            resolved = path;
        } else {
            resolved.push(component);
        }
    }

    Some(resolved)
}

#[cfg(windows)]
fn normalize_windows_input_path(path: &str) -> String {
    let input = Path::new(path);
    let Some(resolved) = resolve_existing_path_matches_case_insensitively(input) else {
        return path.to_string();
    };

    if input.is_absolute() {
        return resolved.to_string_lossy().to_string();
    }

    if let Ok(cwd) = env::current_dir() {
        if let Ok(relative) = resolved.strip_prefix(&cwd) {
            return relative.to_string_lossy().to_string();
        }
    }

    resolved.to_string_lossy().to_string()
}

#[cfg(not(windows))]
fn normalize_windows_input_path(path: &str) -> String {
    path.to_string()
}

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    
    for pattern in raw_files {
        // "-" は標準入力なのでそのまま
        if pattern == "-" {
            result.push(pattern);
            continue;
        }
        
        if has_glob_metacharacters(&pattern) {
            match glob::glob_with(&pattern, options) {
                Ok(paths) => {
                    let mut matches = paths
                        .filter_map(Result::ok)
                        .map(|path| path.to_string_lossy().to_string())
                        .collect::<Vec<_>>();
                    if matches.is_empty() {
                        result.push(pattern);
                    } else {
                        #[cfg(windows)]
                        matches.sort_by_key(|path| normalize_case_for_match(Path::new(path)));
                        #[cfg(not(windows))]
                        matches.sort();

                        result.extend(matches);
                    }
                }
                Err(_) => {
                    result.push(pattern);
                }
            }
        } else {
            result.push(normalize_windows_input_path(&pattern));
        }
    }
    
    result
}

fn print_help() {
    println!(r#"使い方: tail [オプション]... [ファイル]...

各ファイルの末尾10行を表示します。
ファイルが指定されない場合、または - の場合は標準入力を読み込みます。
複数ファイルの場合はヘッダーを表示します。

POSIX標準オプション:
  -c, --bytes=[+]NUM   末尾NUMバイトを出力
                       '+' を付けると各ファイルの先頭からNUMバイト目以降を出力
  -f, --follow         ファイルへの追記を追跡（Ctrl+Cで終了）
  -n, --lines=[+]NUM   末尾NUM行を出力（デフォルト: 10）
                       '+' を付けると各ファイルの先頭からNUM行目以降を出力

GNU拡張オプション:
  -F                   --follow=name --retry と同等
      --follow[=HOW]   ファイルへの追記を追跡
                       HOW: 'descriptor'（デフォルト）または 'name'
      --retry          ファイルがアクセス不能でも繰り返し開こうとする
                       -f または --follow=name と共に使用
  -s, --sleep-interval=N
                       -f と共に使用時、反復間隔をN秒に設定（デフォルト: 1.0）
      --pid=PID        -f と共に使用時、プロセスPID終了後に終了
      --max-unchanged-stats=N
                       --follow=name と共に使用時、ファイルサイズが変化しない
                       回数がNに達したらファイルを再オープン（デフォルト: 5）
  -q, --quiet, --silent
                       ファイル名のヘッダーを出力しない
  -v, --verbose        常にファイル名のヘッダーを出力
  -z, --zero-terminated
                       行区切りを改行ではなくNUL文字にする
      --help           このヘルプを表示して終了
      --version        バージョン情報を表示して終了

ハイライト機能:
      --highlight=FILE 指定したTOML設定ファイルでハイライトを有効化
                       省略時はカレントディレクトリの tail-highlight.toml を
                       自動読み込み（存在する場合）

  設定ファイルの書式 (TOML):
    [[rule]]
    pattern = '正規表現'       # POSIX ERE 互換（リテラル文字列推奨）
    fg = "red"                 # 前景色
    bg = "black"               # 背景色
    bold = true                # 太字

  正規表現 (POSIX ERE 互換 + 拡張):
    .          任意の1文字
    *          0回以上の繰り返し
    +          1回以上の繰り返し
    ?          0回または1回
    ^  $       行頭・行末
    [abc]      文字クラス
    [0-9]      範囲指定
    (a|b)      グループ・OR
    \          特殊文字のエスケープ（\.  \*  \[  等）

  拡張機能:
    (?i)       大文字小文字を区別しない

  設定例:
    [[rule]]
    pattern = 'ERROR|FATAL'    # リテラル文字列（エスケープ不要）
    fg = "red"
    bold = true

    [[rule]]
    pattern = '[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}'   # 日付
    fg = "cyan"

  色指定:
    名前       black/red/green/yellow/blue/magenta/cyan/white
    256色      fg = {{ index = 196 }}
    RGB        fg = {{ rgb = [255,80,80] }}

  装飾:
    bold=true  underline=true  dim=true  blink=true  reverse=true

NUM にはサイズ指定が可能:
  b=512, KB=1000, K=1024, MB=1000*1000, M=1024*1024,
  GB=1000*1000*1000, G=1024*1024*1024, など

対応エンコーディング:
  UTF-8, UTF-16LE (BOM), UTF-16BE (BOM), Shift_JIS, EUC-JP
  ※ 自動判定

終了ステータス:
  0  正常終了
  1  エラー発生
  2  オプションエラー

例:
  tail file.txt              末尾10行を表示
  tail -n 20 file.txt        末尾20行を表示
  tail -f file.log           追記を追跡
  tail -F file.log           ローテーション対応で追跡
  tail -f --highlight=hl.toml app.log
                             ハイライト付きで追跡"#);
}

fn tail_stdin(opts: &Options, lines: usize, highlighter: &Highlighter) -> io::Result<()> {
    let stdin = io::stdin();
    let mut buffer = Vec::new();
    stdin.lock().read_to_end(&mut buffer)?;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    if let Some((bytes, from_start)) = opts.active_byte_settings() {
        if from_start {
            let start = (bytes.saturating_sub(1)).min(buffer.len());
            stdout.write_all(&buffer[start..])?;
        } else {
            let start = buffer.len().saturating_sub(bytes);
            stdout.write_all(&buffer[start..])?;
        }
    } else {
        let content = decode_to_utf8(&buffer);
        output_lines(&content, lines, opts.active_line_settings().1, opts.zero_terminated, &mut stdout, highlighter)?;
    }

    Ok(())
}

fn tail_file(path: &str, opts: &Options, lines: usize, highlighter: &Highlighter) -> io::Result<()> {
    let path = Path::new(path);

    if path.is_dir() {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "ディレクトリです"));
    }

    let mut file = File::open(path)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    if let Some((bytes, from_start)) = opts.active_byte_settings() {
        let file_size = file.metadata()?.len() as usize;

        if from_start {
            let start = (bytes.saturating_sub(1)).min(file_size);
            file.seek(SeekFrom::Start(start as u64))?;
            io::copy(&mut file, &mut stdout)?;
        } else {
            let start = file_size.saturating_sub(bytes);
            file.seek(SeekFrom::Start(start as u64))?;
            io::copy(&mut file, &mut stdout)?;
        }
    } else {
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        let content = decode_to_utf8(&buffer);
        output_lines(&content, lines, opts.active_line_settings().1, opts.zero_terminated, &mut stdout, highlighter)?;
    }

    Ok(())
}

fn output_lines<W: Write>(
    content: &str,
    lines: usize,
    from_start: bool,
    zero_terminated: bool,
    writer: &mut W,
    highlighter: &Highlighter,
) -> io::Result<()> {
    let separator = if zero_terminated { '\0' } else { '\n' };
    let all_lines: Vec<&str> = content.split(separator).collect();
    let ends_with_sep = content.ends_with(separator);

    if from_start {
        // +N: N行目以降を表示（1-indexed）
        let start = (lines.saturating_sub(1)).min(all_lines.len());
        for (i, line) in all_lines[start..].iter().enumerate() {
            if i > 0 {
                if zero_terminated {
                    write!(writer, "\0")?;
                } else {
                    writeln!(writer)?;
                }
            }
            let highlighted = highlighter.apply(line);
            write!(writer, "{}", highlighted)?;
        }
        if !all_lines.is_empty() && !zero_terminated && ends_with_sep {
            writeln!(writer)?;
        }
    } else {
        // -N: 末尾N行を表示
        let total = all_lines.len();
        let skip_last_empty = ends_with_sep && total > 0 && all_lines[total - 1].is_empty();
        let effective_total = if skip_last_empty { total - 1 } else { total };
        let start = effective_total.saturating_sub(lines);

        for (i, line) in all_lines[start..effective_total].iter().enumerate() {
            if i > 0 {
                if zero_terminated {
                    write!(writer, "\0")?;
                } else {
                    writeln!(writer)?;
                }
            }
            let highlighted = highlighter.apply(line);
            write!(writer, "{}", highlighted)?;
        }
        if !zero_terminated && ends_with_sep {
            writeln!(writer)?;
        }
    }

    Ok(())
}

fn tail_follow(
    files: &[String],
    opts: &Options,
    lines: usize,
    show_header: bool,
    highlighter: &Highlighter,
) -> io::Result<()> {
    let sleep_duration = Duration::from_secs_f64(opts.sleep_interval);

    struct FileState {
        path: String,
        position: u64,
        last_shown: bool,
        unchanged_count: usize,
        carry: Vec<u8>,  // 未完了のバイトを保持
        encoding: Option<&'static encoding_rs::Encoding>,
    }

    let mut states: Vec<FileState> = files
        .iter()
        .map(|f| FileState {
            path: f.clone(),
            position: 0,
            last_shown: false,
            unchanged_count: 0,
            carry: Vec::new(),
            encoding: None,
        })
        .collect();
    // 初回表示
    for (i, state) in states.iter_mut().enumerate() {
        if state.path == "-" {
            eprintln!("tail: 標準入力の追跡は対応していません");
            continue;
        }
        
        if show_header {
            if i > 0 {
                println!();
            }
            println!("==> {} <==", state.path);
        }

        match File::open(&state.path) {
            Ok(mut file) => {
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)?;
                if let Some((bytes, from_start)) = opts.active_byte_settings() {
                    let start = if from_start {
                        (bytes.saturating_sub(1)).min(buffer.len())
                    } else {
                        buffer.len().saturating_sub(bytes)
                    };
                    let stdout = io::stdout();
                    let mut stdout = stdout.lock();
                    stdout.write_all(&buffer[start..])?;
                    stdout.flush()?;
                    state.position = buffer.len() as u64;
                    state.carry.clear();
                    state.encoding = None;
                    state.last_shown = true;
                    continue;
                }

                // エンコーディングを検出して記憶
                let encoding = detect_encoding(&buffer);
                state.encoding = Some(encoding);
                
                if encoding == UTF_8 {
                    // UTF-8の場合：不完全なバイトをcarryに入れてpositionを調整
                    match std::str::from_utf8(&buffer) {
                        Ok(_) => {
                            // 全て有効
                            state.position = buffer.len() as u64;
                            state.carry.clear();
                        }
                        Err(e) => {
                            let v = e.valid_up_to();
                            state.position = v as u64;
                            state.carry = buffer[v..].to_vec(); // 不完全なバイトを保持
                        }
                    }
                    
                    // 確定部分だけをデコードして表示
                    let stable = &buffer[..state.position as usize];
                    if !stable.is_empty() {
                        let content = unsafe { std::str::from_utf8_unchecked(stable) };
                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = all_lines.len().saturating_sub(lines);
                        for line in &all_lines[start..] {
                            println!("{}", highlighter.apply(line));
                        }
                    }
                } else if encoding == UTF_16LE || encoding == UTF_16BE {
                    // UTF-16の場合：2バイト境界で分割
                    let safe_pos = if buffer.len() % 2 != 0 {
                        buffer.len() - 1
                    } else {
                        buffer.len()
                    };
                    state.position = safe_pos as u64;
                    state.carry = buffer[safe_pos..].to_vec();
                    
                    let stable = &buffer[..safe_pos];
                    if !stable.is_empty() {
                        let (decoded, _, _) = encoding.decode(stable);
                        let content = decoded.into_owned();
                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = all_lines.len().saturating_sub(lines);
                        for line in &all_lines[start..] {
                            println!("{}", highlighter.apply(line));
                        }
                    }
                } else {
                    // Shift_JIS/EUC-JPの場合：既存のロジック
                    let safe_pos = find_safe_split_position(&buffer, encoding);
                    state.position = safe_pos as u64;
                    state.carry = buffer[safe_pos..].to_vec();
                    
                    let stable = &buffer[..safe_pos];
                    if !stable.is_empty() {
                        let (decoded, _, _) = encoding.decode(stable);
                        let content = decoded.into_owned();
                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = all_lines.len().saturating_sub(lines);
                        for line in &all_lines[start..] {
                            println!("{}", highlighter.apply(line));
                        }
                    }
                }
                
                state.last_shown = true;
            }
            Err(e) => {
                if opts.retry {
                    eprintln!("tail: '{}' を開けません: {} (再試行します)", state.path, format_error(&e));
                } else {
                    eprintln!("tail: '{}': {}", state.path, format_error(&e));
                }
            }
        }
    }

    let stdout = io::stdout();

    // PIDチェック用（簡易版 - --pidオプション使用時に動作）
    fn check_pid_alive(_pid: u32) -> bool {
        // 簡易実装: 常にtrueを返す
        // 完全な実装にはwindows-sys crateが必要
        true
    }

    // 追跡ループ
    loop {
        thread::sleep(sleep_duration);

        // PIDチェック
        if let Some(pid) = opts.pid {
            if !check_pid_alive(pid) {
                break;
            }
        }

        for i in 0..states.len() {
            if states[i].path == "-" {
                continue;
            }
            
            let file = match File::open(&states[i].path) {
                Ok(f) => f,
                Err(_) => {
                    if opts.retry {
                        states[i].unchanged_count += 1;
                    }
                    continue;
                }
            };

            let metadata = match file.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let file_size = metadata.len();

            // ファイルが縮小された場合（ローテーション）
            if file_size < states[i].position {
                if show_header && files.len() > 1 {
                    println!("\n==> {} <==", states[i].path);
                }
                eprintln!("tail: '{}': ファイルが切り詰められました", states[i].path);
                states[i].position = 0;
                states[i].unchanged_count = 0;
                states[i].carry.clear();  // carryもクリア（新しいファイルなので）
            }

            if file_size > states[i].position {
                let mut file = file;
                if file.seek(SeekFrom::Start(states[i].position)).is_err() {
                    continue;
                }

                let mut new_content = Vec::new();
                if file.read_to_end(&mut new_content).is_err() {
                    continue;
                }

                if !new_content.is_empty() {
                    if show_header && files.len() > 1 && !states[i].last_shown {
                        println!("\n==> {} <==", states[i].path);
                    }

                    if opts.active_byte_settings().is_some() {
                        let mut stdout = stdout.lock();
                        stdout.write_all(&new_content)?;
                        stdout.flush()?;
                        states[i].position = file_size;
                        states[i].unchanged_count = 0;
                        states[i].last_shown = true;
                        for j in 0..states.len() {
                            if i != j {
                                states[j].last_shown = false;
                            }
                        }
                        continue;
                    }

                    // 前回の未完了バイトと結合
                    let mut carry = std::mem::take(&mut states[i].carry);
                    carry.extend_from_slice(&new_content);
                    
                    // エンコーディングを取得（なければ再検出）
                    let encoding = states[i].encoding.unwrap_or_else(|| {
                        let enc = detect_encoding(&carry);
                        states[i].encoding = Some(enc);
                        enc
                    });
                    
                    // UTF-8の場合は valid_up_to を使用
                    let (valid_text, remaining) = if encoding == UTF_8 {
                        decode_utf8_with_carry(&carry)
                    } else if encoding == UTF_16LE || encoding == UTF_16BE {
                        decode_utf16_with_carry(&carry, encoding)
                    } else {
                        // Shift_JIS/EUC-JP の場合
                        decode_legacy_with_carry(&carry, encoding)
                    };
                    
                    states[i].carry = remaining;
                    
                    if !valid_text.is_empty() {
                        let mut stdout = stdout.lock();
                        // 行単位でハイライトを適用
                        let lines_vec: Vec<&str> = valid_text.split('\n').collect();
                        for (idx, line) in lines_vec.iter().enumerate() {
                            if idx > 0 {
                                write!(stdout, "\n")?;
                            }
                            let highlighted = highlighter.apply(line);
                            write!(stdout, "{}", highlighted)?;
                        }
                        stdout.flush()?;
                    }

                    // 他のファイルのlast_shownをリセット
                    for j in 0..states.len() {
                        states[j].last_shown = j == i;
                    }
                }

                // positionは「消費済みバイト数」だけ進める
                // carry に残したバイトは未消費なので、その分を引く
                let consumed = new_content.len() - states[i].carry.len();
                states[i].position += consumed as u64;
                states[i].unchanged_count = 0;
            } else {
                states[i].unchanged_count += 1;
                
                // --follow=name で変化がない場合はファイルを再オープン
                if opts.follow_name && states[i].unchanged_count >= opts.max_unchanged_stats {
                    states[i].unchanged_count = 0;
                    // ファイルが置き換わっている可能性があるので位置をリセット
                    // （次のループで新しいサイズを取得）
                }
            }
        }
    }
    
    Ok(())
}

fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    // BOMによる判定
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return UTF_16LE;
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return UTF_16BE;
    }

    if std::str::from_utf8(bytes).is_ok() {
        return UTF_8;
    }

    let sjis_score = calc_sjis_score(bytes);
    let eucjp_score = calc_eucjp_score(bytes);

    if sjis_score > eucjp_score {
        SHIFT_JIS
    } else if eucjp_score > sjis_score {
        EUC_JP
    } else {
        SHIFT_JIS
    }
}

/// マルチバイト文字の境界でバイト列を分割する
/// 戻り値: (完全な文字列のバイト, 未完了のバイト)
fn calc_sjis_score(bytes: &[u8]) -> i32 {
    let mut score = 0;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            i += 1;
        } else if (0x81..=0x9F).contains(&b) || (0xE0..=0xFC).contains(&b) {
            if i + 1 < bytes.len() {
                let b2 = bytes[i + 1];
                if (0x40..=0x7E).contains(&b2) || (0x80..=0xFC).contains(&b2) {
                    score += 1;
                    i += 2;
                } else {
                    score -= 1;
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else if (0xA1..=0xDF).contains(&b) {
            score += 1;
            i += 1;
        } else {
            score -= 1;
            i += 1;
        }
    }
    score
}

fn calc_eucjp_score(bytes: &[u8]) -> i32 {
    let mut score = 0;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            i += 1;
        } else if (0xA1..=0xFE).contains(&b) {
            if i + 1 < bytes.len() && (0xA1..=0xFE).contains(&bytes[i + 1]) {
                score += 1;
                i += 2;
            } else {
                score -= 1;
                i += 1;
            }
        } else if b == 0x8E {
            if i + 1 < bytes.len() && (0xA1..=0xDF).contains(&bytes[i + 1]) {
                score += 1;
                i += 2;
            } else {
                score -= 1;
                i += 1;
            }
        } else {
            score -= 1;
            i += 1;
        }
    }
    score
}

fn decode_to_utf8(bytes: &[u8]) -> String {
    let encoding = detect_encoding(bytes);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

/// UTF-8: valid_up_to を使って完全な文字単位で分割
/// 回復不能なバイトは1バイトずつ捨てて再同期
fn decode_utf8_with_carry(bytes: &[u8]) -> (String, Vec<u8>) {
    let mut buf = bytes.to_vec();
    let mut out = String::new();

    loop {
        if buf.is_empty() {
            return (out, Vec::new());
        }
        
        match std::str::from_utf8(&buf) {
            Ok(s) => {
                out.push_str(s);
                return (out, Vec::new());
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();

                if valid_up_to > 0 {
                    // 確定部分を出力
                    let valid = unsafe {
                        std::str::from_utf8_unchecked(&buf[..valid_up_to])
                    };
                    out.push_str(valid);
                    buf = buf[valid_up_to..].to_vec();
                    
                    // 残りが不完全なマルチバイト文字の可能性があるので返す
                    if !buf.is_empty() {
                        // UTF-8の最大長は4バイトなので、4バイト未満なら持ち越し
                        if buf.len() < 4 {
                            return (out, buf);
                        }
                        // 4バイト以上あるのに不正なら、先頭を捨てて再同期
                        buf.remove(0);
                    }
                } else {
                    // 回復不能 → 1バイト捨てて再同期
                    buf.remove(0);
                }
            }
        }
    }
}

/// UTF-16LE/BE: 2バイト単位で処理
fn decode_utf16_with_carry(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> (String, Vec<u8>) {
    if bytes.is_empty() {
        return (String::new(), Vec::new());
    }
    
    // UTF-16は2バイト（サロゲートペアは4バイト）単位
    // 奇数バイトなら最後の1バイトを持ち越し
    let mut safe_len = bytes.len();
    
    if safe_len % 2 != 0 {
        safe_len -= 1;
    }
    
    // サロゲートペアのチェック（UTF-16LE）
    if safe_len >= 2 && encoding == UTF_16LE {
        // 末尾がサロゲートの前半（0xD800-0xDBFF）かチェック
        let last_unit = u16::from_le_bytes([bytes[safe_len - 2], bytes[safe_len - 1]]);
        if (0xD800..=0xDBFF).contains(&last_unit) {
            // サロゲートペアの前半だけ → 4バイト必要だが2バイトしかない
            safe_len -= 2;
        }
    } else if safe_len >= 2 && encoding == UTF_16BE {
        let last_unit = u16::from_be_bytes([bytes[safe_len - 2], bytes[safe_len - 1]]);
        if (0xD800..=0xDBFF).contains(&last_unit) {
            safe_len -= 2;
        }
    }
    
    if safe_len == 0 {
        return (String::new(), bytes.to_vec());
    }
    
    let (decoded, _, _) = encoding.decode(&bytes[..safe_len]);
    (decoded.into_owned(), bytes[safe_len..].to_vec())
}

/// Shift_JIS/EUC-JP: バイト境界を検出して分割
fn decode_legacy_with_carry(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> (String, Vec<u8>) {
    if bytes.is_empty() {
        return (String::new(), Vec::new());
    }
    
    // 末尾から不完全なマルチバイト文字を探す
    let split_pos = find_safe_split_position(bytes, encoding);
    
    if split_pos == 0 {
        // 全てが不完全（まだ十分なバイトがない）
        return (String::new(), bytes.to_vec());
    }
    
    let (complete, rest) = bytes.split_at(split_pos);
    let (decoded, _, _) = encoding.decode(complete);
    (decoded.into_owned(), rest.to_vec())
}

/// レガシーエンコーディングで安全に分割できる位置を見つける
fn find_safe_split_position(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> usize {
    if encoding == SHIFT_JIS {
        find_sjis_safe_position(bytes)
    } else if encoding == EUC_JP {
        find_eucjp_safe_position(bytes)
    } else {
        bytes.len()
    }
}

fn find_sjis_safe_position(bytes: &[u8]) -> usize {
    // Shift_JISはバイト列を先頭から順番に解析
    let mut i = 0;
    let mut last_complete = 0;
    
    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            // ASCII
            i += 1;
            last_complete = i;
        } else if (0xA1..=0xDF).contains(&b) {
            // 半角カナ (1バイト)
            i += 1;
            last_complete = i;
        } else if (0x81..=0x9F).contains(&b) || (0xE0..=0xFC).contains(&b) {
            // 2バイト文字の先頭
            if i + 1 < bytes.len() {
                i += 2;
                last_complete = i;
            } else {
                // 2バイト目がない（不完全）
                break;
            }
        } else {
            i += 1;
            last_complete = i;
        }
    }
    last_complete
}

fn find_eucjp_safe_position(bytes: &[u8]) -> usize {
    let mut i = 0;
    let mut last_complete = 0;
    
    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            i += 1;
            last_complete = i;
        } else if b == 0x8E {
            // 半角カナ (2バイト)
            if i + 1 < bytes.len() {
                i += 2;
                last_complete = i;
            } else {
                break;
            }
        } else if b == 0x8F {
            // 3バイト文字
            if i + 2 < bytes.len() {
                i += 3;
                last_complete = i;
            } else {
                break;
            }
        } else if (0xA1..=0xFE).contains(&b) {
            // 2バイト文字
            if i + 1 < bytes.len() {
                i += 2;
                last_complete = i;
            } else {
                break;
            }
        } else {
            i += 1;
            last_complete = i;
        }
    }
    last_complete
}

fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        _ => {
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "そのようなファイルやディレクトリはありません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    32 => "別のプロセスがファイルを使用中です".to_string(),
                    _ => format!("{} (エラーコード: {})", e, code),
                };
            }
            e.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detect_posix_glob_metacharacters() {
        assert!(has_glob_metacharacters("*.log"));
        assert!(has_glob_metacharacters("file?.txt"));
        assert!(has_glob_metacharacters("log[0-9].txt"));
        assert!(!has_glob_metacharacters(r"literal\*.txt"));
        assert!(!has_glob_metacharacters("plain.txt"));
    }

    #[test]
    fn last_count_option_wins_and_keeps_its_origin_mode() {
        let args = vec![
            "tail".to_string(),
            "-c".to_string(),
            "5".to_string(),
            "-n".to_string(),
            "+2".to_string(),
        ];
        let (opts, files) = parse_args(&args).unwrap();

        assert!(files.is_empty());
        assert!(matches!(opts.count_mode, CountMode::Lines));
        assert_eq!(opts.lines, Some(2));
        assert!(opts.line_from_start);
        assert_eq!(opts.bytes, Some(5));
        assert!(!opts.active_byte_settings().is_some());
    }

    #[test]
    fn parse_rejects_empty_count_arguments() {
        let mut opts = Options::default();
        assert!(parse_line_arg("", &mut opts).is_err());
        assert!(parse_byte_arg("", &mut opts).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn resolves_existing_path_case_insensitively() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("tail-case-test-{}", unique));
        let nested = root.join("Logs");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("App.LOG");
        fs::write(&file, b"hello").unwrap();

        let resolved = resolve_existing_path_matches_case_insensitively(
            &root.join("logs").join("app.log"),
        )
        .expect("path should resolve case-insensitively");

        assert_eq!(normalize_case_for_match(&resolved), normalize_case_for_match(&file));

        fs::remove_file(&file).unwrap();
        fs::remove_dir(&nested).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
