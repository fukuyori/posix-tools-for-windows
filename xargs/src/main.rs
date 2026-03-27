use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use glob::{glob_with, MatchOptions};

#[derive(Debug)]
struct Config {
    /// 実行するコマンド
    command: String,
    /// コマンドの初期引数
    initial_args: Vec<String>,
    /// 区切り文字（-d）
    delimiter: Option<char>,
    /// 最大引数数（-n）
    max_args: Option<usize>,
    /// 最大コマンドライン長（-s）
    max_chars: usize,
    /// プレースホルダ（-I）
    placeholder: Option<String>,
    /// 詳細表示（-t）
    verbose: bool,
    /// 確認モード（-p）
    prompt: bool,
    /// 入力が空でもコマンド実行（-r の逆、デフォルトはtrue）
    run_if_empty: bool,
    /// 終了文字列（-E）
    eof_str: Option<String>,
    /// 並列実行数（-P）
    max_procs: usize,
    /// NUL区切り（-0）
    null_separator: bool,
    /// 行ごとに実行（-L）
    max_lines: Option<usize>,
    /// 終了時にエラーがあっても継続しない
    exit_on_error: bool,
    /// 入力ファイル（-a）
    input_file: Option<String>,
    /// 対話的確認でデフォルトをnoに（GNU拡張）
    interactive: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            command: "echo".to_string(),
            initial_args: Vec::new(),
            delimiter: None,
            max_args: None,
            max_chars: 128 * 1024, // 128KB default
            placeholder: None,
            verbose: false,
            prompt: false,
            run_if_empty: true,
            eof_str: None,
            max_procs: 1,
            null_separator: false,
            max_lines: None,
            exit_on_error: false,
            input_file: None,
            interactive: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: xargs [オプション]... [コマンド [初期引数]...]
標準入力から引数を読み込み、コマンドを実行します。

オプション:
  -0, --null              入力の区切りをNUL文字に（find -print0と併用）
  -a, --arg-file=FILE     標準入力の代わりにFILEから引数を読み込む
  -d, --delimiter=CHAR    入力の区切り文字を指定（デフォルト：空白/改行）
  -E END                  END を入力終了文字列として扱う
  -I REPLACE              REPLACE を引数のプレースホルダとして使用
                          -I は暗黙的に -L 1 を設定
  -L MAX-LINES            1回のコマンド実行に最大MAX-LINES行を使用
  -n, --max-args=MAX      1回のコマンドに最大MAX個の引数を渡す
  -P, --max-procs=MAX     最大MAX個のプロセスを同時に実行（デフォルト：1）
  -p, --interactive       各コマンド実行前に確認（y/nで応答）
  -r, --no-run-if-empty   入力が空の場合はコマンドを実行しない
  -s, --max-chars=MAX     コマンドラインの最大長をMAX文字に制限
  -t, --verbose           実行するコマンドを標準エラーに表示
  -x, --exit              コマンドライン長を超えたら終了
      --help              このヘルプを表示
      --version           バージョン情報を表示

コマンドが指定されない場合、/bin/echo が使用されます。

例:
  find . -name "*.txt" | xargs cat
      すべての.txtファイルの内容を表示

  find . -name "*.txt" -print0 | xargs -0 cat
      ファイル名にスペースが含まれていても正しく処理

  find . -name "*.bak" | xargs rm
      すべての.bakファイルを削除

  find . -name "*.txt" | xargs -I {{}} cp {{}} {{}}.bak
      各.txtファイルのバックアップを作成

  echo "1 2 3 4 5" | xargs -n 2 echo
      2つずつ引数を渡して実行

  find . -name "*.jpg" | xargs -P 4 -I {{}} convert {{}} {{}}.png
      4並列で画像変換

  ls *.txt | xargs -t wc -l
      実行コマンドを表示しながらwcを実行

  cat urls.txt | xargs -n 1 -P 10 curl -O
      10並列でURLからダウンロード

globパターン対応:
  xargs -a files.txt command    files.txtから引数を読み込む
"#
    );
}

