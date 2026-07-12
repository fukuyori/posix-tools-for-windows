use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, Read};
use std::path::Path;

use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8};
use glob;
use regex::{Regex, RegexBuilder};

/// コマンドラインオプション
#[derive(Default)]
struct Options {
    // POSIX標準オプション
    patterns: Vec<String>,        // -e: パターン（複数可）
    pattern_file: Option<String>, // -f: パターンファイル
    ignore_case: bool,            // -i: 大文字小文字を区別しない
    invert_match: bool,           // -v: マッチしない行を表示
    count: bool,                  // -c: マッチ行数のみ表示
    line_number: bool,            // -n: 行番号を表示
    files_with_matches: bool,     // -l: マッチしたファイル名のみ
    quiet: bool,                  // -q: 出力なし（終了コードのみ）
    no_messages: bool,            // -s: ファイルエラーのメッセージを抑制
    extended_regexp: bool,        // -E: 拡張正規表現 (egrep)
    fixed_strings: bool,          // -F: 固定文字列 (fgrep)
    basic_regexp: bool,           // -G: 基本正規表現（デフォルト）
    line_regexp: bool,            // -x: 行全体でマッチ

    // GNU拡張オプション
    with_filename: bool,           // -H: ファイル名を表示
    no_filename: bool,             // -h: ファイル名を非表示
    only_matching: bool,           // -o: マッチ部分のみ表示
    files_without_match: bool,     // -L: マッチしないファイル名のみ
    recursive: bool,               // -r, -R: 再帰検索
    dereference_recursive: bool,   // -R: シンボリックリンクをたどる
    word_regexp: bool,             // -w: 単語単位でマッチ
    byte_offset: bool,             // -b: バイトオフセットを表示
    max_count: Option<usize>,      // -m: 最大マッチ数
    after_context: usize,          // -A: マッチ後の行数
    before_context: usize,         // -B: マッチ前の行数
    context: usize,                // -C: 前後の行数
    color: ColorMode,              // --color: 色付け
    null_data: bool,               // -z: NULLで行区切り
    null_output: bool,             // -Z, --null: ファイル名後にNULL
    include_patterns: Vec<String>, // --include: 含めるファイルパターン
    exclude_patterns: Vec<String>, // --exclude: 除外するファイルパターン
    exclude_dir: Vec<String>,      // --exclude-dir: 除外するディレクトリ
    exclude_from: Vec<String>,     // --exclude-from: 除外パターンのファイル
    directories: DirAction,        // -d, --directories: ディレクトリの扱い
    group_separator: Option<String>, // --group-separator (None = 出力しない)
    label: Option<String>,         // --label: 標準入力のラベル
    initial_tab: bool,             // -T: TABで位置揃え
    text: bool,                    // -a, --text: バイナリをテキストとして扱う
    binary_files: BinaryMode,      // --binary-files: バイナリファイルの扱い
    perl_regexp: bool,             // -P: Perl正規表現（部分対応）

    // 特殊
    show_help: bool,
    show_version: bool,
    /// -r でファイル省略時（暗黙の "."）は GNU 同様 "./" プレフィックスを付けない
    strip_dot_prefix: bool,
}

/// 色付けモード
#[derive(Default, Clone, Copy, PartialEq)]
enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// バイナリファイルの扱い
#[derive(Default, Clone, Copy, PartialEq)]
enum BinaryMode {
    #[default]
    Binary, // デフォルト: マッチ時にメッセージ
    Text,    // テキストとして扱う
    Without, // スキップ
}

/// ディレクトリの扱い (-d, --directories)
#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum DirAction {
    #[default]
    Read, // デフォルト: エラーを表示（POSIX の「読む」相当）
    Skip, // 黙ってスキップ
}

/// 検索結果
struct GrepResult {
    matches: usize,
    files_matched: usize,
    had_error: bool,
    /// 直前までに通常の行出力があったか（コンテキスト時のファイル間 -- 用）
    printed_output: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("grep: {}", e);
            eprintln!("詳細は 'grep --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("grep 1.1.0 (Rust for Windows)");
        std::process::exit(0);
    }

    // --exclude-from のパターンを読み込む
    for path in std::mem::take(&mut opts.exclude_from) {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                for line in content.lines() {
                    if !line.is_empty() {
                        opts.exclude_patterns.push(line.to_string());
                    }
                }
            }
            Err(e) => {
                eprintln!("grep: {}: {}", path, e);
                std::process::exit(2);
            }
        }
    }

    // パターンを収集
    let patterns = collect_patterns(&opts);
    if patterns.is_empty() {
        // -f で空のパターンファイルが指定された場合、GNU 互換で
        // 「何にもマッチしない」= 終了コード 1
        if opts.pattern_file.is_some() {
            std::process::exit(1);
        }
        eprintln!("grep: パターンを指定してください");
        std::process::exit(2);
    }

    // ファイルリストを取得
    let mut files = collect_files(&opts);

    // GNU 互換: -r でファイル省略時はカレントディレクトリを検索
    // （このとき出力パスに "./" プレフィックスを付けない）
    if files.is_empty() && opts.recursive {
        files.push(".".to_string());
        opts.strip_dot_prefix = true;
    }

    // 正規表現をビルド
    let regex = match build_regex(&patterns, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("grep: 正規表現エラー: {}", e);
            std::process::exit(2);
        }
    };

    // 検索実行
    let result = run_grep(&regex, &files, &opts);

    // 終了コード
    // 0: マッチあり, 1: マッチなし, 2: エラー
    // ただし -q はマッチがあれば、エラーがあっても 0（GNU 互換）
    if opts.quiet && result.matches > 0 {
        std::process::exit(0);
    }
    if result.had_error {
        std::process::exit(2);
    } else if result.matches > 0 {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

/// 引数を解析（残りの非オプション引数も返す）
fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options {
        color: ColorMode::Auto,
        group_separator: Some("--".to_string()),
        ..Default::default()
    };
    let mut positional: Vec<String> = Vec::new();
    let mut end_of_opts = false;
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        if end_of_opts {
            positional.push(arg.clone());
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
            parse_long_option(&arg[2..], &args, &mut i, &mut opts)?;
            i += 1;
            continue;
        }

        // ショートオプション
        if arg.starts_with('-') && arg.len() > 1 {
            parse_short_options(arg, &args, &mut i, &mut opts, &mut positional)?;
            i += 1;
            continue;
        }

        // 位置引数
        positional.push(arg.clone());
        i += 1;
    }

    // 位置引数の処理
    // パターンが指定されていなければ、最初の位置引数をパターンとする
    if opts.patterns.is_empty() && opts.pattern_file.is_none() {
        if let Some(pat) = positional.first() {
            opts.patterns.push(pat.clone());
            positional.remove(0);
        }
    }

    // 残りはファイル
    for f in positional {
        opts.patterns.push(format!("\x00FILE:{}", f)); // 特殊マーカー
    }

    Ok(opts)
}

