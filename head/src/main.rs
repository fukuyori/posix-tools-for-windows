use std::env;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8};
use glob;

/// コマンドラインオプション
#[derive(Default)]
struct Options {
    // POSIX標準
    lines: Option<isize>,  // -n: 行数（負の値は末尾からの除外）
    bytes: Option<isize>,  // -c: バイト数（負の値は末尾からの除外）

    // GNU拡張
    quiet: bool,           // -q, --quiet: ヘッダーを非表示
    verbose: bool,         // -v, --verbose: 常にヘッダーを表示
    zero_terminated: bool, // -z, --zero-terminated: 行区切りをNULLに

    // 特殊
    show_help: bool,
    show_version: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("head: {}", e);
            eprintln!("詳細は 'head --help' を参照してください");
            std::process::exit(1);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("head 1.0.0 (Rust for Windows)");
        std::process::exit(0);
    }

    // デフォルト: 10行
    let line_count = opts.lines.unwrap_or(10);

    // ヘッダー表示判定
    let show_header = opts.verbose || (!opts.quiet && files.len() > 1);

    let exit_code = if files.is_empty() {
        // 標準入力
        match head_stdin(&opts, line_count) {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("head: {}", e);
                1
            }
        }
    } else {
        let mut has_error = false;
        let mut first = true;

        for file in &files {
            // ヘッダー表示
            if show_header {
                if !first {
                    println!();
                }
                println!("==> {} <==", file);
            }
            first = false;

            // ファイル処理
            let result = if file == "-" {
                head_stdin(&opts, line_count)
            } else {
                head_file(file, &opts, line_count)
            };

            if let Err(e) = result {
                eprintln!("head: {}: {}", file, e);
                has_error = true;
            }
        }

        if has_error { 1 } else { 0 }
    };

    std::process::exit(exit_code);
}

/// 引数解析
fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options::default();
    let mut files = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        // -- 以降は全てファイル
        if arg == "--" {
            files.extend(args[i + 1..].iter().cloned());
            break;
        }

        // ロングオプション
        if arg.starts_with("--") {
            parse_long_option(arg, &args, &mut i, &mut opts)?;
            i += 1;
            continue;
        }

        // ショートオプション
        if arg.starts_with('-') && arg.len() > 1 {
            // 数字のみの場合 (-10 など) - 旧形式
            let after_dash = &arg[1..];
            if after_dash.chars().all(|c| c.is_ascii_digit()) {
                opts.lines = Some(after_dash.parse().unwrap_or(10));
                i += 1;
                continue;
            }

            // 負の数チェック (-n -5 の -5 はファイル名ではなく値)
            parse_short_options(arg, &args, &mut i, &mut opts)?;
            i += 1;
            continue;
        }

        // ファイル引数
        files.push(arg.clone());
        i += 1;
    }

    // glob展開
    let files = expand_globs(files);

    Ok((opts, files))
}

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    
    // Windowsでは大文字小文字を区別しない
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
        
        // ワイルドカード（* または ?）を含む場合はglob展開
        if pattern.contains('*') || pattern.contains('?') {
            match glob::glob_with(&pattern, options) {
                Ok(paths) => {
                    let mut matched = false;
                    for entry in paths {
                        if let Ok(path) = entry {
                            let path: PathBuf = path;
                            if path.is_file() {
                                result.push(path.to_string_lossy().to_string());
                                matched = true;
                            }
                        }
                    }
                    if !matched {
                        // マッチなしの場合は元のパターンをそのまま（エラー表示用）
                        result.push(pattern);
                    }
                }
                Err(_) => {
                    // glob解析エラーの場合も元のパターンをそのまま
                    result.push(pattern);
                }
            }
        } else {
            result.push(pattern);
        }
    }
    
    result
}

