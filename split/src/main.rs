use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process;

use glob::{glob_with, MatchOptions};

#[derive(Debug, Clone, Copy, PartialEq)]
enum SplitMode {
    Lines,     // -l: 行数で分割
    Bytes,     // -b: バイト数で分割
    LineBytes, // -C: 最大バイト数で行を保持して分割
    Number,    // -n: N個のファイルに分割
    Chunks,    // -n l/N: 行ベースでN個に分割
}

#[derive(Debug)]
struct Config {
    /// 入力ファイル
    input_file: String,
    /// 出力プレフィックス
    prefix: String,
    /// 分割モード
    mode: SplitMode,
    /// 行数（-l）
    lines: usize,
    /// バイト数（-b, -C）
    bytes: usize,
    /// 分割数（-n）
    number: usize,
    /// サフィックス長（-a）
    suffix_length: usize,
    /// 数字サフィックス（-d）
    numeric_suffix: bool,
    /// 16進サフィックス（-x）
    hex_suffix: bool,
    /// 追加サフィックス（--additional-suffix）
    additional_suffix: String,
    /// 詳細表示（--verbose）
    verbose: bool,
    /// 空ファイルを作成しない（--elide-empty-files, -e）
    elide_empty: bool,
    /// フィルタコマンド（--filter）
    filter: Option<String>,
    /// 開始番号（--numeric-suffix）
    start_number: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            input_file: "-".to_string(),
            prefix: "x".to_string(),
            mode: SplitMode::Lines,
            lines: 1000,
            bytes: 0,
            number: 0,
            suffix_length: 2,
            numeric_suffix: false,
            hex_suffix: false,
            additional_suffix: String::new(),
            verbose: false,
            elide_empty: false,
            filter: None,
            start_number: 0,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: split [オプション]... [ファイル [プレフィックス]]
ファイルを複数の部分に分割します。

出力ファイル名は「プレフィックス + サフィックス」の形式です。
デフォルトのプレフィックスは 'x'、サフィックスは 'aa', 'ab', ... です。

分割モード:
  -l, --lines=NUMBER      NUMBER行ごとに分割（デフォルト：1000）
  -b, --bytes=SIZE        SIZEバイトごとに分割
  -C, --line-bytes=SIZE   最大SIZEバイトで行を保持して分割
  -n, --number=CHUNKS     CHUNKS個のファイルに分割
                          N      : N個のファイルに均等分割
                          l/N    : 行ベースでN個に分割
                          l/K/N  : N個中K番目のみ出力
                          r/N    : ラウンドロビンでN個に分割
                          r/K/N  : ラウンドロビンでN個中K番目のみ出力

サフィックスオプション:
  -a, --suffix-length=N   サフィックス長をNに（デフォルト：2）
  -d                      数字サフィックスを使用（00, 01, ...）
      --numeric-suffix[=FROM]  -d と同じ、開始番号を指定可能
  -x, --hex-suffix[=FROM] 16進サフィックスを使用（00, 01, ... 0f, 10, ...）
      --additional-suffix=SUFFIX  追加サフィックスを付加

その他のオプション:
  -e, --elide-empty-files 空の出力ファイルを作成しない
      --verbose           出力ファイル名を表示
      --filter=COMMAND    出力をCOMMANDに渡す（$FILE でファイル名参照）
      --help              このヘルプを表示
      --version           バージョン情報を表示

SIZEの指定:
  数字の後にサフィックスを付けることができます:
  b=512, K=1024, M=1024*1024, G=1024*1024*1024
  KB=1000, MB=1000*1000, GB=1000*1000*1000

例:
  split file.txt                   1000行ごとにxaa, xab, ...に分割
  split -l 100 file.txt            100行ごとに分割
  split -b 1M file.bin             1MBごとに分割
  split -n 5 file.txt              5個のファイルに均等分割
  split -d -a 3 file.txt part_     part_000, part_001, ...に分割
  split -b 10K -d file.bin chunk   chunk00, chunk01, ...に分割

globパターン対応:
  split *.txt                      shell 風に展開して位置引数として解釈
                                   Windows では大文字小文字を区別しません
"#
    );
}

