// wc - 行数、単語数、文字数、バイト数をカウント
// POSIX.1-2017準拠 + GNU拡張 + 独自拡張

use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8};
use glob::{MatchOptions, Pattern};
use unicode_width::UnicodeWidthChar;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
enum EncodingMode {
    #[default]
    Utf8,
    Auto,
    ShiftJis,
    EucJp,
}

#[derive(Default, Clone, Debug)]
struct Options {
    // POSIX標準オプション
    lines: bool, // -l: 行数
    words: bool, // -w: 単語数
    chars: bool, // -m: 文字数（マルチバイト対応）
    bytes: bool, // -c: バイト数

    // GNU拡張オプション
    max_line: bool,              // -L: 最長行の長さ
    files0_from: Option<String>, // --files0-from: NUL区切りのファイルリスト

    // 独自拡張オプション
    halfwidth: bool, // --halfwidth: 半角文字数
    fullwidth: bool, // --fullwidth: 全角文字数
    encoding: EncodingMode,

    show_help: bool,
    show_version: bool,
}

#[derive(Default, Clone, Debug)]
struct Counts {
    lines: usize,
    words: usize,
    chars: usize,
    bytes: usize,
    max_line: usize,
    halfwidth: usize, // 半角文字数
    fullwidth: usize, // 全角文字数
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("wc: {}", e);
            eprintln!("詳細は 'wc --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("wc (Rust版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張");
        std::process::exit(0);
    }

    // オプションが何も指定されていない場合はデフォルト（行数、単語数、バイト数）
    let opts = if !opts.lines
        && !opts.words
        && !opts.chars
        && !opts.bytes
        && !opts.max_line
        && !opts.halfwidth
        && !opts.fullwidth
    {
        Options {
            lines: true,
            words: true,
            bytes: true,
            ..opts
        }
    } else {
        opts
    };

    let mut exit_code = 0;
    let mut total = Counts::default();
    let mut file_count = 0;

    // --files0-from オプションの処理
    let files = if let Some(ref list_file) = opts.files0_from {
        match read_files0_from(list_file) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("wc: {}: {}", list_file, format_error(&e));
                std::process::exit(1);
            }
        }
    } else {
        // glob展開
        expand_globs(files)
    };

    if files.is_empty() {
        // 標準入力から読み込み
        match count_stdin(&opts) {
            Ok(counts) => {
                print_counts(&counts, &opts, None);
            }
            Err(e) => {
                eprintln!("wc: 標準入力: {}", format_error(&e));
                std::process::exit(1);
            }
        }
    } else {
        for file in &files {
            if file == "-" {
                match count_stdin(&opts) {
                    Ok(counts) => {
                        print_counts(&counts, &opts, Some("-"));
                        add_counts(&mut total, &counts);
                        file_count += 1;
                    }
                    Err(e) => {
                        eprintln!("wc: 標準入力: {}", format_error(&e));
                        exit_code = 1;
                    }
                }
            } else {
                match count_file(file, &opts) {
                    Ok(counts) => {
                        print_counts(&counts, &opts, Some(file));
                        add_counts(&mut total, &counts);
                        file_count += 1;
                    }
                    Err(e) => {
                        eprintln!("wc: '{}': {}", file, format_error(&e));
                        exit_code = 1;
                    }
                }
            }
        }

        // 複数ファイルの場合は合計を表示
        if file_count > 1 {
            print_counts(&total, &opts, Some("total"));
        }
    }

    std::process::exit(exit_code);
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options::default();
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
                "--lines" => opts.lines = true,
                "--words" => opts.words = true,
                "--chars" => opts.chars = true,
                "--bytes" => opts.bytes = true,
                "--max-line-length" => opts.max_line = true,
                "--halfwidth" | "--half" => opts.halfwidth = true,
                "--fullwidth" | "--full" => opts.fullwidth = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                s if s.starts_with("--encoding=") => {
                    opts.encoding = parse_encoding_mode(s.trim_start_matches("--encoding="))?;
                }
                s if s.starts_with("--files0-from=") => {
                    opts.files0_from = Some(s.trim_start_matches("--files0-from=").to_string());
                }
                "--encoding" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--encoding' には引数が必要です".to_string());
                    }
                    opts.encoding = parse_encoding_mode(&args[i])?;
                }
                "--files0-from" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--files0-from' には引数が必要です".to_string());
                    }
                    opts.files0_from = Some(args[i].clone());
                }
                _ => return Err(format!("不明なオプション: '{}'", arg)),
            }
            i += 1;
            continue;
        }

        // 短縮オプション
        if arg.starts_with('-') && arg.len() > 1 {
            for c in arg[1..].chars() {
                match c {
                    // POSIX標準
                    'l' => opts.lines = true,
                    'w' => opts.words = true,
                    'm' => opts.chars = true,
                    'c' => opts.bytes = true,
                    // GNU拡張
                    'L' => opts.max_line = true,
                    // 独自拡張
                    'H' => opts.halfwidth = true,
                    'F' => opts.fullwidth = true,
                    _ => return Err(format!("不正なオプション: '-{}'", c)),
                }
            }
            i += 1;
            continue;
        }

        files.push(arg.clone());
        i += 1;
    }

    if opts.files0_from.is_some() && !files.is_empty() {
        return Err("オプション '--files0-from' とファイル引数は同時に指定できません".to_string());
    }

    Ok((opts, files))
}

