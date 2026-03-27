use std::cmp::Ordering;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::process;

use glob::glob;

#[derive(Debug, Clone)]
struct OutputSpec {
    file: usize,  // 0 = join field, 1 = file1, 2 = file2
    field: usize, // 1-indexed field number
}

#[derive(Debug)]
struct Config {
    /// 最初のファイル
    file1: String,
    /// 2番目のファイル
    file2: String,
    /// file1の結合フィールド（1-indexed）
    field1: usize,
    /// file2の結合フィールド（1-indexed）
    field2: usize,
    /// 入力フィールドセパレータ（-t）
    input_separator: Option<char>,
    /// 出力フィールドセパレータ
    output_separator: String,
    /// 出力フォーマット（-o）
    output_format: Option<Vec<OutputSpec>>,
    /// 空フィールドの置換（-e）
    empty_replacement: String,
    /// マッチしない行も出力（-a）
    print_unpaired: Vec<usize>, // 1 or 2
    /// マッチしない行のみ出力（-v）
    only_unpaired: Vec<usize>, // 1 or 2
    /// 大文字小文字を無視（-i, GNU拡張）
    ignore_case: bool,
    /// ソート確認（--check-order, GNU拡張）
    check_order: bool,
    /// ソート確認を無視（--nocheck-order, GNU拡張）
    nocheck_order: bool,
    /// ヘッダ行（--header, GNU拡張）
    header: bool,
    /// ゼロ終端（-z, GNU拡張）
    zero_terminated: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            file1: String::new(),
            file2: String::new(),
            field1: 1,
            field2: 1,
            input_separator: None,
            output_separator: " ".to_string(),
            output_format: None,
            empty_replacement: String::new(),
            print_unpaired: Vec::new(),
            only_unpaired: Vec::new(),
            ignore_case: false,
            check_order: false,
            nocheck_order: false,
            header: false,
            zero_terminated: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: join [オプション]... ファイル1 ファイル2
2つのソート済みファイルを結合フィールドで結合します。

ファイルがソートされていない場合、結果は未定義です。
ファイル名に - を指定すると標準入力を読み込みます。

オプション:
  -1 FIELD              file1の結合フィールド（デフォルト：1）
  -2 FIELD              file2の結合フィールド（デフォルト：1）
  -j FIELD              -1 FIELD -2 FIELD と同じ
  -t CHAR               入出力のフィールドセパレータ（デフォルト：空白）
  -o FORMAT             出力フォーマットを指定（例：1.1,2.2）
                        0 は結合フィールド、N.M はファイルNのフィールドM
  -e STRING             空フィールドをSTRINGで置換
  -a FILENUM            マッチしない行も出力（1 or 2）
  -v FILENUM            マッチしない行のみ出力（1 or 2）
  -i, --ignore-case     大文字小文字を無視して比較（GNU拡張）
  -z, --zero-terminated 行末をNUL文字として扱う（GNU拡張）
      --check-order     入力がソートされているかチェック（GNU拡張）
      --nocheck-order   入力のソートをチェックしない（GNU拡張）
      --header          最初の行をヘッダとして扱う（GNU拡張）
      --help            このヘルプを表示
      --version         バージョン情報を表示

-o フォーマット:
  0       結合フィールド
  1.N     file1のN番目のフィールド
  2.N     file2のN番目のフィールド
  auto    自動（デフォルト動作）

例:
  join file1 file2                   最初のフィールドで結合
  join -1 2 -2 1 file1 file2         file1の2列目とfile2の1列目で結合
  join -t, file1.csv file2.csv       カンマ区切りで結合
  join -o 1.1,1.2,2.2 file1 file2    出力フィールドを指定
  join -a 1 file1 file2              file1の未マッチ行も出力
  join -v 1 file1 file2              file1の未マッチ行のみ出力

globパターン対応:
  Windows では位置引数の glob を内部展開します
  join sorted*.txt other.txt         Linux のシェル展開に近い形で処理
"#
    );
}