/// ロングオプションを解析
fn parse_long_option(
    opt: &str,
    args: &[String],
    i: &mut usize,
    opts: &mut Options,
) -> Result<(), String> {
    // =付きの値を分離
    let (name, value) = if let Some(pos) = opt.find('=') {
        (&opt[..pos], Some(&opt[pos + 1..]))
    } else {
        (opt, None)
    };

    match name {
        // POSIX標準
        "regexp" => {
            let val = get_option_value("--regexp", value, args, i)?;
            opts.patterns.push(val);
        }
        "file" => {
            let val = get_option_value("--file", value, args, i)?;
            opts.pattern_file = Some(val);
        }
        "ignore-case" => opts.ignore_case = true,
        "no-ignore-case" => opts.ignore_case = false,
        "invert-match" => opts.invert_match = true,
        "count" => opts.count = true,
        "line-number" => opts.line_number = true,
        "files-with-matches" => opts.files_with_matches = true,
        "quiet" | "silent" => opts.quiet = true,
        "no-messages" => opts.no_messages = true,
        "extended-regexp" => opts.extended_regexp = true,
        "fixed-strings" => opts.fixed_strings = true,
        "basic-regexp" => opts.basic_regexp = true,
        "line-regexp" => opts.line_regexp = true,

        // GNU拡張
        "with-filename" => opts.with_filename = true,
        "no-filename" => opts.no_filename = true,
        "only-matching" => opts.only_matching = true,
        "files-without-match" => opts.files_without_match = true,
        "recursive" => opts.recursive = true,
        "dereference-recursive" => {
            opts.recursive = true;
            opts.dereference_recursive = true;
        }
        "word-regexp" => opts.word_regexp = true,
        "byte-offset" => opts.byte_offset = true,
        "max-count" => {
            let val = get_option_value("--max-count", value, args, i)?;
            opts.max_count = Some(val.parse().map_err(|_| format!("不正な数値: '{}'", val))?);
        }
        "after-context" => {
            let val = get_option_value("--after-context", value, args, i)?;
            opts.after_context = val.parse().map_err(|_| format!("不正な数値: '{}'", val))?;
        }
        "before-context" => {
            let val = get_option_value("--before-context", value, args, i)?;
            opts.before_context = val.parse().map_err(|_| format!("不正な数値: '{}'", val))?;
        }
        "context" => {
            let val = get_option_value("--context", value, args, i)?;
            opts.context = val.parse().map_err(|_| format!("不正な数値: '{}'", val))?;
        }
        "color" | "colour" => {
            let val = value.unwrap_or("always");
            opts.color = parse_color_mode(val)?;
        }
        "no-color" | "no-colour" => opts.color = ColorMode::Never,
        "null" => opts.null_output = true,
        "null-data" => opts.null_data = true,
        "include" => {
            let val = get_option_value("--include", value, args, i)?;
            opts.include_patterns.push(val);
        }
        "exclude" => {
            let val = get_option_value("--exclude", value, args, i)?;
            opts.exclude_patterns.push(val);
        }
        "exclude-dir" => {
            let val = get_option_value("--exclude-dir", value, args, i)?;
            opts.exclude_dir.push(val);
        }
        "exclude-from" => {
            let val = get_option_value("--exclude-from", value, args, i)?;
            opts.exclude_from.push(val);
        }
        "directories" => {
            let val = get_option_value("--directories", value, args, i)?;
            apply_dir_action(&val, opts)?;
        }
        "devices" => {
            let val = get_option_value("--devices", value, args, i)?;
            if val != "read" && val != "skip" {
                return Err(format!("'--devices' の値が不正です: '{}'", val));
            }
        }
        "group-separator" => {
            let val = get_option_value("--group-separator", value, args, i)?;
            opts.group_separator = Some(val);
        }
        "no-group-separator" => opts.group_separator = None,
        // 互換のため受理して無視するオプション
        "line-buffered" | "mmap" | "unix-byte-offsets" | "binary" => {}
        "label" => {
            let val = get_option_value("--label", value, args, i)?;
            opts.label = Some(val);
        }
        "initial-tab" => opts.initial_tab = true,
        "text" => {
            opts.text = true;
            opts.binary_files = BinaryMode::Text;
        }
        "binary-files" => {
            let val = get_option_value("--binary-files", value, args, i)?;
            opts.binary_files = match val.as_str() {
                "binary" => BinaryMode::Binary,
                "text" => BinaryMode::Text,
                "without-match" => BinaryMode::Without,
                _ => return Err(format!("'--binary-files' の値が不正です: '{}'", val)),
            };
        }
        "perl-regexp" => opts.perl_regexp = true,
        "help" => opts.show_help = true,
        "version" => opts.show_version = true,

        _ => return Err(format!("不明なオプション: '--{}'", name)),
    }

    Ok(())
}