fn print_version() {
    eprintln!("xargs (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn has_glob_meta(arg: &str) -> bool {
    arg.contains('*') || arg.contains('?') || arg.contains('[')
}

/// Windows のシェル展開不足を補うため、実行直前に glob を適用する。
/// マッチしない場合は POSIX シェルの既定挙動に合わせて元の文字列を残す。
fn expand_glob_arg(arg: &str) -> Result<Vec<String>, String> {
    if !has_glob_meta(arg) {
        return Ok(vec![arg.to_string()]);
    }

    let options = MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    let paths: Vec<String> = glob_with(arg, options)
        .map_err(|e| format!("globパターンエラー: {}", e))?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if paths.is_empty() {
        Ok(vec![arg.to_string()])
    } else {
        Ok(paths)
    }
}

fn expand_command_args(args: Vec<String>) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in args {
        expanded.extend(expand_glob_arg(&arg)?);
    }
    Ok(expanded)
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut config = Config::default();
    let mut i = 0;
    let mut command_started = false;

    while i < args.len() {
        let arg = &args[i];

        if command_started {
            // コマンドと引数を収集
            config.initial_args.push(arg.clone());
            i += 1;
            continue;
        }

        if arg == "--help" {
            print_help();
            std::process::exit(0);
        } else if arg == "--version" {
            print_version();
            std::process::exit(0);
        } else if arg == "-0" || arg == "--null" {
            config.null_separator = true;
        } else if arg == "-t" || arg == "--verbose" {
            config.verbose = true;
        } else if arg == "-p" || arg == "--interactive" {
            config.prompt = true;
            config.interactive = true;
        } else if arg == "-r" || arg == "--no-run-if-empty" {
            config.run_if_empty = false;
        } else if arg == "-x" || arg == "--exit" {
            config.exit_on_error = true;
        } else if arg == "-a" || arg.starts_with("--arg-file") {
            let file = if arg == "-a" {
                i += 1;
                if i >= args.len() {
                    return Err("-a オプションにはファイル名が必要です".to_string());
                }
                args[i].clone()
            } else if arg.starts_with("--arg-file=") {
                arg[11..].to_string()
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("--arg-file オプションにはファイル名が必要です".to_string());
                }
                args[i].clone()
            };
            config.input_file = Some(file);
        } else if arg == "-d" || arg.starts_with("--delimiter") {
            let delim_str = if arg == "-d" {
                i += 1;
                if i >= args.len() {
                    return Err("-d オプションには区切り文字が必要です".to_string());
                }
                &args[i]
            } else if arg.starts_with("--delimiter=") {
                &arg[12..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("--delimiter オプションには区切り文字が必要です".to_string());
                }
                &args[i]
            };
            
            let delim = parse_delimiter(delim_str)?;
            config.delimiter = Some(delim);
        } else if arg == "-E" {
            i += 1;
            if i >= args.len() {
                return Err("-E オプションには終了文字列が必要です".to_string());
            }
            config.eof_str = Some(args[i].clone());
        } else if arg == "-I" {
            i += 1;
            if i >= args.len() {
                return Err("-I オプションにはプレースホルダが必要です".to_string());
            }
            config.placeholder = Some(args[i].clone());
            // -I は暗黙的に -L 1 を設定
            if config.max_lines.is_none() {
                config.max_lines = Some(1);
            }
        } else if arg.starts_with("-I") && arg.len() > 2 {
            config.placeholder = Some(arg[2..].to_string());
            if config.max_lines.is_none() {
                config.max_lines = Some(1);
            }
        } else if arg == "-L" {
            i += 1;
            if i >= args.len() {
                return Err("-L オプションには行数が必要です".to_string());
            }
            config.max_lines = Some(
                args[i]
                    .parse()
                    .map_err(|_| format!("無効な行数: '{}'", args[i]))?,
            );
        } else if arg.starts_with("-L") && arg.len() > 2 {
            config.max_lines = Some(
                arg[2..]
                    .parse()
                    .map_err(|_| format!("無効な行数: '{}'", &arg[2..]))?,
            );
        } else if arg == "-n" || arg.starts_with("--max-args") {
            let n_str = if arg == "-n" {
                i += 1;
                if i >= args.len() {
                    return Err("-n オプションには引数数が必要です".to_string());
                }
                &args[i]
            } else if arg.starts_with("--max-args=") {
                &arg[11..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("--max-args オプションには引数数が必要です".to_string());
                }
                &args[i]
            };
            config.max_args = Some(
                n_str
                    .parse()
                    .map_err(|_| format!("無効な引数数: '{}'", n_str))?,
            );
        } else if arg.starts_with("-n") && arg.len() > 2 {
            config.max_args = Some(
                arg[2..]
                    .parse()
                    .map_err(|_| format!("無効な引数数: '{}'", &arg[2..]))?,
            );
        } else if arg == "-P" || arg.starts_with("--max-procs") {
            let p_str = if arg == "-P" {
                i += 1;
                if i >= args.len() {
                    return Err("-P オプションにはプロセス数が必要です".to_string());
                }
                &args[i]
            } else if arg.starts_with("--max-procs=") {
                &arg[12..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("--max-procs オプションにはプロセス数が必要です".to_string());
                }
                &args[i]
            };
            config.max_procs = p_str
                .parse()
                .map_err(|_| format!("無効なプロセス数: '{}'", p_str))?;
            if config.max_procs == 0 {
                // 0 は利用可能なCPU数を意味する
                config.max_procs = std::thread::available_parallelism()
                    .map(|p| p.get())
                    .unwrap_or(1);
            }
        } else if arg.starts_with("-P") && arg.len() > 2 {
            config.max_procs = arg[2..]
                .parse()
                .map_err(|_| format!("無効なプロセス数: '{}'", &arg[2..]))?;
            if config.max_procs == 0 {
                config.max_procs = std::thread::available_parallelism()
                    .map(|p| p.get())
                    .unwrap_or(1);
            }
        } else if arg == "-s" || arg.starts_with("--max-chars") {
            let s_str = if arg == "-s" {
                i += 1;
                if i >= args.len() {
                    return Err("-s オプションには文字数が必要です".to_string());
                }
                &args[i]
            } else if arg.starts_with("--max-chars=") {
                &arg[12..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("--max-chars オプションには文字数が必要です".to_string());
                }
                &args[i]
            };
            config.max_chars = s_str
                .parse()
                .map_err(|_| format!("無効な文字数: '{}'", s_str))?;
        } else if arg.starts_with('-') && arg != "-" {
            // 未知のオプション - コマンドの開始かもしれない
            // 単一の - はオプション終了
            if arg == "--" {
                command_started = true;
            } else {
                return Err(format!("不明なオプション: {}", arg));
            }
        } else {
            // コマンドの開始
            command_started = true;
            config.command = arg.clone();
        }

        i += 1;
    }

    Ok(config)
}