/// ロングオプション解析
fn parse_long_option(
    arg: &str,
    args: &[String],
    i: &mut usize,
    opts: &mut Options,
) -> Result<(), String> {
    let opt = &arg[2..];

    // = 付きの値を分離
    let (name, value) = if let Some(pos) = opt.find('=') {
        (&opt[..pos], Some(&opt[pos + 1..]))
    } else {
        (opt, None)
    };

    match name {
        "lines" => {
            let val = get_option_value("--lines", value, args, i)?;
            opts.lines = Some(parse_number(&val)?);
        }
        "bytes" => {
            let val = get_option_value("--bytes", value, args, i)?;
            opts.bytes = Some(parse_size(&val)?);
        }
        "quiet" | "silent" => opts.quiet = true,
        "verbose" => opts.verbose = true,
        "zero-terminated" => opts.zero_terminated = true,
        "help" => opts.show_help = true,
        "version" => opts.show_version = true,
        _ => return Err(format!("不明なオプション: '--{}'", name)),
    }

    Ok(())
}

/// ショートオプション解析
fn parse_short_options(
    arg: &str,
    args: &[String],
    i: &mut usize,
    opts: &mut Options,
) -> Result<(), String> {
    let chars: Vec<char> = arg[1..].chars().collect();
    let mut j = 0;

    while j < chars.len() {
        match chars[j] {
            'n' => {
                let val = get_short_option_value('n', &chars, j, args, i)?;
                opts.lines = Some(parse_number(&val)?);
                return Ok(());
            }
            'c' => {
                let val = get_short_option_value('c', &chars, j, args, i)?;
                opts.bytes = Some(parse_size(&val)?);
                return Ok(());
            }
            'q' => opts.quiet = true,
            'v' => opts.verbose = true,
            'z' => opts.zero_terminated = true,
            _ => return Err(format!("不正なオプション: '-{}'", chars[j])),
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
        Err(format!("オプション '{}' には引数が必要です", opt_name))
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
        Err(format!("オプション '-{}' には引数が必要です", opt))
    }
}

/// 数値をパース（負の値も対応）
fn parse_number(s: &str) -> Result<isize, String> {
    s.parse()
        .map_err(|_| format!("不正な行数: '{}'", s))
}

/// サイズをパース（サフィックス対応、負の値も対応）
fn parse_size(s: &str) -> Result<isize, String> {
    let negative = s.starts_with('-');
    let s = if negative { &s[1..] } else { s };
    let s_upper = s.to_uppercase();

    let (num_part, multiplier): (&str, isize) = if s_upper.ends_with("KB") {
        (&s_upper[..s_upper.len() - 2], 1000)
    } else if s_upper.ends_with("MB") {
        (&s_upper[..s_upper.len() - 2], 1000 * 1000)
    } else if s_upper.ends_with("GB") {
        (&s_upper[..s_upper.len() - 2], 1000 * 1000 * 1000)
    } else if s_upper.ends_with('K') {
        (&s_upper[..s_upper.len() - 1], 1024)
    } else if s_upper.ends_with('M') {
        (&s_upper[..s_upper.len() - 1], 1024 * 1024)
    } else if s_upper.ends_with('G') {
        (&s_upper[..s_upper.len() - 1], 1024 * 1024 * 1024)
    } else if s_upper.ends_with('B') {
        (&s_upper[..s_upper.len() - 1], 1)
    } else {
        (s_upper.as_str(), 1)
    };

    let num: isize = num_part
        .parse()
        .map_err(|_| format!("不正なバイト数: '{}'", s))?;

    let result = num * multiplier;
    Ok(if negative { -result } else { result })
}

/// ヘルプを表示
fn print_help() {
    println!(
        r#"使い方: head [オプション]... [ファイル]...

各ファイルの先頭10行を標準出力に出力します。
複数ファイルの場合、各ファイルにヘッダーを付けて出力します。
ファイルが指定されない場合、標準入力から読み込みます。

UTF-8, Shift_JIS, EUC-JP を自動判定します。

オプション:
  -c, --bytes=[-]NUM    先頭NUMバイトを表示
                        NUMが'-'で始まる場合、末尾NUMバイトを除いて表示
  -n, --lines=[-]NUM    先頭NUM行を表示（デフォルト: 10）
                        NUMが'-'で始まる場合、末尾NUM行を除いて表示
  -q, --quiet, --silent ヘッダーを表示しない
  -v, --verbose         常にヘッダーを表示
  -z, --zero-terminated 行区切りを改行ではなくNULLにする
      --help            このヘルプを表示して終了
      --version         バージョン情報を表示して終了

NUMには以下のサフィックスを付けられます:
  b = 512, kB = 1000, K = 1024, MB = 1000*1000, M = 1024*1024,
  GB = 1000*1000*1000, G = 1024*1024*1024

使用例:
  head file.txt           先頭10行を表示
  head -n 20 file.txt     先頭20行を表示
  head -20 file.txt       先頭20行を表示（旧形式）
  head -n -5 file.txt     末尾5行を除いて表示
  head -c 100 file.txt    先頭100バイトを表示
  head -c 1K file.txt     先頭1024バイトを表示
  head file1.txt file2.txt  複数ファイルを表示
  head -q file1.txt file2.txt  ヘッダーなしで表示"#
    );
}

/// 標準入力を処理
fn head_stdin(opts: &Options, line_count: isize) -> io::Result<()> {
    let stdin = io::stdin();
    let mut buffer = Vec::new();
    stdin.lock().read_to_end(&mut buffer)?;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    if let Some(byte_count) = opts.bytes {
        output_bytes(&buffer, byte_count, &mut stdout)?;
    } else {
        let content = decode_to_utf8(&buffer);
        output_lines(&content, line_count, opts.zero_terminated, &mut stdout)?;
    }

    Ok(())
}

/// ファイルを処理
fn head_file(path: &str, opts: &Options, line_count: isize) -> io::Result<()> {
    let path_ref = Path::new(path);

    if path_ref.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "ディレクトリです",
        ));
    }

    let mut file = File::open(path_ref)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    if let Some(byte_count) = opts.bytes {
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        output_bytes(&buffer, byte_count, &mut stdout)?;
    } else {
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let content = decode_to_utf8(&buffer);
        output_lines(&content, line_count, opts.zero_terminated, &mut stdout)?;
    }

    Ok(())
}