/// ショートオプションを解析
fn parse_short_options(
    arg: &str,
    args: &[String],
    i: &mut usize,
    opts: &mut Options,
    _positional: &mut Vec<String>,
) -> Result<(), String> {
    let chars: Vec<char> = arg.chars().skip(1).collect();
    let mut j = 0;

    while j < chars.len() {
        let c = chars[j];

        match c {
            // POSIX標準
            'e' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                opts.patterns.push(val);
                return Ok(());
            }
            'f' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                opts.pattern_file = Some(val);
                return Ok(());
            }
            'i' | 'y' => opts.ignore_case = true, // -y は -i の旧式の同義語
            'v' => opts.invert_match = true,
            'c' => opts.count = true,
            'n' => opts.line_number = true,
            'l' => opts.files_with_matches = true,
            'q' => opts.quiet = true,
            's' => opts.no_messages = true, // POSIX: エラーメッセージのみ抑制
            'E' => opts.extended_regexp = true,
            'F' => opts.fixed_strings = true,
            'G' => opts.basic_regexp = true,
            'x' => opts.line_regexp = true,

            // GNU拡張
            'H' => opts.with_filename = true,
            'h' => opts.no_filename = true,
            'o' => opts.only_matching = true,
            'L' => opts.files_without_match = true,
            'r' => opts.recursive = true,
            'R' => {
                opts.recursive = true;
                opts.dereference_recursive = true;
            }
            'w' => opts.word_regexp = true,
            'b' => opts.byte_offset = true,
            'a' => {
                opts.text = true;
                opts.binary_files = BinaryMode::Text;
            }
            'P' => opts.perl_regexp = true,
            'z' => opts.null_data = true,
            'Z' => opts.null_output = true,
            'T' => opts.initial_tab = true,
            'I' => opts.binary_files = BinaryMode::Without,
            'U' | 'u' => {} // --binary / --unix-byte-offsets: 互換のため受理して無視
            'V' => opts.show_version = true,

            // 値を取るオプション
            'd' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                apply_dir_action(&val, opts)?;
                return Ok(());
            }
            'D' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                if val != "read" && val != "skip" {
                    return Err(format!("オプション '-D' の値が不正です: '{}'", val));
                }
                return Ok(());
            }
            'm' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                opts.max_count = Some(
                    val.parse()
                        .map_err(|_| format!("オプション '-m' の値が不正です: '{}'", val))?,
                );
                return Ok(());
            }
            'A' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                opts.after_context = val
                    .parse()
                    .map_err(|_| format!("オプション '-A' の値が不正です: '{}'", val))?;
                return Ok(());
            }
            'B' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                opts.before_context = val
                    .parse()
                    .map_err(|_| format!("オプション '-B' の値が不正です: '{}'", val))?;
                return Ok(());
            }
            'C' => {
                let val = get_short_option_value(c, &chars, j, args, i)?;
                opts.context = val
                    .parse()
                    .map_err(|_| format!("オプション '-C' の値が不正です: '{}'", val))?;
                return Ok(());
            }

            // 数字はコンテキスト行数のショートカット (-3 = -C3)
            '0'..='9' => {
                let num_str: String = chars[j..].iter().collect();
                if let Ok(n) = num_str.parse::<usize>() {
                    opts.context = n;
                    return Ok(());
                }
            }

            _ => return Err(format!("不正なオプション: '-{}'", c)),
        }

        j += 1;
    }

    Ok(())
}

/// オプションの値を取得
fn get_option_value(
    opt_name: &str,
    value: Option<&str>,
    args: &[String],
    i: &mut usize,
) -> Result<String, String> {
    if let Some(v) = value {
        Ok(v.to_string())
    } else if *i + 1 < args.len() {
        *i += 1;
        Ok(args[*i].clone())
    } else {
        Err(format!("option '{}' requires an argument", opt_name))
    }
}

/// ショートオプションの値を取得
fn get_short_option_value(
    opt: char,
    chars: &[char],
    j: usize,
    args: &[String],
    i: &mut usize,
) -> Result<String, String> {
    // 残りの文字があればそれを値として使用
    if j + 1 < chars.len() {
        return Ok(chars[j + 1..].iter().collect());
    }

    // 次の引数を使用
    if *i + 1 < args.len() {
        *i += 1;
        Ok(args[*i].clone())
    } else {
        Err(format!("option requires an argument -- '{}'", opt))
    }
}

/// -d / --directories の値を適用
fn apply_dir_action(val: &str, opts: &mut Options) -> Result<(), String> {
    match val {
        "read" => opts.directories = DirAction::Read,
        "skip" => opts.directories = DirAction::Skip,
        "recurse" => opts.recursive = true, // -d recurse は -r と同義
        _ => return Err(format!("'--directories' の値が不正です: '{}'", val)),
    }
    Ok(())
}

/// 色モードを解析
fn parse_color_mode(s: &str) -> Result<ColorMode, String> {
    match s {
        "auto" => Ok(ColorMode::Auto),
        "always" | "yes" | "force" => Ok(ColorMode::Always),
        "never" | "no" | "none" => Ok(ColorMode::Never),
        _ => Err(format!("'--color' の値が不正です: '{}'", s)),
    }
}

/// パターンを収集。
/// GNU 互換: -e / 位置引数のパターン内の改行はパターン区切り（OR）として扱い、
/// -f のパターンファイルでは空行も「全行にマッチする空パターン」として保持する。
fn collect_patterns(opts: &Options) -> Vec<String> {
    let mut patterns: Vec<String> = Vec::new();

    for p in opts
        .patterns
        .iter()
        .filter(|p| !p.starts_with("\x00FILE:"))
    {
        for part in p.split('\n') {
            patterns.push(part.to_string());
        }
    }

    // -f オプションでパターンファイルから読み込み
    if let Some(ref path) = opts.pattern_file {
        if path == "-" {
            // 標準入力からパターンを読む
            let stdin = io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                patterns.push(line);
            }
        } else if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                patterns.push(line.to_string());
            }
        } else {
            eprintln!("grep: {}: パターンファイルを読み込めません", path);
            std::process::exit(2);
        }
    }

    patterns
}