/// 区切り文字をパース（エスケープシーケンス対応）
fn parse_delimiter(s: &str) -> Result<char, String> {
    if s.is_empty() {
        return Err("区切り文字が空です".to_string());
    }

    let mut chars = s.chars();
    let first = chars.next().unwrap();

    if first == '\\' {
        match chars.next() {
            Some('n') => Ok('\n'),
            Some('t') => Ok('\t'),
            Some('r') => Ok('\r'),
            Some('0') => Ok('\0'),
            Some('\\') => Ok('\\'),
            Some(c) if c.is_ascii_digit() => {
                // 8進数
                let mut octal = String::new();
                octal.push(c);
                for c in chars {
                    if c.is_ascii_digit() && c < '8' {
                        octal.push(c);
                    } else {
                        break;
                    }
                }
                let code = u32::from_str_radix(&octal, 8)
                    .map_err(|_| format!("無効な8進数: \\{}", octal))?;
                char::from_u32(code).ok_or_else(|| format!("無効な文字コード: {}", code))
            }
            Some(c) => Ok(c),
            None => Ok('\\'),
        }
    } else {
        Ok(first)
    }
}

/// 入力から引数を読み込む
fn read_arguments(config: &Config) -> Result<Vec<String>, String> {
    let input: Box<dyn BufRead> = if let Some(ref file) = config.input_file {
        let f = std::fs::File::open(file)
            .map_err(|e| format!("xargs: '{}': {}", file, e))?;
        Box::new(BufReader::new(f))
    } else {
        Box::new(BufReader::new(io::stdin()))
    };

    let mut arguments = Vec::new();

    if config.null_separator {
        // NUL区切り
        let mut reader = input;
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];

        loop {
            match reader.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    if byte[0] == 0 {
                        if !buffer.is_empty() {
                            let arg = String::from_utf8_lossy(&buffer).to_string();
                            if should_stop(&arg, config) {
                                break;
                            }
                            arguments.push(arg);
                            buffer.clear();
                        }
                    } else {
                        buffer.push(byte[0]);
                    }
                }
                Err(e) => return Err(format!("読み込みエラー: {}", e)),
            }
        }
        if !buffer.is_empty() {
            let arg = String::from_utf8_lossy(&buffer).to_string();
            if !should_stop(&arg, config) {
                arguments.push(arg);
            }
        }
    } else if let Some(delim) = config.delimiter {
        // カスタム区切り文字
        let mut content = String::new();
        let mut reader = input;
        reader
            .read_to_string(&mut content)
            .map_err(|e| format!("読み込みエラー: {}", e))?;

        // POSIX: 区切り文字で分割した要素は空文字列でも引数として有効。
        // ただし末尾の区切り文字による空要素（最後の空文字列）は除外する。
        let parts: Vec<&str> = content.split(delim).collect();
        let len = parts.len();
        for (idx, part) in parts.into_iter().enumerate() {
            // 末尾の空要素のみ除外
            if part.is_empty() && idx + 1 == len {
                break;
            }
            let arg = part.to_string();
            if should_stop(&arg, config) {
                break;
            }
            arguments.push(arg);
        }
    } else if config.max_lines.is_some() {
        // 行モード
        for line_result in input.lines() {
            let line = line_result.map_err(|e| format!("読み込みエラー: {}", e))?;
            if should_stop(&line, config) {
                break;
            }
            if !line.is_empty() {
                arguments.push(line);
            }
        }
    } else {
        // デフォルト：空白区切り（クォート対応）
        let mut content = String::new();
        let mut reader = input;
        reader
            .read_to_string(&mut content)
            .map_err(|e| format!("読み込みエラー: {}", e))?;

        let parsed = parse_quoted_arguments(&content, config)?;
        arguments = parsed;
    }

    Ok(arguments)
}