fn print_version() {
    eprintln!("join (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn has_glob_meta_chars(arg: &str) -> bool {
    arg.contains('*') || arg.contains('?') || arg.contains('[')
}

#[cfg(windows)]
fn expand_positional_arg(arg: &str) -> Result<Vec<String>, String> {
    if arg == "-" || !has_glob_meta_chars(arg) {
        return Ok(vec![arg.to_string()]);
    }

    let matches: Vec<String> = glob(arg)
        .map_err(|e| format!("globパターンエラー: {}", e))?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if matches.is_empty() {
        Ok(vec![arg.to_string()])
    } else {
        Ok(matches)
    }
}

#[cfg(not(windows))]
fn expand_positional_arg(arg: &str) -> Result<Vec<String>, String> {
    Ok(vec![arg.to_string()])
}

fn expand_positional_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in args {
        expanded.extend(expand_positional_arg(arg)?);
    }
    Ok(expanded)
}

/// 出力フォーマットをパース
fn parse_output_format(s: &str) -> Result<Vec<OutputSpec>, String> {
    let mut specs = Vec::new();

    for part in s.split(',') {
        let part = part.trim();
        if part == "0" {
            specs.push(OutputSpec { file: 0, field: 0 });
        } else if part.contains('.') {
            let parts: Vec<&str> = part.split('.').collect();
            if parts.len() != 2 {
                return Err(format!("無効な出力フォーマット: '{}'", part));
            }
            let file: usize = parts[0]
                .parse()
                .map_err(|_| format!("無効なファイル番号: '{}'", parts[0]))?;
            let field: usize = parts[1]
                .parse()
                .map_err(|_| format!("無効なフィールド番号: '{}'", parts[1]))?;
            if file != 1 && file != 2 {
                return Err(format!("ファイル番号は1または2: '{}'", file));
            }
            specs.push(OutputSpec { file, field });
        } else if part == "auto" {
            return Ok(Vec::new()); // auto = デフォルト
        } else {
            return Err(format!("無効な出力フォーマット: '{}'", part));
        }
    }

    Ok(specs)
}