/// バイト単位で出力
fn output_bytes<W: Write>(buffer: &[u8], count: isize, writer: &mut W) -> io::Result<()> {
    let len = buffer.len();

    let end = if count >= 0 {
        // 正の値: 先頭からcount バイト
        (count as usize).min(len)
    } else {
        // 負の値: 末尾から|count|バイトを除く
        let exclude = (-count) as usize;
        len.saturating_sub(exclude)
    };

    writer.write_all(&buffer[..end])?;
    Ok(())
}

/// 行単位で出力
fn output_lines<W: Write>(
    content: &str,
    count: isize,
    zero_terminated: bool,
    writer: &mut W,
) -> io::Result<()> {
    let separator = if zero_terminated { '\0' } else { '\n' };
    let lines: Vec<&str> = content.split(separator).collect();

    // 元のコンテンツがセパレータで終わっているか
    let ends_with_sep = content.ends_with(separator);

    // 実際の行数（末尾の空要素を考慮）
    let actual_line_count = if ends_with_sep && !lines.is_empty() && lines.last() == Some(&"") {
        lines.len() - 1
    } else {
        lines.len()
    };

    let output_count = if count >= 0 {
        // 正の値: 先頭からcount行
        (count as usize).min(actual_line_count)
    } else {
        // 負の値: 末尾から|count|行を除く
        let exclude = (-count) as usize;
        actual_line_count.saturating_sub(exclude)
    };

    for (i, line) in lines.iter().take(output_count).enumerate() {
        write!(writer, "{}", line)?;

        // 最後の行以外、または元データでセパレータが続いていた場合
        if i < output_count - 1 || (i < actual_line_count - 1) || (ends_with_sep && i < actual_line_count) {
            if zero_terminated {
                write!(writer, "\0")?;
            } else {
                writeln!(writer)?;
            }
        }
    }

    Ok(())
}

// ===== 文字コード関連 =====

/// 文字コードを検出
fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    // BOMチェック
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