/// 終了文字列かチェック
fn should_stop(arg: &str, config: &Config) -> bool {
    if let Some(ref eof) = config.eof_str {
        arg == eof
    } else {
        false
    }
}

/// クォートを考慮して引数をパース
fn parse_quoted_arguments(content: &str, config: &Config) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' if !in_single_quote && !in_double_quote => {
                // クォート外でのバックスラッシュ処理
                // Windowsパス対応：基本的にそのまま保持
                // 特別なケース：スペース、クォート、バックスラッシュのエスケープのみ処理
                if let Some(&next) = chars.peek() {
                    match next {
                        ' ' | '\t' => {
                            // スペース/タブのエスケープ（引数区切りを防ぐ）
                            current.push(chars.next().unwrap());
                        }
                        '\'' | '"' => {
                            // クォートのエスケープ
                            current.push(chars.next().unwrap());
                        }
                        '\\' => {
                            // バックスラッシュのエスケープ
                            chars.next();
                            current.push('\\');
                        }
                        '\n' => {
                            // 行継続（バックスラッシュ+改行は無視）
                            chars.next();
                        }
                        _ => {
                            // それ以外はバックスラッシュをそのまま保持
                            // （Windowsパス: .\news.txt, C:\Users など）
                            current.push('\\');
                        }
                    }
                } else {
                    // 末尾のバックスラッシュ
                    current.push('\\');
                }
            }
            '\\' if in_double_quote => {
                // ダブルクォート内でのバックスラッシュ
                if let Some(&next) = chars.peek() {
                    match next {
                        '"' | '\\' | '$' | '`' => {
                            // 特殊文字のエスケープ
                            current.push(chars.next().unwrap());
                        }
                        _ => {
                            current.push('\\');
                        }
                    }
                } else {
                    current.push('\\');
                }
            }
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' | '\n' | '\r' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    if should_stop(&current, config) {
                        break;
                    }
                    arguments.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() && !should_stop(&current, config) {
        arguments.push(current);
    }

    if in_single_quote || in_double_quote {
        return Err("閉じられていないクォートがあります".to_string());
    }

    Ok(arguments)
}