fn is_glob_pattern(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

fn glob_match_options() -> glob::MatchOptions {
    glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

fn normalize_glob_pattern(pattern: &str) -> String {
    if cfg!(windows) {
        pattern.replace('\\', "/")
    } else {
        pattern.to_string()
    }
}

fn normalize_path_for_glob(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .strip_prefix("./")
        .map(str::to_string)
        .unwrap_or(normalized)
}

/// ファイルリストを収集（内部glob展開対応）
fn collect_files(opts: &Options) -> Vec<String> {
    let raw_files: Vec<String> = opts
        .patterns
        .iter()
        .filter(|p| p.starts_with("\x00FILE:"))
        .map(|p| p.trim_start_matches("\x00FILE:").to_string())
        .collect();

    let mut result = Vec::new();

    for pattern in raw_files {
        let normalized_pattern = normalize_glob_pattern(&pattern);

        if is_glob_pattern(&normalized_pattern) {
            match glob::glob_with(&normalized_pattern, glob_match_options()) {
                Ok(paths) => {
                    let mut matched = false;
                    for entry in paths {
                        if let Ok(path) = entry {
                            // ファイルもディレクトリも帰す（-r での再帰挙動を一致）
                            result.push(path.to_string_lossy().to_string());
                            matched = true;
                        }
                    }
                    if !matched {
                        // マッチなしの場合は元のパターンをそのまま渡す（シェルと同じ挙動）
                        result.push(pattern);
                    }
                }
                Err(_) => {
                    result.push(pattern);
                }
            }
        } else {
            result.push(pattern);
        }
    }

    result
}

/// 正規表現をビルド
fn build_regex(patterns: &[String], opts: &Options) -> Result<Regex, String> {
    // パターンを結合
    let combined = if patterns.len() == 1 {
        process_pattern(&patterns[0], opts)
    } else {
        // 複数パターンは OR で結合
        let processed: Vec<String> = patterns.iter().map(|p| process_pattern(p, opts)).collect();
        format!("({})", processed.join("|"))
    };

    // 正規表現をビルド
    RegexBuilder::new(&combined)
        .case_insensitive(opts.ignore_case)
        .multi_line(true)
        .build()
        .map_err(|e| e.to_string())
}

/// パターンを処理
fn process_pattern(pattern: &str, opts: &Options) -> String {
    let mut pat = if opts.fixed_strings {
        // -F: 正規表現メタ文字をエスケープ
        regex::escape(pattern)
    } else if !opts.extended_regexp && !opts.perl_regexp {
        // デフォルト（および -G）: POSIX どおり基本正規表現（BRE）として解釈する。
        // BREでは +, ?, |, (, ), {, } はリテラル、\+, \?, \|, \(, \) 等がメタ
        convert_bre_to_ere(pattern)
    } else {
        pattern.to_string()
    };

    // -w: 単語境界
    if opts.word_regexp {
        pat = format!(r"\b{}\b", pat);
    }

    // -x: 行全体マッチ
    if opts.line_regexp {
        pat = format!("^{}$", pat);
    }

    pat
}

/// BRE (基本正規表現) を ERE 風に変換
fn convert_bre_to_ere(pattern: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '(' | ')' | '{' | '}' | '+' | '?' | '|' => {
                    // BREのエスケープメタ文字をEREのメタ文字に
                    result.push(chars[i + 1]);
                    i += 2;
                }
                '<' | '>' => {
                    // GNU 拡張 \< \> （単語境界）
                    result.push_str(r"\b");
                    i += 2;
                }
                _ => {
                    result.push(chars[i]);
                    result.push(chars[i + 1]);
                    i += 2;
                }
            }
        } else {
            match chars[i] {
                // BREではリテラル、エスケープして残す
                '(' | ')' | '{' | '}' | '+' | '?' | '|' => {
                    result.push('\\');
                    result.push(chars[i]);
                }
                _ => result.push(chars[i]),
            }
            i += 1;
        }
    }

    result
}

/// grep を実行
fn run_grep(regex: &Regex, files: &[String], opts: &Options) -> GrepResult {
    let mut result = GrepResult {
        matches: 0,
        files_matched: 0,
        had_error: false,
        printed_output: false,
    };

    // コンテキストオプションの調整
    let before_ctx = if opts.context > 0 {
        opts.context
    } else {
        opts.before_context
    };
    let after_ctx = if opts.context > 0 {
        opts.context
    } else {
        opts.after_context
    };

    if files.is_empty() {
        // 標準入力から読み込み
        // 標準入力の場合、-Hが明示的に指定されない限りファイル名は表示しない
        let label = if opts.with_filename {
            Some(opts.label.as_deref().unwrap_or("(standard input)"))
        } else {
            None
        };
        match grep_stdin(regex, opts, label, before_ctx, after_ctx) {
            Ok(count) => {
                result.matches += count;
                if count > 0 {
                    result.files_matched += 1;
                }
            }
            Err(e) => {
                eprintln!("grep: {}", e);
                result.had_error = true;
            }
        }
    } else {
        // ファイル名表示の判定
        let show_filename = if opts.no_filename {
            false
        } else if opts.with_filename {
            true
        } else {
            files.len() > 1 || opts.recursive
        };

        for file in files {
            let path = Path::new(file);

            if path.is_dir() {
                if opts.recursive {
                    grep_directory(
                        path,
                        regex,
                        opts,
                        show_filename,
                        &mut result,
                        before_ctx,
                        after_ctx,
                    );
                } else if opts.directories == DirAction::Skip {
                    // -d skip: 黙ってスキップ
                } else {
                    if !opts.no_messages {
                        eprintln!("grep: {}: ディレクトリです", file);
                    }
                    result.had_error = true;
                }
            } else {
                grep_single_file(
                    path,
                    regex,
                    opts,
                    show_filename,
                    &mut result,
                    before_ctx,
                    after_ctx,
                );
            }
        }
    }

    result
}

/// 単一ファイルを検索
fn grep_single_file(
    path: &Path,
    regex: &Regex,
    opts: &Options,
    show_filename: bool,
    result: &mut GrepResult,
    before_ctx: usize,
    after_ctx: usize,
) {
    // ファイルパターンフィルタ
    if !should_process_file(path, opts) {
        return;
    }

    // コンテキスト表示時、直前のファイルの出力との間にグループ区切りを入れる
    let context_active = before_ctx > 0 || after_ctx > 0;
    let normal_output = !opts.quiet
        && !opts.count
        && !opts.files_with_matches
        && !opts.files_without_match;
    let file_sep_pending = context_active && normal_output && result.printed_output;

    match grep_file(
        path,
        regex,
        opts,
        show_filename,
        before_ctx,
        after_ctx,
        file_sep_pending,
    ) {
        Ok(count) => {
            result.matches += count;
            if count > 0 {
                result.files_matched += 1;
                if normal_output {
                    result.printed_output = true;
                }
            }
        }
        Err(e) => {
            if !opts.no_messages {
                eprintln!("grep: {}: {}", path.display(), e);
            }
            result.had_error = true;
        }
    }
}