fn parse_encoding_mode(value: &str) -> Result<EncodingMode, String> {
    match value.to_ascii_lowercase().as_str() {
        "utf8" | "utf-8" => Ok(EncodingMode::Utf8),
        "auto" => Ok(EncodingMode::Auto),
        "sjis" | "shift_jis" | "shift-jis" | "cp932" => Ok(EncodingMode::ShiftJis),
        "eucjp" | "euc-jp" => Ok(EncodingMode::EucJp),
        _ => Err(format!(
            "不正な文字コード指定: '{}'. 使用可能: utf8, auto, sjis, eucjp",
            value
        )),
    }
}

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();

    for pattern in raw_files {
        // "-" は標準入力なのでそのまま
        if pattern == "-" {
            result.push(pattern);
            continue;
        }

        if has_glob_metacharacters(&pattern) {
            let mut matches = expand_glob_pattern(&pattern);
            if matches.is_empty() {
                // マッチしない場合はパターンをそのまま追加（エラーになる）
                result.push(pattern);
            } else {
                matches.sort_by_cached_key(|path| normalize_path_for_sort(path));
                result.extend(matches);
            }
        } else {
            result.push(pattern);
        }
    }

    result
}

fn has_glob_metacharacters(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn normalize_path_for_sort(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn expand_glob_pattern(pattern: &str) -> Vec<String> {
    let path = Path::new(pattern);
    let mut base = PathBuf::new();
    let mut segments = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => base.push(prefix.as_os_str()),
            Component::RootDir => base.push(component.as_os_str()),
            Component::CurDir => {
                if base.as_os_str().is_empty() {
                    base.push(component.as_os_str());
                }
            }
            Component::ParentDir | Component::Normal(_) => {
                segments.push(component.as_os_str().to_string_lossy().to_string());
            }
        }
    }

    expand_glob_segments(base, &segments)
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

fn expand_glob_segments(base: PathBuf, segments: &[String]) -> Vec<PathBuf> {
    if segments.is_empty() {
        return vec![base];
    }

    let head = &segments[0];
    let tail = &segments[1..];

    if !has_glob_metacharacters(head) {
        return expand_glob_segments(base.join(head), tail);
    }

    let search_dir = if base.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        base.clone()
    };

    let pattern = match Pattern::new(head) {
        Ok(pattern) => pattern,
        Err(_) => return Vec::new(),
    };

    let read_dir = match fs::read_dir(&search_dir) {
        Ok(read_dir) => read_dir,
        Err(_) => return Vec::new(),
    };

    let options = MatchOptions {
        case_sensitive: false,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };

    let mut result = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if is_special_dot_name(name) {
            continue;
        }
        if name.starts_with('.') && !head.starts_with('.') {
            continue;
        }
        if !pattern.matches_with(name, options) {
            continue;
        }

        result.extend(expand_glob_segments(base.join(name), tail));
    }

    result
}

fn is_special_dot_name(name: &str) -> bool {
    matches!(name, "." | "..")
}