fn print_version() {
    eprintln!("split (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

/// サイズ文字列をバイト数に変換
fn parse_size(s: &str) -> Result<usize, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("サイズが空です".to_string());
    }

    let mut num_end = 0;
    for (i, c) in s.char_indices() {
        if !c.is_ascii_digit() {
            num_end = i;
            break;
        }
        num_end = i + 1;
    }

    let num_str = &s[..num_end];
    let suffix = &s[num_end..];

    let num: usize = num_str
        .parse()
        .map_err(|_| format!("無効な数値: '{}'", num_str))?;

    let multiplier: usize = match suffix.to_uppercase().as_str() {
        "" => 1,
        "B" => 512,
        "K" | "KIB" => 1024,
        "KB" => 1000,
        "M" | "MIB" => 1024 * 1024,
        "MB" => 1000 * 1000,
        "G" | "GIB" => 1024 * 1024 * 1024,
        "GB" => 1000 * 1000 * 1000,
        _ => return Err(format!("無効なサフィックス: '{}'", suffix)),
    };

    Ok(num * multiplier)
}

fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn glob_match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    }
}

/// Windows 上でも shell の pathname expansion に近い形で glob を展開する。
/// マッチしなかった場合は POSIX shell 同様にリテラルのまま残す。
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if pattern == "-" || !is_glob_pattern(pattern) {
        return Ok(vec![pattern.to_string()]);
    }

    let mut paths: Vec<String> = glob_with(pattern, glob_match_options())
        .map_err(|e| format!("globパターンエラー: {}", e))?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if paths.is_empty() {
        return Ok(vec![pattern.to_string()]);
    }

    paths.sort_by_cached_key(|path| path.to_ascii_lowercase());
    Ok(paths)
}

fn expand_positional_args(positional: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in positional {
        expanded.extend(expand_glob(arg)?);
    }
    Ok(expanded)
}