/// ファイルを処理すべきかチェック
fn should_process_file(path: &Path, opts: &Options) -> bool {
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let normalized_path = normalize_path_for_glob(path);

    // --include パターン
    if !opts.include_patterns.is_empty() {
        let matches = opts
            .include_patterns
            .iter()
            .any(|pat| glob_match_path(pat, filename, &normalized_path));
        if !matches {
            return false;
        }
    }

    // --exclude パターン
    if opts
        .exclude_patterns
        .iter()
        .any(|pat| glob_match_path(pat, filename, &normalized_path))
    {
        return false;
    }

    true
}

/// ディレクトリを処理すべきかチェック
fn should_process_dir(path: &Path, opts: &Options) -> bool {
    let dirname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let normalized_path = normalize_path_for_glob(path);

    // --exclude-dir パターン
    !opts
        .exclude_dir
        .iter()
        .any(|pat| glob_match_path(pat, dirname, &normalized_path))
}

/// 簡易globマッチ
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = normalize_glob_pattern(pattern);
    let text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.to_string()
    };

    match glob::Pattern::new(&pattern) {
        Ok(g) => g.matches_with(&text, glob_match_options()),
        Err(_) => false,
    }
}

fn glob_match_path(pattern: &str, basename: &str, normalized_path: &str) -> bool {
    let normalized_pattern = normalize_glob_pattern(pattern);
    let targets = if normalized_pattern.contains('/') {
        [normalized_path, basename]
    } else {
        [basename, basename]
    };

    targets
        .iter()
        .any(|candidate| glob_match(&normalized_pattern, candidate))
}

/// 行分割と各行の（デコード後の）バイトオフセットを計算する。
/// 通常は改行区切り（CRLF の \r も除去）、-z では NUL 区切り。
fn split_lines(content: &str, null_data: bool) -> (Vec<&str>, Vec<usize>) {
    let sep = if null_data { '\0' } else { '\n' };
    let mut lines = Vec::new();
    let mut offsets = Vec::new();
    let mut pos = 0usize;

    for chunk in content.split_inclusive(sep) {
        offsets.push(pos);
        pos += chunk.len();
        let mut line = chunk.strip_suffix(sep).unwrap_or(chunk);
        if !null_data {
            line = line.strip_suffix('\r').unwrap_or(line);
        }
        lines.push(line);
    }

    (lines, offsets)
}

/// 標準入力を検索
fn grep_stdin(
    regex: &Regex,
    opts: &Options,
    label: Option<&str>,
    before_ctx: usize,
    after_ctx: usize,
) -> io::Result<usize> {
    let stdin = io::stdin();
    let mut bytes = Vec::new();
    stdin.lock().read_to_end(&mut bytes)?;

    let content = decode_to_utf8(&bytes);
    let (lines, offsets) = split_lines(&content, opts.null_data);

    grep_lines(
        &lines, &offsets, regex, opts, label, before_ctx, after_ctx, false,
    )
}

/// ファイルを検索
fn grep_file(
    path: &Path,
    regex: &Regex,
    opts: &Options,
    show_filename: bool,
    before_ctx: usize,
    after_ctx: usize,
    file_sep_pending: bool,
) -> io::Result<usize> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    // バイナリファイルチェック（-z では NUL は区切り文字なので判定しない）
    if !opts.null_data && is_binary(&bytes) {
        match opts.binary_files {
            BinaryMode::Without => return Ok(0),
            BinaryMode::Binary => {
                // マッチがあればメッセージを表示
                let content = decode_to_utf8(&bytes);
                if regex.is_match(&content) {
                    if !opts.quiet {
                        println!("Binary file {} matches", path.display());
                    }
                    return Ok(1);
                }
                return Ok(0);
            }
            BinaryMode::Text => {
                // そのまま処理
            }
        }
    }

    let content = decode_to_utf8(&bytes);
    let (lines, offsets) = split_lines(&content, opts.null_data);

    let filename = if show_filename {
        let mut name = path.to_string_lossy().to_string();
        // 再帰検索では GNU 同様 '/' 区切りで表示する
        if opts.recursive {
            name = name.replace('\\', "/");
        }
        if opts.strip_dot_prefix {
            if let Some(stripped) = name.strip_prefix("./") {
                name = stripped.to_string();
            }
        }
        Some(name)
    } else {
        None
    };

    grep_lines(
        &lines,
        &offsets,
        regex,
        opts,
        filename.as_deref(),
        before_ctx,
        after_ctx,
        file_sep_pending,
    )
}

/// ディレクトリを再帰検索
fn grep_directory(
    dir: &Path,
    regex: &Regex,
    opts: &Options,
    show_filename: bool,
    result: &mut GrepResult,
    before_ctx: usize,
    after_ctx: usize,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            if !opts.no_messages {
                eprintln!("grep: {}: {}", dir.display(), e);
            }
            result.had_error = true;
            return;
        }
    };

    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                if !opts.no_messages {
                    eprintln!(
                        "grep: '{}' 内のエントリを読み取れません: {}",
                        dir.display(),
                        e
                    );
                }
                result.had_error = true;
                continue;
            }
        };
        let path = entry.path();

        // GNU 互換: -r は走査中のシンボリックリンクを辿らない（-R は辿る）
        if !opts.dereference_recursive {
            if let Ok(ft) = entry.file_type() {
                if ft.is_symlink() {
                    continue;
                }
            }
        }

        if path.is_dir() {
            if should_process_dir(&path, opts) {
                grep_directory(
                    &path,
                    regex,
                    opts,
                    show_filename,
                    result,
                    before_ctx,
                    after_ctx,
                );
            }
        } else if path.is_file() {
            grep_single_file(
                &path,
                regex,
                opts,
                show_filename,
                result,
                before_ctx,
                after_ctx,
            );
        }
    }
}