/// ユーザーに確認
fn prompt_user(command: &str, args: &[String]) -> bool {
    eprint!("{}", command);
    for arg in args {
        eprint!(" {}", arg);
    }
    eprint!("?...");
    io::stderr().flush().ok();

    // stdin ではなく TTY から直接読む。
    // これにより read_arguments が stdin を使い切った後でも
    // -p プロンプトへの応答が可能になる。
    let mut response = String::new();
    let ok = open_tty_reader()
        .and_then(|mut r| r.read_line(&mut response).ok())
        .is_some();
    if ok {
        let r = response.trim().to_lowercase();
        r == "y" || r == "yes"
    } else {
        false
    }
}

/// TTY / コンソールへの読み取り専用アクセスを開く。
/// Unix: /dev/tty  Windows: CONIN$
fn open_tty_reader() -> Option<io::BufReader<std::fs::File>> {
    #[cfg(unix)]
    let tty_path = "/dev/tty";
    #[cfg(windows)]
    let tty_path = "CONIN$";
    #[cfg(not(any(unix, windows)))]
    let tty_path = "/dev/tty";

    std::fs::OpenOptions::new()
        .read(true)
        .open(tty_path)
        .ok()
        .map(io::BufReader::new)
}

/// コマンドを実行
fn execute_command(
    command: &str,
    initial_args: &[String],
    args: &[String],
    config: &Config,
) -> Result<i32, String> {
    let final_args: Vec<String> = if let Some(ref placeholder) = config.placeholder {
        // プレースホルダ置換
        // -I モードでは 1バッチ = 1引数。args.join(" ") は改行を含む値を
        // 空白で結合してしまうため、最初の引数のみを使う。
        // 複数引数が渡された場合は各引数を連結した文字列で置換する（GNU xargs 互換）。
        let replacement = if args.len() == 1 {
            args[0].clone()
        } else {
            args.join(" ")
        };
        initial_args
            .iter()
            .map(|a| a.replace(placeholder, &replacement))
            .collect()
    } else {
        initial_args
            .iter()
            .cloned()
            .chain(args.iter().cloned())
            .collect()
    };
    let final_args = expand_command_args(final_args)?;

    if config.verbose {
        eprint!("{}", command);
        for arg in &final_args {
            eprint!(" {}", arg);
        }
        eprintln!();
    }

    if config.prompt {
        if !prompt_user(command, &final_args) {
            return Ok(0);
        }
    }

    // ↓ final_args はここ以降 all_args に変換して使う
    // -p モード時は stdin を閉じる（TTY から読むため競合しない）
    // 通常時は stdin を継承してコマンドにパイプ入力を渡せるようにする。
    let stdin_cfg = if config.prompt {
        Stdio::null()
    } else {
        Stdio::inherit()
    };

    // Windows では "echo" は cmd.exe の内部コマンドのため
    // Command::new("echo") が失敗する。cmd /c echo に変換する。
    #[cfg(windows)]
    let (actual_cmd, prepended): (&str, Vec<String>) = if command == "echo" {
        ("cmd", vec!["/c".to_string(), "echo".to_string()])
    } else {
        (command, vec![])
    };
    #[cfg(not(windows))]
    let (actual_cmd, prepended): (&str, Vec<String>) = (command, vec![]);

    let all_args: Vec<String> = prepended.into_iter().chain(final_args.into_iter()).collect();

    let status = Command::new(actual_cmd)
        .args(&all_args)
        .stdin(stdin_cfg)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("xargs: {}: {}", command, e))?;

    Ok(status.code().unwrap_or(1))
}