fn parse_args_from(args: Vec<String>) -> Result<Config, String> {
    let mut config = Config::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-l" || arg == "--lines" {
            i += 1;
            if i >= args.len() {
                return Err("-l には行数が必要です".to_string());
            }
            config.mode = SplitMode::Lines;
            config.lines = args[i]
                .parse()
                .map_err(|_| format!("無効な行数: '{}'", args[i]))?;
        } else if arg.starts_with("--lines=") {
            config.mode = SplitMode::Lines;
            config.lines = arg[8..]
                .parse()
                .map_err(|_| format!("無効な行数: '{}'", &arg[8..]))?;
        } else if arg.starts_with("-l") && arg.len() > 2 {
            config.mode = SplitMode::Lines;
            config.lines = arg[2..]
                .parse()
                .map_err(|_| format!("無効な行数: '{}'", &arg[2..]))?;
        } else if arg == "-b" || arg == "--bytes" {
            i += 1;
            if i >= args.len() {
                return Err("-b にはバイト数が必要です".to_string());
            }
            config.mode = SplitMode::Bytes;
            config.bytes = parse_size(&args[i])?;
        } else if arg.starts_with("--bytes=") {
            config.mode = SplitMode::Bytes;
            config.bytes = parse_size(&arg[8..])?;
        } else if arg.starts_with("-b") && arg.len() > 2 {
            config.mode = SplitMode::Bytes;
            config.bytes = parse_size(&arg[2..])?;
        } else if arg == "-C" || arg == "--line-bytes" {
            i += 1;
            if i >= args.len() {
                return Err("-C にはバイト数が必要です".to_string());
            }
            config.mode = SplitMode::LineBytes;
            config.bytes = parse_size(&args[i])?;
        } else if arg.starts_with("--line-bytes=") {
            config.mode = SplitMode::LineBytes;
            config.bytes = parse_size(&arg[13..])?;
        } else if arg.starts_with("-C") && arg.len() > 2 {
            config.mode = SplitMode::LineBytes;
            config.bytes = parse_size(&arg[2..])?;
        } else if arg == "-n" || arg == "--number" {
            i += 1;
            if i >= args.len() {
                return Err("-n には分割数が必要です".to_string());
            }
            parse_number_arg(&args[i], &mut config)?;
        } else if arg.starts_with("--number=") {
            parse_number_arg(&arg[9..], &mut config)?;
        } else if arg.starts_with("-n") && arg.len() > 2 {
            parse_number_arg(&arg[2..], &mut config)?;
        } else if arg == "-a" || arg == "--suffix-length" {
            i += 1;
            if i >= args.len() {
                return Err("-a にはサフィックス長が必要です".to_string());
            }
            config.suffix_length = args[i]
                .parse()
                .map_err(|_| format!("無効なサフィックス長: '{}'", args[i]))?;
        } else if arg.starts_with("--suffix-length=") {
            config.suffix_length = arg[16..]
                .parse()
                .map_err(|_| format!("無効なサフィックス長: '{}'", &arg[16..]))?;
        } else if arg.starts_with("-a") && arg.len() > 2 {
            config.suffix_length = arg[2..]
                .parse()
                .map_err(|_| format!("無効なサフィックス長: '{}'", &arg[2..]))?;
        } else if arg == "-d" {
            config.numeric_suffix = true;
        } else if arg == "--numeric-suffix" {
            config.numeric_suffix = true;
        } else if arg.starts_with("--numeric-suffix=") {
            config.numeric_suffix = true;
            config.start_number = arg[17..]
                .parse()
                .map_err(|_| format!("無効な開始番号: '{}'", &arg[17..]))?;
        } else if arg == "-x" || arg == "--hex-suffix" {
            config.hex_suffix = true;
        } else if arg.starts_with("--hex-suffix=") {
            config.hex_suffix = true;
            config.start_number = usize::from_str_radix(&arg[13..], 16)
                .map_err(|_| format!("無効な開始番号: '{}'", &arg[13..]))?;
        } else if arg == "--additional-suffix" {
            i += 1;
            if i >= args.len() {
                return Err("--additional-suffix にはサフィックスが必要です".to_string());
            }
            config.additional_suffix = args[i].clone();
        } else if arg.starts_with("--additional-suffix=") {
            config.additional_suffix = arg[20..].to_string();
        } else if arg == "-e" || arg == "--elide-empty-files" {
            config.elide_empty = true;
        } else if arg == "--verbose" {
            config.verbose = true;
        } else if arg == "--filter" {
            i += 1;
            if i >= args.len() {
                return Err("--filter にはコマンドが必要です".to_string());
            }
            config.filter = Some(args[i].clone());
        } else if arg.starts_with("--filter=") {
            config.filter = Some(arg[9..].to_string());
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                positional.push(args[j].clone());
            }
            break;
        } else if arg.starts_with('-') && arg != "-" {
            if let Ok(n) = arg[1..].parse::<usize>() {
                config.mode = SplitMode::Lines;
                config.lines = n;
            } else {
                return Err(format!("不明なオプション: {}", arg));
            }
        } else {
            positional.push(arg.clone());
        }

        i += 1;
    }

    let positional = expand_positional_args(&positional)?;
    if positional.len() > 2 {
        return Err(format!("余分なオペランドがあります: '{}'", positional[2]));
    }
    if !positional.is_empty() {
        config.input_file = positional[0].clone();
    }
    if positional.len() >= 2 {
        config.prefix = positional[1].clone();
    }

    Ok(config)
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    parse_args_from(args)
}