/// 行を検索
fn grep_lines(
    lines: &[&str],
    byte_offsets: &[usize],
    regex: &Regex,
    opts: &Options,
    filename: Option<&str>,
    before_ctx: usize,
    after_ctx: usize,
    file_sep_pending: bool,
) -> io::Result<usize> {
    let mut match_count = 0;
    let mut printed_lines: HashSet<usize> = HashSet::new();
    let mut pending_after: usize = 0;
    let mut last_printed_line: Option<usize> = None;

    // マッチ行を収集
    let mut is_match_line: Vec<bool> = Vec::with_capacity(lines.len());
    let mut total_matches = 0usize;
    for line in lines.iter() {
        let is_match = regex.is_match(line);
        let should_match = if opts.invert_match {
            !is_match
        } else {
            is_match
        };
        if should_match {
            total_matches += 1;
        }
        is_match_line.push(should_match);
    }

    // -l: マッチしたファイル名のみ
    if opts.files_with_matches {
        if total_matches > 0 {
            if let Some(f) = filename {
                if !opts.quiet {
                    if opts.null_output {
                        print!("{}\0", f);
                    } else {
                        println!("{}", f);
                    }
                }
            }
            return Ok(1);
        }
        return Ok(0);
    }

    // -L: マッチしないファイル名のみ
    if opts.files_without_match {
        if total_matches == 0 {
            if let Some(f) = filename {
                if !opts.quiet {
                    if opts.null_output {
                        print!("{}\0", f);
                    } else {
                        println!("{}", f);
                    }
                }
            }
            return Ok(1);
        }
        return Ok(0);
    }

    // -c: カウントのみ
    if opts.count {
        let count = if let Some(max) = opts.max_count {
            total_matches.min(max)
        } else {
            total_matches
        };

        if !opts.quiet {
            if let Some(f) = filename {
                if opts.null_output {
                    // -Z: ファイル名の直後の ':' の代わりに NUL
                    print!("{}\0{}\n", f, count);
                } else {
                    println!("{}:{}", f, count);
                }
            } else {
                println!("{}", count);
            }
        }
        return Ok(count);
    }

    // 各行から次のマッチ行までの距離（before-context 判定用、O(n) で前計算）
    let mut next_match_dist: Vec<usize> = vec![usize::MAX; lines.len()];
    {
        let mut dist = usize::MAX;
        for i in (0..lines.len()).rev() {
            if is_match_line[i] {
                dist = 0;
            } else if dist != usize::MAX {
                dist = dist.saturating_add(1);
            }
            next_match_dist[i] = dist;
        }
    }

    // 色を使うか判定
    let use_color = match opts.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => atty::is(atty::Stream::Stdout),
    };

    // 通常出力
    for (i, line) in lines.iter().enumerate() {
        let is_match = is_match_line[i];

        // コンテキスト範囲内かチェック
        let in_before_context =
            !is_match && next_match_dist[i] != usize::MAX && next_match_dist[i] <= before_ctx;
        let in_after_context = if let Some(last) = last_printed_line {
            pending_after > 0 && i > last
        } else {
            false
        };

        let should_print_context = in_before_context || in_after_context;

        // -m: 上限到達で読み取りを打ち切る
        if is_match {
            if let Some(max) = opts.max_count {
                if match_count >= max {
                    break;
                }
            }
        }

        // グループセパレータ: 出力が途切れた後に次のグループ（マッチ or コンテキスト）
        // を印字する直前に出す。直前のファイルの出力との間にも出す。
        let will_print = (is_match || should_print_context) && !printed_lines.contains(&i);
        if will_print && (before_ctx > 0 || after_ctx > 0) {
            let gap = match last_printed_line {
                Some(last) => i > last + 1,
                None => file_sep_pending,
            };
            if gap && !opts.quiet {
                if let Some(ref sep) = opts.group_separator {
                    println!("{}", sep);
                }
            }
        }

        if is_match {
            if !printed_lines.contains(&i) {
                print_line(
                    i,
                    byte_offsets.get(i).copied().unwrap_or(0),
                    line,
                    regex,
                    opts,
                    filename,
                    true,
                    use_color,
                );
                printed_lines.insert(i);
                last_printed_line = Some(i);
            }

            match_count += 1;
            pending_after = after_ctx;
        } else if should_print_context {
            if !printed_lines.contains(&i) {
                print_line(
                    i,
                    byte_offsets.get(i).copied().unwrap_or(0),
                    line,
                    regex,
                    opts,
                    filename,
                    false,
                    use_color,
                );
                printed_lines.insert(i);
                last_printed_line = Some(i);
            }
            if in_after_context {
                pending_after = pending_after.saturating_sub(1);
            }
        } else {
            pending_after = 0;
        }
    }

    Ok(match_count)
}