fn parse_args_from<I>(iter: I) -> Result<Config, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = iter.into_iter().collect();
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
        } else if arg == "-1" {
            i += 1;
            if i >= args.len() {
                return Err("-1 にはフィールド番号が必要です".to_string());
            }
            config.field1 = args[i]
                .parse()
                .map_err(|_| format!("無効なフィールド番号: '{}'", args[i]))?;
        } else if arg.starts_with("-1") && arg.len() > 2 {
            config.field1 = arg[2..]
                .parse()
                .map_err(|_| format!("無効なフィールド番号: '{}'", &arg[2..]))?;
        } else if arg == "-2" {
            i += 1;
            if i >= args.len() {
                return Err("-2 にはフィールド番号が必要です".to_string());
            }
            config.field2 = args[i]
                .parse()
                .map_err(|_| format!("無効なフィールド番号: '{}'", args[i]))?;
        } else if arg.starts_with("-2") && arg.len() > 2 {
            config.field2 = arg[2..]
                .parse()
                .map_err(|_| format!("無効なフィールド番号: '{}'", &arg[2..]))?;
        } else if arg == "-j" {
            i += 1;
            if i >= args.len() {
                return Err("-j にはフィールド番号が必要です".to_string());
            }
            let field: usize = args[i]
                .parse()
                .map_err(|_| format!("無効なフィールド番号: '{}'", args[i]))?;
            config.field1 = field;
            config.field2 = field;
        } else if arg == "-t" {
            i += 1;
            if i >= args.len() {
                return Err("-t にはセパレータが必要です".to_string());
            }
            let sep = &args[i];
            if sep.is_empty() {
                return Err("セパレータは空にできません".to_string());
            }
            config.input_separator = Some(sep.chars().next().unwrap());
            config.output_separator = sep.chars().next().unwrap().to_string();
        } else if arg.starts_with("-t") && arg.len() > 2 {
            config.input_separator = Some(arg.chars().nth(2).unwrap());
            config.output_separator = arg.chars().nth(2).unwrap().to_string();
        } else if arg == "-o" {
            i += 1;
            if i >= args.len() {
                return Err("-o には出力フォーマットが必要です".to_string());
            }
            let specs = parse_output_format(&args[i])?;
            if !specs.is_empty() {
                config.output_format = Some(specs);
            }
        } else if arg.starts_with("-o") && arg.len() > 2 {
            let specs = parse_output_format(&arg[2..])?;
            if !specs.is_empty() {
                config.output_format = Some(specs);
            }
        } else if arg == "-e" {
            i += 1;
            if i >= args.len() {
                return Err("-e には置換文字列が必要です".to_string());
            }
            config.empty_replacement = args[i].clone();
        } else if arg.starts_with("-e") && arg.len() > 2 {
            config.empty_replacement = arg[2..].to_string();
        } else if arg == "-a" {
            i += 1;
            if i >= args.len() {
                return Err("-a にはファイル番号が必要です".to_string());
            }
            let filenum: usize = args[i]
                .parse()
                .map_err(|_| format!("無効なファイル番号: '{}'", args[i]))?;
            if filenum != 1 && filenum != 2 {
                return Err("-a のファイル番号は1または2".to_string());
            }
            if !config.print_unpaired.contains(&filenum) {
                config.print_unpaired.push(filenum);
            }
        } else if arg.starts_with("-a") && arg.len() > 2 {
            let filenum: usize = arg[2..]
                .parse()
                .map_err(|_| format!("無効なファイル番号: '{}'", &arg[2..]))?;
            if filenum != 1 && filenum != 2 {
                return Err("-a のファイル番号は1または2".to_string());
            }
            if !config.print_unpaired.contains(&filenum) {
                config.print_unpaired.push(filenum);
            }
        } else if arg == "-v" {
            i += 1;
            if i >= args.len() {
                return Err("-v にはファイル番号が必要です".to_string());
            }
            let filenum: usize = args[i]
                .parse()
                .map_err(|_| format!("無効なファイル番号: '{}'", args[i]))?;
            if filenum != 1 && filenum != 2 {
                return Err("-v のファイル番号は1または2".to_string());
            }
            if !config.only_unpaired.contains(&filenum) {
                config.only_unpaired.push(filenum);
            }
        } else if arg.starts_with("-v") && arg.len() > 2 {
            let filenum: usize = arg[2..]
                .parse()
                .map_err(|_| format!("無効なファイル番号: '{}'", &arg[2..]))?;
            if filenum != 1 && filenum != 2 {
                return Err("-v のファイル番号は1または2".to_string());
            }
            if !config.only_unpaired.contains(&filenum) {
                config.only_unpaired.push(filenum);
            }
        } else if arg == "-i" || arg == "--ignore-case" {
            config.ignore_case = true;
        } else if arg == "-z" || arg == "--zero-terminated" {
            config.zero_terminated = true;
        } else if arg == "--check-order" {
            config.check_order = true;
        } else if arg == "--nocheck-order" {
            config.nocheck_order = true;
        } else if arg == "--header" {
            config.header = true;
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                positional.push(args[j].clone());
            }
            break;
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("不明なオプション: {}", arg));
        } else {
            positional.push(arg.clone());
        }

        i += 1;
    }

    let positional = expand_positional_args(&positional)?;

    if positional.len() < 2 {
        return Err("結合する2つのファイルを指定してください".to_string());
    }

    if positional.len() > 2 {
        return Err("引数が多すぎます".to_string());
    }

    config.file1 = positional[0].clone();
    config.file2 = positional[1].clone();

    Ok(config)
}

fn parse_args() -> Result<Config, String> {
    parse_args_from(env::args().skip(1))
}

/// ファイルリーダー
enum InputReader {
    File(BufReader<File>),
    Stdin(BufReader<io::Stdin>),
}

impl InputReader {
    fn new(path: &str) -> Result<Self, String> {
        if path == "-" {
            Ok(InputReader::Stdin(BufReader::new(io::stdin())))
        } else {
            let file = File::open(path).map_err(|e| format!("join: '{}': {}", path, e))?;
            Ok(InputReader::File(BufReader::new(file)))
        }
    }

    fn read_line(&mut self, buf: &mut String, zero_terminated: bool) -> io::Result<usize> {
        buf.clear();
        if zero_terminated {
            let reader: &mut dyn BufRead = match self {
                InputReader::File(r) => r,
                InputReader::Stdin(r) => r,
            };
            let mut byte = [0u8; 1];
            let mut count = 0;
            loop {
                match std::io::Read::read(reader, &mut byte)? {
                    0 => break,
                    _ => {
                        count += 1;
                        if byte[0] == 0 {
                            break;
                        }
                        buf.push(byte[0] as char);
                    }
                }
            }
            Ok(count)
        } else {
            match self {
                InputReader::File(r) => r.read_line(buf),
                InputReader::Stdin(r) => r.read_line(buf),
            }
        }
    }
}