/// -n オプションの引数をパース
fn parse_number_arg(s: &str, config: &mut Config) -> Result<(), String> {
    if s.starts_with("l/") {
        // l/N または l/K/N
        config.mode = SplitMode::Chunks;
        let parts: Vec<&str> = s[2..].split('/').collect();
        if parts.len() == 1 {
            config.number = parts[0]
                .parse()
                .map_err(|_| format!("無効な分割数: '{}'", parts[0]))?;
        } else {
            // K/N 形式は未実装
            config.number = parts[0]
                .parse()
                .map_err(|_| format!("無効な分割数: '{}'", s))?;
        }
    } else if s.starts_with("r/") {
        // ラウンドロビン（未実装）
        config.mode = SplitMode::Chunks;
        config.number = s[2..]
            .split('/')
            .next()
            .unwrap_or("1")
            .parse()
            .map_err(|_| format!("無効な分割数: '{}'", s))?;
    } else {
        config.mode = SplitMode::Number;
        config.number = s.parse().map_err(|_| format!("無効な分割数: '{}'", s))?;
    }
    Ok(())
}

/// サフィックスを生成
fn generate_suffix(n: usize, config: &Config) -> Result<String, String> {
    let num = n + config.start_number;

    if config.hex_suffix {
        let max_val = 16usize.pow(config.suffix_length as u32);
        if num >= max_val {
            return Err("出力ファイル名のサフィックスが使い果たされました".to_string());
        }
        Ok(format!(
            "{:0>width$x}{}",
            num,
            config.additional_suffix,
            width = config.suffix_length
        ))
    } else if config.numeric_suffix {
        let max_val = 10usize.pow(config.suffix_length as u32);
        if num >= max_val {
            return Err("出力ファイル名のサフィックスが使い果たされました".to_string());
        }
        Ok(format!(
            "{:0>width$}{}",
            num,
            config.additional_suffix,
            width = config.suffix_length
        ))
    } else {
        // アルファベットサフィックス (aa, ab, ..., zz, aaa, ...)
        let mut suffix = String::new();
        let mut remaining = num;

        for _ in 0..config.suffix_length {
            let c = (b'a' + (remaining % 26) as u8) as char;
            suffix.insert(0, c);
            remaining /= 26;
        }

        if remaining > 0 {
            return Err("出力ファイル名のサフィックスが使い果たされました".to_string());
        }

        suffix.push_str(&config.additional_suffix);
        Ok(suffix)
    }
}

/// 出力ファイルを開く
fn open_output(filename: &str, config: &Config) -> Result<Box<dyn Write>, String> {
    if let Some(ref filter) = config.filter {
        // フィルタコマンドを使用
        let cmd = filter.replace("$FILE", filename);
        let child = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("フィルタの起動に失敗: {}", e))?;

        Ok(Box::new(child.stdin.unwrap()))
    } else {
        // 通常のファイル出力
        if let Some(parent) = Path::new(filename).parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {}", e))?;
            }
        }

        let file = File::create(filename).map_err(|e| format!("split: '{}': {}", filename, e))?;
        Ok(Box::new(BufWriter::new(file)))
    }
}

/// 行数で分割
fn split_by_lines(config: &Config) -> Result<(), String> {
    let input: Box<dyn BufRead> = if config.input_file == "-" {
        Box::new(BufReader::new(io::stdin()))
    } else {
        let file = File::open(&config.input_file)
            .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;
        Box::new(BufReader::new(file))
    };

    let mut file_num = 0;
    let mut line_count = 0;
    let mut current_output: Option<Box<dyn Write>> = None;
    let mut current_filename = String::new();
    let mut bytes_written = 0usize;

    for line_result in input.lines() {
        let line = line_result.map_err(|e| format!("読み込みエラー: {}", e))?;

        if line_count == 0 || line_count >= config.lines {
            // 新しいファイルを開始
            if let Some(mut output) = current_output.take() {
                output
                    .flush()
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
                if config.elide_empty && bytes_written == 0 {
                    // 空ファイルを削除
                    let _ = fs::remove_file(&current_filename);
                }
            }

            let suffix = generate_suffix(file_num, config)?;
            current_filename = format!("{}{}", config.prefix, suffix);

            if config.verbose {
                eprintln!("creating file '{}'", current_filename);
            }

            current_output = Some(open_output(&current_filename, config)?);
            file_num += 1;
            line_count = 0;
            bytes_written = 0;
        }

        if let Some(ref mut output) = current_output {
            writeln!(output, "{}", line).map_err(|e| format!("書き込みエラー: {}", e))?;
            bytes_written += line.len() + 1;
        }
        line_count += 1;
    }

    if let Some(mut output) = current_output {
        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;
        if config.elide_empty && bytes_written == 0 {
            let _ = fs::remove_file(&current_filename);
        }
    }

    Ok(())
}