/// 行を出力
fn print_line(
    line_num: usize,
    byte_off: usize,
    line: &str,
    regex: &Regex,
    opts: &Options,
    filename: Option<&str>,
    is_match: bool,
    use_color: bool,
) {
    if opts.quiet {
        return;
    }

    // -z では出力レコードも NUL 終端（GNU 互換）
    let eol = if opts.null_data { '\0' } else { '\n' };
    let sep = if is_match { ':' } else { '-' };

    // マッチ単位のオフセットを使う -o -b のため、prefix を都度組み立てる
    let build_prefix = |byte_off: usize| -> String {
        let mut prefix = String::new();

        // ファイル名
        if let Some(f) = filename {
            if use_color {
                prefix.push_str(&format!("\x1b[35m{}\x1b[0m", f));
            } else {
                prefix.push_str(f);
            }
            if opts.null_output {
                prefix.push('\0');
            } else {
                prefix.push(sep);
            }
        }

        // 行番号
        if opts.line_number {
            if use_color {
                prefix.push_str(&format!("\x1b[32m{}\x1b[0m", line_num + 1));
            } else {
                prefix.push_str(&(line_num + 1).to_string());
            }
            prefix.push(sep);
        }

        // バイトオフセット
        if opts.byte_offset {
            if use_color {
                prefix.push_str(&format!("\x1b[32m{}\x1b[0m", byte_off));
            } else {
                prefix.push_str(&byte_off.to_string());
            }
            prefix.push(sep);
        }

        // TAB位置揃え
        if opts.initial_tab && (opts.byte_offset || opts.line_number || filename.is_some()) {
            prefix.push('\t');
        }

        prefix
    };

    // 行内容
    if opts.only_matching && is_match {
        // -o: マッチ部分のみ（-b はマッチ位置のオフセット）
        for mat in regex.find_iter(line) {
            let prefix = build_prefix(byte_off + mat.start());
            let matched = mat.as_str();
            if use_color {
                print!("{}\x1b[1;31m{}\x1b[0m{}", prefix, matched, eol);
            } else {
                print!("{}{}{}", prefix, matched, eol);
            }
        }
    } else if use_color && is_match {
        // 色付き出力
        let prefix = build_prefix(byte_off);
        let colored = regex.replace_all(line, "\x1b[1;31m$0\x1b[0m");
        print!("{}{}{}", prefix, colored, eol);
    } else {
        let prefix = build_prefix(byte_off);
        print!("{}{}{}", prefix, line, eol);
    }
}

/// バイナリファイルか判定
fn is_binary(bytes: &[u8]) -> bool {
    let check_len = bytes.len().min(8192);
    bytes[..check_len].iter().any(|&b| b == 0)
}

/// 文字コードを検出
fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    // BOM チェック
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }

    // UTF-8として有効かチェック
    if std::str::from_utf8(bytes).is_ok() {
        return UTF_8;
    }

    // Shift_JIS と EUC-JP のスコアを比較
    let sjis_score = calc_sjis_score(bytes);
    let eucjp_score = calc_eucjp_score(bytes);

    if sjis_score > eucjp_score {
        SHIFT_JIS
    } else if eucjp_score > sjis_score {
        EUC_JP
    } else {
        SHIFT_JIS // デフォルト
    }
}

/// Shift_JIS スコア計算
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

/// EUC-JP スコア計算
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