/// 引数をバッチに分割
fn split_into_batches(
    arguments: Vec<String>,
    config: &Config,
) -> Vec<Vec<String>> {
    let mut batches = Vec::new();

    if arguments.is_empty() {
        return batches;
    }

    if config.max_lines.is_some() && config.placeholder.is_some() {
        // -I モード：1行ずつ処理
        for arg in arguments {
            batches.push(vec![arg]);
        }
        return batches;
    }

    if let Some(max_args) = config.max_args {
        // -n: 最大引数数で分割。-s も同時に考慮する。
        let base_len = config.command.len()
            + config.initial_args.iter().map(|a| a.len() + 1).sum::<usize>();
        let mut current_batch: Vec<String> = Vec::new();
        let mut current_len = base_len;
        for arg in arguments {
            let arg_len = arg.len() + 1;
            // -n の上限 または -s の上限 を超えたらフラッシュ
            let n_exceeded = current_batch.len() >= max_args;
            let s_exceeded = current_len + arg_len > config.max_chars
                && !current_batch.is_empty();
            if n_exceeded || s_exceeded {
                batches.push(current_batch);
                current_batch = Vec::new();
                current_len = base_len;
            }
            current_len += arg_len;
            current_batch.push(arg);
        }
        if !current_batch.is_empty() {
            batches.push(current_batch);
        }
        return batches;
    }

    if let Some(max_lines) = config.max_lines {
        // -L: 最大行数で分割
        for chunk in arguments.chunks(max_lines) {
            batches.push(chunk.to_vec());
        }
        return batches;
    }

    // コマンドライン長で分割
    let base_len = config.command.len()
        + config.initial_args.iter().map(|a| a.len() + 1).sum::<usize>();

    let mut current_batch = Vec::new();
    let mut current_len = base_len;

    for arg in arguments {
        let arg_len = arg.len() + 1; // スペース分

        if current_len + arg_len > config.max_chars && !current_batch.is_empty() {
            batches.push(current_batch);
            current_batch = Vec::new();
            current_len = base_len;
        }

        current_len += arg_len;
        current_batch.push(arg);
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    batches
}

/// メイン処理
fn process(config: &Config) -> Result<i32, String> {
    let arguments = read_arguments(config)?;

    if arguments.is_empty() && !config.run_if_empty {
        return Ok(0);
    }

    if arguments.is_empty() && config.run_if_empty {
        // 引数なしで1回実行
        return execute_command(
            &config.command,
            &config.initial_args,
            &[],
            config,
        );
    }

    let batches = split_into_batches(arguments, config);

    // split_into_batches が空を返すケースは arguments が空の場合のみ。
    // その場合は上の run_if_empty チェックで処理済みなのでここには来ないが、
    // 念のため同じ判定を維持する。
    if batches.is_empty() {
        return Ok(0);
    }

    let mut exit_code = 0;

    if config.max_procs > 1 {
        // 並列実行
        use std::sync::{Arc, Mutex};
        use std::thread;

        let batches = Arc::new(Mutex::new(batches.into_iter()));
        let exit_code = Arc::new(Mutex::new(0i32));
        let config = Arc::new(config.clone());

        let mut handles = Vec::new();

        for _ in 0..config.max_procs {
            let batches = Arc::clone(&batches);
            let exit_code = Arc::clone(&exit_code);
            let config = Arc::clone(&config);

            let handle = thread::spawn(move || {
                loop {
                    let batch = {
                        let mut batches = batches.lock().unwrap();
                        batches.next()
                    };

                    match batch {
                        Some(args) => {
                            // exit_on_error が true かつ既にエラーがある場合は実行しない
                            {
                                let ec = exit_code.lock().unwrap();
                                if config.exit_on_error && *ec != 0 {
                                    break;
                                }
                            }
                            match execute_command(
                                &config.command,
                                &config.initial_args,
                                &args,
                                &config,
                            ) {
                                Ok(code) if code != 0 => {
                                    let mut ec = exit_code.lock().unwrap();
                                    if *ec == 0 {
                                        *ec = code;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("{}", e);
                                    let mut ec = exit_code.lock().unwrap();
                                    if *ec == 0 {
                                        *ec = 1;
                                    }
                                }
                                _ => {}
                            }
                        }
                        None => break,
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.join().ok();
        }

        let final_code = *exit_code.lock().unwrap();
        return Ok(final_code);
    }

    // 順次実行
    for batch in batches {
        match execute_command(&config.command, &config.initial_args, &batch, config) {
            Ok(code) => {
                if code != 0 {
                    exit_code = code;
                    if config.exit_on_error {
                        return Ok(exit_code);
                    }
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                exit_code = 1;
                if config.exit_on_error {
                    return Ok(exit_code);
                }
            }
        }
    }

    Ok(exit_code)
}

// Clone実装（並列処理用）
impl Clone for Config {
    fn clone(&self) -> Self {
        Config {
            command: self.command.clone(),
            initial_args: self.initial_args.clone(),
            delimiter: self.delimiter,
            max_args: self.max_args,
            max_chars: self.max_chars,
            placeholder: self.placeholder.clone(),
            verbose: self.verbose,
            prompt: self.prompt,
            run_if_empty: self.run_if_empty,
            eof_str: self.eof_str.clone(),
            max_procs: self.max_procs,
            null_separator: self.null_separator,
            max_lines: self.max_lines,
            exit_on_error: self.exit_on_error,
            input_file: self.input_file.clone(),
            interactive: self.interactive,
        }
    }
}

fn main() {
    match parse_args() {
        Ok(config) => match process(&config) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("xargs: {}", e);
            eprintln!("詳しくは 'xargs --help' を参照してください");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_command_args, expand_glob_arg, has_glob_meta, parse_quoted_arguments, Config};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("xargs_{name}_{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_glob_meta_characters() {
        assert!(has_glob_meta("*.txt"));
        assert!(has_glob_meta("file?.txt"));
        assert!(has_glob_meta("[ab].txt"));
        assert!(!has_glob_meta("plain.txt"));
    }

    #[test]
    fn keeps_unmatched_pattern_as_is() {
        let expanded = expand_glob_arg("does-not-exist-*.txt").unwrap();
        assert_eq!(expanded, vec!["does-not-exist-*.txt".to_string()]);
    }

    #[test]
    fn expands_multiple_arguments_and_preserves_literals() {
        let dir = temp_test_dir("glob_multi");
        let alpha = dir.join("alpha.txt");
        let beta = dir.join("beta.txt");
        fs::write(&alpha, "a").unwrap();
        fs::write(&beta, "b").unwrap();

        let pattern = format!("{}\\*.txt", dir.display());
        let expanded = expand_command_args(vec!["prefix".to_string(), pattern]).unwrap();

        assert_eq!(expanded[0], "prefix");
        assert_eq!(expanded.len(), 3);
        assert!(expanded.iter().any(|p| p.ends_with("alpha.txt")));
        assert!(expanded.iter().any(|p| p.ends_with("beta.txt")));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn glob_matching_is_case_insensitive_on_windows_style_paths() {
        let dir = temp_test_dir("glob_case");
        let mixed = dir.join("News.TXT");
        fs::write(&mixed, "hello").unwrap();

        let pattern = format!("{}\\*.txt", dir.display());
        let expanded = expand_glob_arg(&pattern).unwrap();

        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].ends_with("News.TXT"));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quoted_argument_parser_treats_crlf_as_separator() {
        let parsed = parse_quoted_arguments("alpha\r\nbeta\r\n", &Config::default()).unwrap();
        assert_eq!(parsed, vec!["alpha".to_string(), "beta".to_string()]);
    }
}