/// バイト数で分割
fn split_by_bytes(config: &Config) -> Result<(), String> {
    let mut input: Box<dyn Read> = if config.input_file == "-" {
        Box::new(io::stdin())
    } else {
        let file = File::open(&config.input_file)
            .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;
        Box::new(file)
    };

    let mut file_num = 0;
    let mut bytes_in_file = 0;
    let mut current_output: Option<Box<dyn Write>> = None;
    let mut current_filename = String::new();
    let mut buffer = vec![0u8; 8192.min(config.bytes)];

    loop {
        let bytes_to_read = (config.bytes - bytes_in_file).min(buffer.len());
        let bytes_read = input
            .read(&mut buffer[..bytes_to_read])
            .map_err(|e| format!("読み込みエラー: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        if current_output.is_none() {
            let suffix = generate_suffix(file_num, config)?;
            current_filename = format!("{}{}", config.prefix, suffix);

            if config.verbose {
                eprintln!("creating file '{}'", current_filename);
            }

            current_output = Some(open_output(&current_filename, config)?);
            file_num += 1;
            bytes_in_file = 0;
        }

        if let Some(ref mut output) = current_output {
            output
                .write_all(&buffer[..bytes_read])
                .map_err(|e| format!("書き込みエラー: {}", e))?;
        }
        bytes_in_file += bytes_read;

        if bytes_in_file >= config.bytes {
            if let Some(mut output) = current_output.take() {
                output
                    .flush()
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }
            bytes_in_file = 0;
        }
    }

    if let Some(mut output) = current_output {
        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;
        if config.elide_empty && bytes_in_file == 0 {
            let _ = fs::remove_file(&current_filename);
        }
    }

    Ok(())
}

/// 最大バイト数で行を保持して分割
fn split_by_line_bytes(config: &Config) -> Result<(), String> {
    let input: Box<dyn BufRead> = if config.input_file == "-" {
        Box::new(BufReader::new(io::stdin()))
    } else {
        let file = File::open(&config.input_file)
            .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;
        Box::new(BufReader::new(file))
    };

    let mut file_num = 0;
    let mut bytes_in_file = 0;
    let mut current_output: Option<Box<dyn Write>> = None;
    let mut current_filename = String::new();

    for line_result in input.lines() {
        let line = line_result.map_err(|e| format!("読み込みエラー: {}", e))?;
        let line_bytes = line.len() + 1; // +1 for newline

        // 行が最大サイズを超える場合、そのまま1行で1ファイル
        if line_bytes > config.bytes {
            if let Some(mut output) = current_output.take() {
                output
                    .flush()
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }

            let suffix = generate_suffix(file_num, config)?;
            current_filename = format!("{}{}", config.prefix, suffix);

            if config.verbose {
                eprintln!("creating file '{}'", current_filename);
            }

            let mut output = open_output(&current_filename, config)?;
            writeln!(output, "{}", line).map_err(|e| format!("書き込みエラー: {}", e))?;
            output
                .flush()
                .map_err(|e| format!("書き込みエラー: {}", e))?;

            file_num += 1;
            bytes_in_file = 0;
            continue;
        }

        // 現在のファイルに収まらない場合、新しいファイルを開始
        if bytes_in_file + line_bytes > config.bytes && bytes_in_file > 0 {
            if let Some(mut output) = current_output.take() {
                output
                    .flush()
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }
            bytes_in_file = 0;
        }

        if current_output.is_none() {
            let suffix = generate_suffix(file_num, config)?;
            current_filename = format!("{}{}", config.prefix, suffix);

            if config.verbose {
                eprintln!("creating file '{}'", current_filename);
            }

            current_output = Some(open_output(&current_filename, config)?);
            file_num += 1;
        }

        if let Some(ref mut output) = current_output {
            writeln!(output, "{}", line).map_err(|e| format!("書き込みエラー: {}", e))?;
        }
        bytes_in_file += line_bytes;
    }

    if let Some(mut output) = current_output {
        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;
        if config.elide_empty && bytes_in_file == 0 {
            let _ = fs::remove_file(&current_filename);
        }
    }

    Ok(())
}

/// N個のファイルに均等分割
fn split_by_number(config: &Config) -> Result<(), String> {
    // まずファイルサイズを取得
    let file_size = if config.input_file == "-" {
        // 標準入力の場合、一時的に全て読み込む
        return split_by_number_stdin(config);
    } else {
        let metadata = fs::metadata(&config.input_file)
            .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;
        metadata.len() as usize
    };

    let chunk_size = (file_size + config.number - 1) / config.number;

    let mut input = File::open(&config.input_file)
        .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;

    let mut buffer = vec![0u8; 8192.min(chunk_size)];

    for file_num in 0..config.number {
        let suffix = generate_suffix(file_num, config)?;
        let filename = format!("{}{}", config.prefix, suffix);

        if config.verbose {
            eprintln!("creating file '{}'", filename);
        }

        let mut output = open_output(&filename, config)?;
        let mut bytes_written = 0;

        while bytes_written < chunk_size {
            let to_read = (chunk_size - bytes_written).min(buffer.len());
            let bytes_read = input
                .read(&mut buffer[..to_read])
                .map_err(|e| format!("読み込みエラー: {}", e))?;

            if bytes_read == 0 {
                break;
            }

            output
                .write_all(&buffer[..bytes_read])
                .map_err(|e| format!("書き込みエラー: {}", e))?;
            bytes_written += bytes_read;
        }

        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;

        if config.elide_empty && bytes_written == 0 {
            let _ = fs::remove_file(&filename);
        }
    }

    Ok(())
}

/// 標準入力をN個に分割
fn split_by_number_stdin(config: &Config) -> Result<(), String> {
    // 標準入力を全て読み込む
    let mut data = Vec::new();
    io::stdin()
        .read_to_end(&mut data)
        .map_err(|e| format!("読み込みエラー: {}", e))?;

    let chunk_size = (data.len() + config.number - 1) / config.number;

    for file_num in 0..config.number {
        let start = file_num * chunk_size;
        if start >= data.len() {
            if !config.elide_empty {
                let suffix = generate_suffix(file_num, config)?;
                let filename = format!("{}{}", config.prefix, suffix);
                if config.verbose {
                    eprintln!("creating file '{}'", filename);
                }
                let mut output = open_output(&filename, config)?;
                output
                    .flush()
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }
            continue;
        }

        let end = (start + chunk_size).min(data.len());

        let suffix = generate_suffix(file_num, config)?;
        let filename = format!("{}{}", config.prefix, suffix);

        if config.verbose {
            eprintln!("creating file '{}'", filename);
        }

        let mut output = open_output(&filename, config)?;
        output
            .write_all(&data[start..end])
            .map_err(|e| format!("書き込みエラー: {}", e))?;
        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;
    }

    Ok(())
}

/// 行ベースでN個に分割
fn split_by_chunks(config: &Config) -> Result<(), String> {
    // まず行数をカウント
    let total_lines = if config.input_file == "-" {
        return split_by_chunks_stdin(config);
    } else {
        let file = File::open(&config.input_file)
            .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;
        BufReader::new(file).lines().count()
    };

    let lines_per_chunk = (total_lines + config.number - 1) / config.number;

    let file = File::open(&config.input_file)
        .map_err(|e| format!("split: '{}': {}", config.input_file, e))?;
    let reader = BufReader::new(file);

    let mut file_num = 0;
    let mut line_count = 0;
    let mut current_output: Option<Box<dyn Write>> = None;
    let mut current_filename = String::new();
    let mut bytes_written = 0;

    for line_result in reader.lines() {
        let line = line_result.map_err(|e| format!("読み込みエラー: {}", e))?;

        if line_count == 0 {
            let suffix = generate_suffix(file_num, config)?;
            current_filename = format!("{}{}", config.prefix, suffix);

            if config.verbose {
                eprintln!("creating file '{}'", current_filename);
            }

            current_output = Some(open_output(&current_filename, config)?);
            bytes_written = 0;
        }

        if let Some(ref mut output) = current_output {
            writeln!(output, "{}", line).map_err(|e| format!("書き込みエラー: {}", e))?;
            bytes_written += line.len() + 1;
        }
        line_count += 1;

        if line_count >= lines_per_chunk && file_num < config.number - 1 {
            if let Some(mut output) = current_output.take() {
                output
                    .flush()
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }
            file_num += 1;
            line_count = 0;
        }
    }

    if let Some(mut output) = current_output {
        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;
        if config.elide_empty && bytes_written == 0 {
            let _ = fs::remove_file(&current_filename);
        }
    }

    Ok(())
}

/// 標準入力を行ベースでN個に分割
fn split_by_chunks_stdin(config: &Config) -> Result<(), String> {
    // 標準入力を全て読み込む
    let mut lines = Vec::new();
    for line in BufReader::new(io::stdin()).lines() {
        lines.push(line.map_err(|e| format!("読み込みエラー: {}", e))?);
    }

    let lines_per_chunk = (lines.len() + config.number - 1) / config.number;

    for file_num in 0..config.number {
        let start = file_num * lines_per_chunk;

        let suffix = generate_suffix(file_num, config)?;
        let filename = format!("{}{}", config.prefix, suffix);

        if config.verbose {
            eprintln!("creating file '{}'", filename);
        }

        let mut output = open_output(&filename, config)?;

        if start < lines.len() {
            let end = (start + lines_per_chunk).min(lines.len());
            for line in &lines[start..end] {
                writeln!(output, "{}", line).map_err(|e| format!("書き込みエラー: {}", e))?;
            }
        } else if config.elide_empty {
            let _ = fs::remove_file(&filename);
            continue;
        }

        output
            .flush()
            .map_err(|e| format!("書き込みエラー: {}", e))?;
    }

    Ok(())
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("split: {}", e);
            eprintln!("詳しくは 'split --help' を参照してください");
            process::exit(1);
        }
    };

    let result = match config.mode {
        SplitMode::Lines => split_by_lines(&config),
        SplitMode::Bytes => split_by_bytes(&config),
        SplitMode::LineBytes => split_by_line_bytes(&config),
        SplitMode::Number => split_by_number(&config),
        SplitMode::Chunks => split_by_chunks(&config),
    };

    if let Err(e) = result {
        eprintln!("{}", e);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("split-test-{}", unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unmatched_glob_remains_literal() {
        let expanded = expand_glob("no-match-*.txt").unwrap();
        assert_eq!(expanded, vec!["no-match-*.txt"]);
    }

    #[test]
    fn glob_expansion_is_case_insensitive_and_sorted() {
        let dir = make_temp_dir();
        let original_dir = env::current_dir().unwrap();
        fs::write(dir.join("B.TXT"), "b").unwrap();
        fs::write(dir.join("a.txt"), "a").unwrap();
        env::set_current_dir(&dir).unwrap();

        let expanded = expand_glob("*.txt").unwrap();

        env::set_current_dir(original_dir).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(expanded, vec!["a.txt", "B.TXT"]);
    }

    #[test]
    fn positional_glob_matches_extra_operand_behavior() {
        let dir = make_temp_dir();
        let original_dir = env::current_dir().unwrap();
        fs::write(dir.join("first.txt"), "1").unwrap();
        fs::write(dir.join("second.txt"), "2").unwrap();
        fs::write(dir.join("third.txt"), "3").unwrap();
        env::set_current_dir(&dir).unwrap();

        let result = parse_args_from(vec!["*.txt".to_string()]);

        env::set_current_dir(original_dir).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert!(matches!(result, Err(ref e) if e.contains("余分なオペランド")));
    }
}