/// UTF-8にデコード
fn decode_to_utf8(bytes: &[u8]) -> String {
    let encoding = detect_encoding(bytes);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_is_glob_pattern() {
        assert!(is_glob_pattern("*.txt"));
        assert!(is_glob_pattern("file?.log"));
        assert!(is_glob_pattern("[abc].txt"));
        assert!(!is_glob_pattern("README.md"));
    }

    #[test]
    fn test_glob_match() {
        assert!(!glob_match("*.TXT", "foo.txt"));
        assert!(glob_match("f*o.txt", "foo.txt"));
        assert!(glob_match("?.txt", "a.txt"));
    }

    #[test]
    fn test_collect_files_glob_expansion() -> io::Result<()> {
        let base = env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = base.join(format!("grep_test_{unique}"));

        fs::create_dir_all(&dir)?;
        fs::write(dir.join("a.txt"), "hello")?;
        fs::write(dir.join("b.txt"), "world")?;
        fs::create_dir_all(dir.join("subdir"))?;

        let pattern = format!("{}{}*.txt", dir.display(), std::path::MAIN_SEPARATOR);
        let opts = Options {
            patterns: vec![format!("\x00FILE:{}", pattern)],
            ..Default::default()
        };

        let files = collect_files(&opts);

        assert!(files.iter().any(|p| p.ends_with("a.txt")));
        assert!(files.iter().any(|p| p.ends_with("b.txt")));

        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn test_collect_files_glob_expansion_with_windows_separator() -> io::Result<()> {
        let base = env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = base.join(format!("grep_test_sep_{unique}"));

        fs::create_dir_all(dir.join("nested"))?;
        fs::write(dir.join("nested").join("a.txt"), "hello")?;

        let raw_pattern = if cfg!(windows) {
            format!(r"{}\nested\*.txt", dir.display())
        } else {
            format!("{}/nested/*.txt", dir.display())
        };
        let opts = Options {
            patterns: vec![format!("\x00FILE:{}", raw_pattern)],
            ..Default::default()
        };

        let files = collect_files(&opts);

        assert!(files.iter().any(|p| p.ends_with("a.txt")));

        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn test_glob_match_path_supports_path_patterns() {
        let path = Path::new("./src/main.rs");
        let normalized_path = normalize_path_for_glob(path);

        assert!(glob_match_path("src/*.rs", "main.rs", &normalized_path));
        assert!(!glob_match_path("src/*.txt", "main.rs", &normalized_path));
    }

    #[test]
    fn test_bre_is_default_syntax() {
        // デフォルト（BRE）では + ? | ( ) { } はリテラル
        let opts = Options::default();
        let pat = process_pattern("a+b?c|d(e){f}", &opts);
        let re = Regex::new(&pat).unwrap();
        assert!(re.is_match("a+b?c|d(e){f}"));
        assert!(!re.is_match("aab"));

        // \+ \| などはメタ文字に昇格
        let pat = process_pattern(r"a\+", &opts);
        let re = Regex::new(&pat).unwrap();
        assert!(re.is_match("aaa"));
    }

    #[test]
    fn test_bre_word_boundary_extension() {
        let opts = Options::default();
        let pat = process_pattern(r"\<word\>", &opts);
        let re = Regex::new(&pat).unwrap();
        assert!(re.is_match("a word here"));
        assert!(!re.is_match("sword"));
    }

    #[test]
    fn test_ere_syntax_with_extended_flag() {
        let opts = Options {
            extended_regexp: true,
            ..Default::default()
        };
        let pat = process_pattern("a+(b|c)", &opts);
        let re = Regex::new(&pat).unwrap();
        assert!(re.is_match("aab"));
        assert!(re.is_match("ac"));
    }

    #[test]
    fn test_convert_bre_escaped_backslash() {
        // \\ はリテラルのバックスラッシュのまま（後続の ( を誤って昇格しない）
        assert_eq!(convert_bre_to_ere(r"a\\(b"), r"a\\\(b");
    }

    #[test]
    fn test_split_lines_strips_crlf_and_tracks_offsets() {
        let content = "abc\r\ndef\nghi";
        let (lines, offsets) = split_lines(content, false);
        assert_eq!(lines, vec!["abc", "def", "ghi"]);
        assert_eq!(offsets, vec![0, 5, 9]);
    }

    #[test]
    fn test_split_lines_null_data() {
        let content = "abc\0def\0";
        let (lines, offsets) = split_lines(content, true);
        assert_eq!(lines, vec!["abc", "def"]);
        assert_eq!(offsets, vec![0, 4]);
    }

    #[test]
    fn test_parse_args_s_is_not_quiet() {
        let args: Vec<String> = ["grep", "-s", "pat"].iter().map(|s| s.to_string()).collect();
        let opts = parse_args(&args).unwrap();
        assert!(opts.no_messages);
        assert!(!opts.quiet);
    }

    #[test]
    fn test_parse_args_directories_skip() {
        let args: Vec<String> = ["grep", "-d", "skip", "pat"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).unwrap();
        assert_eq!(opts.directories, DirAction::Skip);

        let args: Vec<String> = ["grep", "--directories=recurse", "pat"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&args).unwrap();
        assert!(opts.recursive);
    }

    #[test]
    fn test_patterns_split_on_newline() {
        let opts = Options {
            patterns: vec!["foo\nbar".to_string()],
            ..Default::default()
        };
        let patterns = collect_patterns(&opts);
        assert_eq!(patterns, vec!["foo".to_string(), "bar".to_string()]);
    }
}

/// ヘルプを表示
fn print_help() {
    println!(
        r#"使い方: grep [オプション] パターン [ファイル...]
       grep [オプション] -e パターン [-e パターン...] [ファイル...]
       grep [オプション] -f ファイル [ファイル...]

パターンにマッチする行を検索します。
UTF-8, Shift_JIS, EUC-JP を自動判定します。

パターン構文の選択:
  -E, --extended-regexp  拡張正規表現（ERE）を使用
  -F, --fixed-strings    固定文字列として検索（正規表現無効）
  -G, --basic-regexp     基本正規表現（BRE）を使用（デフォルト）
  -P, --perl-regexp      Perl互換正規表現を使用

パターン選択:
  -e, --regexp=パターン  検索パターンを指定（複数指定可）
  -f, --file=ファイル    パターンをファイルから読み込み
  -i, --ignore-case      大文字小文字を区別しない
      --no-ignore-case   大文字小文字を区別する（デフォルト）
  -w, --word-regexp      単語全体としてマッチ
  -x, --line-regexp      行全体としてマッチ

出力制御:
  -c, --count            マッチした行数のみ表示
  -l, --files-with-matches    マッチしたファイル名のみ表示
  -L, --files-without-match   マッチしないファイル名のみ表示
  -m, --max-count=NUM    NUM回マッチしたら終了
  -o, --only-matching    マッチした部分のみ表示
  -q, --quiet, --silent  何も出力しない（終了コードのみ）
  -s, --no-messages      ファイルのエラーメッセージを抑制

出力行の接頭辞制御:
  -b, --byte-offset      各行のバイトオフセットを表示
  -H, --with-filename    各行にファイル名を表示
  -h, --no-filename      ファイル名を表示しない
  -n, --line-number      各行に行番号を表示
  -T, --initial-tab      TABで位置を揃える
  -Z, --null             ファイル名の後にNULLバイトを出力

コンテキスト制御:
  -A, --after-context=NUM     マッチ後のNUM行を表示
  -B, --before-context=NUM    マッチ前のNUM行を表示
  -C, --context=NUM           前後のNUM行を表示
  -NUM                        -C NUM と同じ
      --group-separator=SEP   コンテキストのグループ区切り（デフォルト: --）
      --no-group-separator    グループ区切りを出力しない

ファイルとディレクトリの選択:
  -r, --recursive        ディレクトリを再帰的に検索（ファイル省略時は .）
                         走査中のシンボリックリンクは辿らない
  -R, --dereference-recursive  再帰検索でシンボリックリンクをたどる
  -d, --directories=ACTION  ディレクトリの扱い (read, skip, recurse)
  -D, --devices=ACTION      デバイスファイルの扱い (read, skip)
      --include=GLOB     GLOBにマッチするファイルのみ検索
      --exclude=GLOB     GLOBにマッチするファイルを除外
      --exclude-from=FILE FILEから除外パターンを読み込み
      --exclude-dir=GLOB GLOBにマッチするディレクトリを除外

その他:
  -v, --invert-match     マッチしない行を表示
  -a, --text             バイナリファイルをテキストとして扱う
  -I                     バイナリファイルをスキップ（--binary-files=without-match）
      --binary-files=TYPE  バイナリファイルの扱い
                           TYPE: binary, text, without-match
      --color[=WHEN]     マッチ部分を色付け
                           WHEN: auto, always, never
      --label=LABEL      標準入力のラベル
  -z, --null-data        行区切りをNULLバイトとする（出力もNUL終端）
      --line-buffered, -U, -u, --mmap  互換のため受理（無視）
      --help             このヘルプを表示
  -V, --version          バージョン情報を表示

終了コード:
  0  マッチが見つかった
  1  マッチが見つからなかった
  2  エラーが発生した

使用例:
  grep 'hello' file.txt              helloを含む行を表示
  grep -i 'HELLO' file.txt           大文字小文字を無視
  grep -n 'pattern' *.txt            行番号付きで表示
  grep -r 'TODO' .                   カレントディレクトリを再帰検索
  grep -E 'foo|bar' file.txt         fooまたはbarにマッチ
  grep -v 'comment' file.txt         commentを含まない行
  grep -c 'error' *.log              各ファイルのマッチ数
  grep -l 'main' *.c                 マッチするファイル名のみ
  grep -A2 -B2 'error' log.txt       前後2行のコンテキスト
  grep --include='*.c' -r 'func' .   .cファイルのみ検索"#
    );
}