/// 行をフィールドに分割
fn split_fields(line: &str, separator: Option<char>) -> Vec<String> {
    let trimmed = line.trim_end_matches(&['\n', '\r', '\0'][..]);

    if let Some(sep) = separator {
        trimmed.split(sep).map(|s| s.to_string()).collect()
    } else {
        // 空白で分割（連続空白は1つとして扱う）
        trimmed.split_whitespace().map(|s| s.to_string()).collect()
    }
}

/// 結合フィールドを取得（比較用）
fn get_join_key(fields: &[String], field_num: usize, ignore_case: bool) -> String {
    if field_num == 0 || field_num > fields.len() {
        return String::new();
    }
    let key = &fields[field_num - 1];
    if ignore_case {
        key.to_lowercase()
    } else {
        key.clone()
    }
}

/// 出力行を生成
fn format_output(fields1: &[String], fields2: &[String], config: &Config) -> String {
    let mut parts = Vec::new();

    if let Some(ref format) = config.output_format {
        for spec in format {
            let value = match spec.file {
                0 => {
                    // 結合フィールド
                    if !fields1.is_empty() && config.field1 <= fields1.len() {
                        fields1[config.field1 - 1].clone()
                    } else if !fields2.is_empty() && config.field2 <= fields2.len() {
                        fields2[config.field2 - 1].clone()
                    } else {
                        config.empty_replacement.clone()
                    }
                }
                1 => {
                    if spec.field > 0 && spec.field <= fields1.len() {
                        fields1[spec.field - 1].clone()
                    } else {
                        config.empty_replacement.clone()
                    }
                }
                2 => {
                    if spec.field > 0 && spec.field <= fields2.len() {
                        fields2[spec.field - 1].clone()
                    } else {
                        config.empty_replacement.clone()
                    }
                }
                _ => config.empty_replacement.clone(),
            };
            parts.push(value);
        }
    } else {
        // デフォルト: 結合フィールド + file1の他フィールド + file2の他フィールド
        // 結合フィールド
        if !fields1.is_empty() && config.field1 <= fields1.len() {
            parts.push(fields1[config.field1 - 1].clone());
        } else if !fields2.is_empty() && config.field2 <= fields2.len() {
            parts.push(fields2[config.field2 - 1].clone());
        }

        // file1の他フィールド
        for (i, f) in fields1.iter().enumerate() {
            if i + 1 != config.field1 {
                parts.push(f.clone());
            }
        }

        // file2の他フィールド
        for (i, f) in fields2.iter().enumerate() {
            if i + 1 != config.field2 {
                parts.push(f.clone());
            }
        }
    }

    parts.join(&config.output_separator)
}

/// 未マッチ行を出力
fn format_unpaired(fields: &[String], file_num: usize, config: &Config) -> String {
    if let Some(ref format) = config.output_format {
        let mut parts = Vec::new();
        let field_num = if file_num == 1 {
            config.field1
        } else {
            config.field2
        };

        for spec in format {
            let value = if spec.file == 0 {
                // 結合フィールド
                if field_num > 0 && field_num <= fields.len() {
                    fields[field_num - 1].clone()
                } else {
                    config.empty_replacement.clone()
                }
            } else if spec.file == file_num {
                if spec.field > 0 && spec.field <= fields.len() {
                    fields[spec.field - 1].clone()
                } else {
                    config.empty_replacement.clone()
                }
            } else {
                config.empty_replacement.clone()
            };
            parts.push(value);
        }
        parts.join(&config.output_separator)
    } else {
        // デフォルト出力
        fields.join(&config.output_separator)
    }
}

