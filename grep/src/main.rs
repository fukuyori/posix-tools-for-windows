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
    quiet: bool,                  // -q, -s: 出力なし（終了コードのみ）
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
    label: Option<String>,         // --label: 標準入力のラベル
    initial_tab: bool,             // -T: TABで位置揃え
    text: bool,                    // -a, --text: バイナリをテキストとして扱う
    binary_files: BinaryMode,      // --binary-files: バイナリファイルの扱い
    perl_regexp: bool,             // -P: Perl正規表現（部分対応）

    // 特殊
    show_help: bool,
    show_version: bool,
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

/// 検索結果
struct GrepResult {
    matches: usize,
    files_matched: usize,
    had_error: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let opts = match parse_args(&args) {
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
        println!("grep 1.0.0 (Rust for Windows)");
        std::process::exit(0);
    }

    // パターンを収集
    let patterns = collect_patterns(&opts);
    if patterns.is_empty() {
        eprintln!("grep: パターンを指定してください");
        std::process::exit(2);
    }

    // ファイルリストを取得
    let files = collect_files(&opts);

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
        "invert-match" => opts.invert_match = true,
        "count" => opts.count = true,
        "line-number" => opts.line_number = true,
        "files-with-matches" => opts.files_with_matches = true,
        "quiet" | "silent" => opts.quiet = true,
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
            'i' => opts.ignore_case = true,
            'v' => opts.invert_match = true,
            'c' => opts.count = true,
            'n' => opts.line_number = true,
            'l' => opts.files_with_matches = true,
            'q' => opts.quiet = true,
            's' => opts.quiet = true, // POSIX: suppress error messages
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