/// --files0-from オプション用: NUL区切りのファイルリストを読み込む
fn read_files0_from(path: &str) -> io::Result<Vec<String>> {
    let content = if path == "-" {
        let mut buffer = Vec::new();
        io::stdin().lock().read_to_end(&mut buffer)?;
        String::from_utf8_lossy(&buffer).into_owned()
    } else {
        let mut file = File::open(path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        String::from_utf8_lossy(&buffer).into_owned()
    };

    Ok(content
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

fn print_help() {
    println!(
        r#"使い方: wc [オプション]... [ファイル]...

各ファイルの改行数、単語数、バイト数を表示します。
ファイルが指定されない場合、または - の場合は標準入力を読み込みます。
複数ファイルが指定された場合は合計も表示します。
既定では UTF-8 として処理します。

POSIX標準オプション:
  -c, --bytes           バイト数を表示
  -l, --lines           改行数を表示
  -m, --chars           文字数を表示（マルチバイト対応）
  -w, --words           単語数を表示

GNU拡張オプション:
  -L, --max-line-length 最長行の表示幅を表示
      --files0-from=F   ファイル F からNUL区切りのファイル名リストを読み込む
                        F が - の場合は標準入力から読み込む

独自拡張オプション:
  -H, --halfwidth       半角文字数を表示（印刷可能文字のみ）
  -F, --fullwidth       全角文字数を表示（印刷可能文字のみ）
      --encoding=ENC    文字系オプションの文字コードを指定
                        utf8 (既定), auto, sjis, eucjp

その他:
      --help            このヘルプを表示して終了
      --version         バージョン情報を表示して終了

オプションを指定しない場合、-l -w -c と同等（行数、単語数、バイト数）。

表示順序:
  改行数, 単語数, 文字数, バイト数, 最長行, 半角数, 全角数, ファイル名

文字幅について:
  -L オプションの表示幅は、ASCII文字を1、CJK文字（漢字、ひらがな等）を2として計算。
  -H/-F オプションは、印刷可能文字のみをカウント（改行・タブ等の制御文字は除外）。

終了ステータス:
  0  正常終了
  1  ファイルエラー
  2  オプションエラー

例:
  wc file.txt               改行数、単語数、バイト数を表示
  wc -l file.txt            改行数のみ表示
  wc -w file.txt            単語数のみ表示
  wc -m file.txt            文字数を表示（漢字対応）
  wc -c file.txt            バイト数を表示
  wc -L file.txt            最長行の表示幅を表示
  wc -HF file.txt           半角・全角文字数を表示
  wc -lwc file.txt          改行数、単語数、バイト数を表示
  wc *.txt                  複数ファイル（合計付き）
  wc file1.txt file2.txt    複数ファイル指定
  cat file | wc             パイプ入力
  wc -l < file.txt          リダイレクト入力"#
    );
}

fn count_stdin(opts: &Options) -> io::Result<Counts> {
    let stdin = io::stdin();
    let mut buffer = Vec::new();
    stdin.lock().read_to_end(&mut buffer)?;

    count_data(&buffer, opts)
}

fn count_file(path: &str, opts: &Options) -> io::Result<Counts> {
    let path = Path::new(path);

    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "ディレクトリです",
        ));
    }

    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    count_data(&buffer, opts)
}

fn count_data(bytes: &[u8], opts: &Options) -> io::Result<Counts> {
    let mut counts = Counts::default();

    // バイト数
    counts.bytes = bytes.len();

    // 改行数（POSIX: 改行バイト 0x0A の数）
    counts.lines = bytes.iter().filter(|&&b| b == b'\n').count();

    if needs_text_decoding(opts) {
        let content = decode_to_utf8(bytes, &opts.encoding)?;

        // 単語数（空白から非空白への遷移で数える）
        counts.words = count_words(&content);

        // 文字数（Unicodeコードポイント単位）
        counts.chars = content.chars().count();

        // 半角・全角文字数（印刷可能文字のみ）
        if opts.halfwidth || opts.fullwidth {
            for c in content.chars() {
                // 制御文字（改行、タブ等）はカウントしない
                if c.is_control() {
                    continue;
                }

                if is_wide_char(c) {
                    counts.fullwidth += 1;
                } else {
                    counts.halfwidth += 1;
                }
            }
        }

        // 最長行の表示幅
        if opts.max_line {
            for line in content.lines() {
                let width = display_width(line);
                if width > counts.max_line {
                    counts.max_line = width;
                }
            }
        }
    }

    Ok(counts)
}

fn needs_text_decoding(opts: &Options) -> bool {
    opts.words || opts.chars || opts.max_line || opts.halfwidth || opts.fullwidth
}

fn display_width(s: &str) -> usize {
    let mut width = 0;

    for c in s.chars() {
        width += match c {
            '\t' => 8 - (width % 8),
            _ if c.is_control() => 0,
            _ => UnicodeWidthChar::width(c).unwrap_or_else(|| if is_wide_char(c) { 2 } else { 0 }),
        };
    }

    width
}

fn count_words(s: &str) -> usize {
    let mut in_word = false;
    let mut words = 0;

    for c in s.chars() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            words += 1;
            in_word = true;
        }
    }

    words
}