/// join処理
fn join(config: &Config) -> Result<(), String> {
    let mut reader1 = InputReader::new(&config.file1)?;
    let mut reader2 = InputReader::new(&config.file2)?;

    let mut line1 = String::new();
    let mut line2 = String::new();

    let line_terminator = if config.zero_terminated { "\0" } else { "\n" };

    // ヘッダ処理
    if config.header {
        let has1 = reader1
            .read_line(&mut line1, config.zero_terminated)
            .map_err(|e| format!("join: 読み込みエラー: {}", e))?
            > 0;
        let has2 = reader2
            .read_line(&mut line2, config.zero_terminated)
            .map_err(|e| format!("join: 読み込みエラー: {}", e))?
            > 0;

        if has1 || has2 {
            let fields1 = if has1 {
                split_fields(&line1, config.input_separator)
            } else {
                Vec::new()
            };
            let fields2 = if has2 {
                split_fields(&line2, config.input_separator)
            } else {
                Vec::new()
            };
            let output = format_output(&fields1, &fields2, config);
            print!("{}{}", output, line_terminator);
        }
    }

    let mut has_line1 = reader1
        .read_line(&mut line1, config.zero_terminated)
        .map_err(|e| format!("join: 読み込みエラー: {}", e))?
        > 0;
    let mut has_line2 = reader2
        .read_line(&mut line2, config.zero_terminated)
        .map_err(|e| format!("join: 読み込みエラー: {}", e))?
        > 0;

    let mut prev_key1: Option<String> = None;
    let mut prev_key2: Option<String> = None;

    // file2の同一キー行をバッファ
    let mut file2_buffer: Vec<Vec<String>> = Vec::new();
    let mut file2_buffer_key: Option<String> = None;

    while has_line1 || has_line2 {
        let fields1 = if has_line1 {
            split_fields(&line1, config.input_separator)
        } else {
            Vec::new()
        };
        let fields2 = if has_line2 {
            split_fields(&line2, config.input_separator)
        } else {
            Vec::new()
        };

        let key1 = if has_line1 {
            get_join_key(&fields1, config.field1, config.ignore_case)
        } else {
            String::new()
        };
        let key2 = if has_line2 {
            get_join_key(&fields2, config.field2, config.ignore_case)
        } else {
            String::new()
        };

        // ソート順チェック
        if config.check_order && !config.nocheck_order {
            if let Some(ref prev) = prev_key1 {
                if has_line1 && key1 < *prev {
                    eprintln!("join: ファイル1がソートされていません: {}", line1.trim());
                }
            }
            if let Some(ref prev) = prev_key2 {
                if has_line2 && key2 < *prev {
                    eprintln!("join: ファイル2がソートされていません: {}", line2.trim());
                }
            }
        }

        let cmp = if !has_line1 {
            Ordering::Greater
        } else if !has_line2 {
            Ordering::Less
        } else {
            key1.cmp(&key2)
        };

        match cmp {
            Ordering::Less => {
                // file1のみ
                if config.only_unpaired.contains(&1)
                    || (config.print_unpaired.contains(&1) && config.only_unpaired.is_empty())
                {
                    let output = format_unpaired(&fields1, 1, config);
                    print!("{}{}", output, line_terminator);
                }
                prev_key1 = Some(key1);
                has_line1 = reader1
                    .read_line(&mut line1, config.zero_terminated)
                    .map_err(|e| format!("join: 読み込みエラー: {}", e))?
                    > 0;
            }
            Ordering::Greater => {
                // file2のみ
                if config.only_unpaired.contains(&2)
                    || (config.print_unpaired.contains(&2) && config.only_unpaired.is_empty())
                {
                    let output = format_unpaired(&fields2, 2, config);
                    print!("{}{}", output, line_terminator);
                }
                prev_key2 = Some(key2);
                has_line2 = reader2
                    .read_line(&mut line2, config.zero_terminated)
                    .map_err(|e| format!("join: 読み込みエラー: {}", e))?
                    > 0;
            }
            Ordering::Equal => {
                // マッチ
                if config.only_unpaired.is_empty() {
                    // file2の同一キー行をバッファに集める
                    if file2_buffer_key.as_ref() != Some(&key2) {
                        file2_buffer.clear();
                        file2_buffer_key = Some(key2.clone());
                        file2_buffer.push(fields2.clone());

                        // 同一キーの行を読み進める
                        loop {
                            let mut next_line2 = String::new();
                            let has_next = reader2
                                .read_line(&mut next_line2, config.zero_terminated)
                                .map_err(|e| format!("join: 読み込みエラー: {}", e))?
                                > 0;

                            if !has_next {
                                has_line2 = false;
                                break;
                            }

                            let next_fields2 = split_fields(&next_line2, config.input_separator);
                            let next_key2 =
                                get_join_key(&next_fields2, config.field2, config.ignore_case);

                            if next_key2 == key2 {
                                file2_buffer.push(next_fields2);
                            } else {
                                line2 = next_line2;
                                has_line2 = true;
                                break;
                            }
                        }
                    }

                    // file1の行とfile2のバッファ内全行を結合
                    for buf_fields2 in &file2_buffer {
                        let output = format_output(&fields1, buf_fields2, config);
                        print!("{}{}", output, line_terminator);
                    }
                } else {
                    // -v オプションが指定されている場合、マッチした行はスキップ
                    // file2も進める必要がある
                    loop {
                        let mut next_line2 = String::new();
                        let has_next = reader2
                            .read_line(&mut next_line2, config.zero_terminated)
                            .map_err(|e| format!("join: 読み込みエラー: {}", e))?
                            > 0;

                        if !has_next {
                            has_line2 = false;
                            break;
                        }

                        let next_fields2 = split_fields(&next_line2, config.input_separator);
                        let next_key2 =
                            get_join_key(&next_fields2, config.field2, config.ignore_case);

                        if next_key2 != key2 {
                            line2 = next_line2;
                            has_line2 = true;
                            break;
                        }
                    }
                }

                prev_key1 = Some(key1);
                prev_key2 = Some(key2);
                has_line1 = reader1
                    .read_line(&mut line1, config.zero_terminated)
                    .map_err(|e| format!("join: 読み込みエラー: {}", e))?
                    > 0;

                // file1の次の行が同じキーでなければバッファをクリア
                if has_line1 {
                    let next_fields1 = split_fields(&line1, config.input_separator);
                    let next_key1 = get_join_key(&next_fields1, config.field1, config.ignore_case);
                    if file2_buffer_key.as_ref() != Some(&next_key1) {
                        file2_buffer.clear();
                        file2_buffer_key = None;
                    }
                }
            }
        }
    }

    Ok(())
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join: {}", e);
            eprintln!("詳しくは 'join --help' を参照してください");
            process::exit(1);
        }
    };

    if let Err(e) = join(&config) {
        eprintln!("{}", e);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        dir.push(format!("join-tests-{}-{}", process::id(), stamp));
        dir
    }

    #[test]
    fn parse_args_accepts_two_plain_files() {
        let config = parse_args_from(["left.txt", "right.txt"].into_iter().map(String::from))
            .expect("plain args should parse");

        assert_eq!(config.file1, "left.txt");
        assert_eq!(config.file2, "right.txt");
    }

    #[test]
    fn parse_args_rejects_more_than_two_positionals() {
        let err = parse_args_from(["a", "b", "c"].into_iter().map(String::from))
            .expect_err("extra operands should fail");

        assert!(err.contains("引数が多すぎます"));
    }

    #[cfg(windows)]
    #[test]
    fn unmatched_glob_stays_literal_on_windows() {
        let config = parse_args_from(["no-such-*.txt", "right.txt"].into_iter().map(String::from))
            .expect("unmatched glob should stay literal");

        assert_eq!(config.file1, "no-such-*.txt");
        assert_eq!(config.file2, "right.txt");
    }

    #[cfg(windows)]
    #[test]
    fn matched_glob_can_expand_to_extra_operands_on_windows() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("temp dir should be created");
        fs::write(dir.join("left-a.txt"), b"").expect("left-a should be created");
        fs::write(dir.join("left-b.txt"), b"").expect("left-b should be created");
        fs::write(dir.join("right.txt"), b"").expect("right should be created");

        let pattern = format!("{}\\left-*.txt", dir.display());
        let right = dir.join("right.txt").to_string_lossy().to_string();
        let err = parse_args_from([pattern, right].into_iter())
            .expect_err("multiple matches should become extra operands");

        assert!(err.contains("引数が多すぎます"));

        fs::remove_dir_all(&dir).expect("temp dir should be removed");
    }
}