            // 値を取るオプション
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

/// 色モードを解析
fn parse_color_mode(s: &str) -> Result<ColorMode, String> {
    match s {
        "auto" => Ok(ColorMode::Auto),
        "always" | "yes" | "force" => Ok(ColorMode::Always),
        "never" | "no" | "none" => Ok(ColorMode::Never),
        _ => Err(format!("'--color' の値が不正です: '{}'", s)),
    }
}

/// パターンを収集
fn collect_patterns(opts: &Options) -> Vec<String> {
    let mut patterns: Vec<String> = opts
        .patterns
        .iter()
        .filter(|p| !p.starts_with("\x00FILE:"))
        .cloned()
        .collect();

    // -f オプションでパターンファイルから読み込み
    if let Some(ref path) = opts.pattern_file {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                if !line.is_empty() {
                    patterns.push(line.to_string());
                }
            }
        } else if path == "-" {
            // 標準入力からパターンを読む
            let stdin = io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                if !line.is_empty() {
                    patterns.push(line);
                }
            }
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
    } else if opts.basic_regexp && !opts.extended_regexp {
        // -G: 基本正規表現（BRE）をERE風に変換
        // BREでは +, ?, |, (, ) はリテラル、\+, \?, \|, \(, \) がメタ
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
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        } else if chars[i] == '(' || chars[i] == ')' || chars[i] == '{' || chars[i] == '}' {
            // BREではリテラル、エスケープして残す
            result.push('\\');
            result.push(chars[i]);
            i += 1;
        } else {
            result.push(chars[i]);
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
                } else {
                    eprintln!("grep: {}: ディレクトリです", file);
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

    match grep_file(path, regex, opts, show_filename, before_ctx, after_ctx) {
        Ok(count) => {
            result.matches += count;
            if count > 0 {
                result.files_matched += 1;
            }
        }
        Err(e) => {
            if !opts.quiet {
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
    let lines: Vec<&str> = if opts.null_data {
        content.split('\0').collect()
    } else {
        content.lines().collect()
    };

    grep_lines(&lines, regex, opts, label, before_ctx, after_ctx)
}

/// ファイルを検索
fn grep_file(
    path: &Path,
    regex: &Regex,
    opts: &Options,
    show_filename: bool,
    before_ctx: usize,
    after_ctx: usize,
) -> io::Result<usize> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    // バイナリファイルチェック
    if is_binary(&bytes) {
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
    let lines: Vec<&str> = if opts.null_data {
        content.split('\0').collect()
    } else {
        content.lines().collect()
    };

    let filename = if show_filename {
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    grep_lines(
        &lines,
        regex,
        opts,
        filename.as_deref(),
        before_ctx,
        after_ctx,
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
            if !opts.quiet {
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
                if !opts.quiet {
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
    regex: &Regex,
    opts: &Options,
    filename: Option<&str>,
    before_ctx: usize,
    after_ctx: usize,
) -> io::Result<usize> {
    let mut match_count = 0;
    let mut printed_lines: HashSet<usize> = HashSet::new();
    let mut pending_after: usize = 0;
    let mut last_printed_line: Option<usize> = None;
    let mut byte_offset: usize = 0;
    let mut byte_offsets: Vec<usize> = Vec::new();

    // 各行のバイトオフセットを計算
    for line in lines {
        byte_offsets.push(byte_offset);
        byte_offset += line.len() + 1; // +1 for newline
    }

    // マッチ行を収集
    let mut match_indices: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let is_match = regex.is_match(line);
        let should_match = if opts.invert_match {
            !is_match
        } else {
            is_match
        };

        if should_match {
            match_indices.push(i);
        }
    }

    // -l: マッチしたファイル名のみ
    if opts.files_with_matches {
        if !match_indices.is_empty() {
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
        if match_indices.is_empty() {
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
            match_indices.len().min(max)
        } else {
            match_indices.len()
        };

        if !opts.quiet {
            if let Some(f) = filename {
                if opts.null_output {
                    print!("{}:\0{}\n", f, count);
                } else {
                    println!("{}:{}", f, count);
                }
            } else {
                println!("{}", count);
            }
        }
        return Ok(count);
    }

    // 色を使うか判定
    let use_color = match opts.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => atty::is(atty::Stream::Stdout),
    };

    // 通常出力
    for (i, line) in lines.iter().enumerate() {
        let is_match_line = match_indices.contains(&i);

        // コンテキスト範囲内かチェック
        let in_before_context = match_indices
            .iter()
            .any(|&m| i < m && i >= m.saturating_sub(before_ctx));
        let in_after_context = if let Some(last) = last_printed_line {
            pending_after > 0 && i > last
        } else {
            false
        };

        let should_print_context = in_before_context || in_after_context;

        if is_match_line {
            if let Some(max) = opts.max_count {
                if match_count >= max {
                    break;
                }
            }

            // セパレータ
            if before_ctx > 0 || after_ctx > 0 {
                if let Some(last) = last_printed_line {
                    if i > last + 1 && !printed_lines.contains(&(i - 1)) {
                        if !opts.quiet {
                            println!("--");
                        }
                    }
                }
            }

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

    let sep = if is_match { ':' } else { '-' };
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

    // バイトオフセット
    if opts.byte_offset {
        if use_color {
            prefix.push_str(&format!("\x1b[32m{}\x1b[0m", byte_off));
        } else {
            prefix.push_str(&byte_off.to_string());
        }
        prefix.push(sep);
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

    // TAB位置揃え
    if opts.initial_tab && (opts.byte_offset || opts.line_number || filename.is_some()) {
        prefix.push('\t');
    }

    // 行内容
    if opts.only_matching && is_match {
        // -o: マッチ部分のみ
        for mat in regex.find_iter(line) {
            let matched = mat.as_str();
            if use_color {
                println!("{}\x1b[1;31m{}\x1b[0m", prefix, matched);
            } else {
                println!("{}{}", prefix, matched);
            }
        }
    } else if use_color && is_match {
        // 色付き出力
        let colored = regex.replace_all(line, "\x1b[1;31m$0\x1b[0m");
        println!("{}{}", prefix, colored);
    } else {
        println!("{}{}", prefix, line);
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
  -w, --word-regexp      単語全体としてマッチ
  -x, --line-regexp      行全体としてマッチ

出力制御:
  -c, --count            マッチした行数のみ表示
  -l, --files-with-matches    マッチしたファイル名のみ表示
  -L, --files-without-match   マッチしないファイル名のみ表示
  -m, --max-count=NUM    NUM回マッチしたら終了
  -o, --only-matching    マッチした部分のみ表示
  -q, --quiet, --silent  何も出力しない（終了コードのみ）
  -s                     エラーメッセージを抑制

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

ファイルとディレクトリの選択:
  -r, --recursive        ディレクトリを再帰的に検索
  -R, --dereference-recursive  シンボリックリンクをたどる
      --include=GLOB     GLOBにマッチするファイルのみ検索
      --exclude=GLOB     GLOBにマッチするファイルを除外
      --exclude-dir=GLOB GLOBにマッチするディレクトリを除外

その他:
  -v, --invert-match     マッチしない行を表示
  -a, --text             バイナリファイルをテキストとして扱う
      --binary-files=TYPE  バイナリファイルの扱い
                           TYPE: binary, text, without-match
      --color[=WHEN]     マッチ部分を色付け
                           WHEN: auto, always, never
      --label=LABEL      標準入力のラベル
  -z, --null-data        行区切りをNULLバイトとする
      --help             このヘルプを表示
      --version          バージョン情報を表示

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