/// 全角文字かどうかを判定
/// 全角: CJK漢字、ひらがな、カタカナ、全角英数字など
/// 半角: ASCII、半角カタカナなど
fn is_wide_char(c: char) -> bool {
    let cp = c as u32;

    // 制御文字は幅0として扱う（ただしカウントでは半角扱い）
    if cp < 0x20 || (0x7F..=0x9F).contains(&cp) {
        return false;
    }

    // ASCII printable
    if (0x20..=0x7E).contains(&cp) {
        return false;
    }

    // 半角カタカナ
    if (0xFF61..=0xFF9F).contains(&cp) {
        return false;
    }

    // CJK統合漢字
    if (0x4E00..=0x9FFF).contains(&cp) {
        return true;
    }
    // CJK統合漢字拡張A
    if (0x3400..=0x4DBF).contains(&cp) {
        return true;
    }
    // CJK統合漢字拡張B-G
    if (0x20000..=0x3134F).contains(&cp) {
        return true;
    }
    // ひらがな
    if (0x3040..=0x309F).contains(&cp) {
        return true;
    }
    // カタカナ
    if (0x30A0..=0x30FF).contains(&cp) {
        return true;
    }
    // 全角英数字・記号
    if (0xFF01..=0xFF60).contains(&cp) {
        return true;
    }
    // CJK記号・句読点
    if (0x3000..=0x303F).contains(&cp) {
        return true;
    }
    // 韓国語ハングル
    if (0xAC00..=0xD7AF).contains(&cp) {
        return true;
    }
    // ハングル字母
    if (0x1100..=0x11FF).contains(&cp) {
        return true;
    }
    // 囲み CJK 文字・月
    if (0x3200..=0x32FF).contains(&cp) {
        return true;
    }
    // CJK互換文字
    if (0x3300..=0x33FF).contains(&cp) {
        return true;
    }
    // CJK互換漢字
    if (0xF900..=0xFAFF).contains(&cp) {
        return true;
    }
    // 縦書き形
    if (0xFE10..=0xFE1F).contains(&cp) {
        return true;
    }
    // CJK互換形
    if (0xFE30..=0xFE4F).contains(&cp) {
        return true;
    }
    // 全角形
    if (0xFFE0..=0xFFE6).contains(&cp) {
        return true;
    }

    false
}

fn add_counts(total: &mut Counts, counts: &Counts) {
    total.lines += counts.lines;
    total.words += counts.words;
    total.chars += counts.chars;
    total.bytes += counts.bytes;
    total.halfwidth += counts.halfwidth;
    total.fullwidth += counts.fullwidth;
    if counts.max_line > total.max_line {
        total.max_line = counts.max_line;
    }
}

fn print_counts(counts: &Counts, opts: &Options, filename: Option<&str>) {
    let mut parts = Vec::new();

    // 表示順序: 改行数, 単語数, 文字数, バイト数, 最長行, 半角数, 全角数
    if opts.lines {
        parts.push(format!("{:>8}", counts.lines));
    }
    if opts.words {
        parts.push(format!("{:>8}", counts.words));
    }
    if opts.chars {
        parts.push(format!("{:>8}", counts.chars));
    }
    if opts.bytes {
        parts.push(format!("{:>8}", counts.bytes));
    }
    if opts.max_line {
        parts.push(format!("{:>8}", counts.max_line));
    }
    if opts.halfwidth {
        parts.push(format!("{:>8}", counts.halfwidth));
    }
    if opts.fullwidth {
        parts.push(format!("{:>8}", counts.fullwidth));
    }

    let output = parts.join("");

    if let Some(name) = filename {
        println!("{} {}", output, name);
    } else {
        println!("{}", output);
    }
}

// 文字コード関連
fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    // BOMチェック
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }

    // UTF-8として有効かチェック
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

fn decode_to_utf8(bytes: &[u8], mode: &EncodingMode) -> io::Result<String> {
    match mode {
        EncodingMode::Utf8 => decode_utf8_strict(bytes),
        EncodingMode::Auto => {
            if let Ok(decoded) = decode_utf8_strict(bytes) {
                return Ok(decoded);
            }

            let encoding = detect_encoding(bytes);
            decode_with_encoding_strict(bytes, encoding)
        }
        EncodingMode::ShiftJis => decode_with_encoding_strict(bytes, SHIFT_JIS),
        EncodingMode::EucJp => decode_with_encoding_strict(bytes, EUC_JP),
    }
}

fn decode_utf8_strict(bytes: &[u8]) -> io::Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "UTF-8 として正しくデコードできません",
        )
    })
}

fn decode_with_encoding_strict(
    bytes: &[u8],
    encoding: &'static encoding_rs::Encoding,
) -> io::Result<String> {
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} として正しくデコードできません", encoding.name()),
        ));
    }
    Ok(decoded.into_owned())
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn glob_expansion_is_case_insensitive_and_sorted() {
        let dir = create_temp_dir("glob-sort");

        fs::write(dir.join("b.TXT"), b"b").unwrap();
        fs::write(dir.join("A.txt"), b"a").unwrap();
        fs::write(dir.join(".hidden.txt"), b"h").unwrap();

        let pattern = format!("{}\\*.txt", dir.display());
        let expanded = expand_globs(vec![pattern]);

        assert_eq!(
            expanded,
            vec![
                dir.join("A.txt").to_string_lossy().to_string(),
                dir.join("b.TXT").to_string_lossy().to_string(),
            ]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn glob_star_does_not_cross_directory_boundaries() {
        let dir = create_temp_dir("glob-separator");
        let nested = dir.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("child.txt"), b"x").unwrap();

        let pattern = format!("{}\\*.txt", dir.display());
        let expanded = expand_globs(vec![pattern.clone()]);

        assert_eq!(expanded, vec![pattern]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dotfiles_require_explicit_dot_in_pattern() {
        let dir = create_temp_dir("glob-dot");

        fs::write(dir.join(".env"), b"x").unwrap();

        let implicit_pattern = format!("{}\\*", dir.display());
        let explicit_pattern = format!("{}\\.*", dir.display());

        assert_eq!(
            expand_globs(vec![implicit_pattern.clone()]),
            vec![implicit_pattern]
        );
        assert_eq!(
            expand_globs(vec![explicit_pattern]),
            vec![dir.join(".env").to_string_lossy().to_string()]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files0_from_conflicts_with_positional_files() {
        let args = vec![
            "wc".to_string(),
            "--files0-from".to_string(),
            "list.bin".to_string(),
            "extra.txt".to_string(),
        ];

        let err = parse_args(&args).unwrap_err();
        assert!(err.contains("--files0-from"));
    }

    #[test]
    fn dash_is_kept_as_stdin_operand() {
        let args = vec!["wc".to_string(), "-".to_string()];

        let (_opts, files) = parse_args(&args).unwrap();
        assert_eq!(files, vec!["-".to_string()]);
    }

    #[test]
    fn word_count_uses_whitespace_transitions() {
        assert_eq!(count_words(" alpha\tbeta\n\ngamma "), 3);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words(" \t\r\n "), 0);
    }

    #[test]
    fn max_line_length_expands_tabs_to_tab_stops() {
        assert_eq!(display_width("a\tb"), 9);
        assert_eq!(display_width("1234567\tZ"), 9);
        assert_eq!(display_width("ab\t漢"), 10);
    }

    #[test]
    fn max_line_length_treats_combining_marks_as_zero_width() {
        assert_eq!(display_width("e\u{0301}"), 1);
        assert_eq!(display_width("A\u{0301}\tB"), 9);
    }

    #[test]
    fn parse_encoding_option_variants() {
        assert_eq!(parse_encoding_mode("utf8").unwrap(), EncodingMode::Utf8);
        assert_eq!(parse_encoding_mode("auto").unwrap(), EncodingMode::Auto);
        assert_eq!(parse_encoding_mode("sjis").unwrap(), EncodingMode::ShiftJis);
        assert_eq!(parse_encoding_mode("euc-jp").unwrap(), EncodingMode::EucJp);
    }

    #[test]
    fn count_lines_uses_raw_bytes_even_without_valid_utf8() {
        let opts = Options {
            lines: true,
            ..Options::default()
        };
        let counts = count_data(b"\xFF\n\xFE\n", &opts).unwrap();

        assert_eq!(counts.lines, 2);
        assert_eq!(counts.bytes, 4);
    }

    #[test]
    fn utf8_mode_rejects_invalid_text_input() {
        let opts = Options {
            chars: true,
            ..Options::default()
        };
        let err = count_data(b"\xFF", &opts).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn auto_mode_decodes_shift_jis_for_text_counts() {
        let (encoded, _, _) = SHIFT_JIS.encode("漢字");
        let opts = Options {
            chars: true,
            encoding: EncodingMode::Auto,
            ..Options::default()
        };
        let counts = count_data(encoded.as_ref(), &opts).unwrap();

        assert_eq!(counts.chars, 2);
    }

    #[test]
    fn utf8_mode_counts_ascii_text() {
        let opts = Options {
            words: true,
            chars: true,
            max_line: true,
            ..Options::default()
        };
        let counts = count_data(b"alpha beta\nz", &opts).unwrap();

        assert_eq!(counts.words, 3);
        assert_eq!(counts.chars, 12);
        assert_eq!(counts.max_line, 10);
    }

    fn create_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("wc-{prefix}-{unique}"));
        fs::create_dir(&dir).unwrap();
        dir
    }
}
