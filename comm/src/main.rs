use std::cmp::Ordering;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::process;

use glob::glob;

#[derive(Debug)]
struct Config {
    /// 最初のファイル
    file1: String,
    /// 2番目のファイル
    file2: String,
    /// file1のみの行を抑制（-1）
    suppress_col1: bool,
    /// file2のみの行を抑制（-2）
    suppress_col2: bool,
    /// 共通行を抑制（-3）
    suppress_col3: bool,
    /// 大文字小文字を無視（-i, GNU拡張）
    ignore_case: bool,
    /// 出力デリミタ（--output-delimiter, GNU拡張）
    output_delimiter: String,
    /// ソート確認（--check-order, GNU拡張）
    check_order: bool,
    /// ソート確認を無視（--nocheck-order, GNU拡張）
    nocheck_order: bool,
    /// 合計を表示（--total, GNU拡張）
    show_total: bool,
    /// ゼロ終端（-z, GNU拡張）
    zero_terminated: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            file1: String::new(),
            file2: String::new(),
            suppress_col1: false,
            suppress_col2: false,
            suppress_col3: false,
            ignore_case: false,
            output_delimiter: "\t".to_string(),
            check_order: false,
            nocheck_order: false,
            show_total: false,
            zero_terminated: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: comm [オプション]... ファイル1 ファイル2
ソート済みの2つのファイルを行ごとに比較します。

ファイルがソートされていない場合、結果は未定義です。
ファイル名に - を指定すると標準入力を読み込みます。

オプション:
  -1                      file1にのみある行を出力しない
  -2                      file2にのみある行を出力しない
  -3                      両方にある行を出力しない
  -i, --ignore-case       大文字小文字を無視して比較（GNU拡張）
  -z, --zero-terminated   行末をNUL文字として扱う（GNU拡張）
      --check-order       入力がソートされているかチェック（GNU拡張）
      --nocheck-order     入力のソートをチェックしない（GNU拡張）
      --output-delimiter=STR  列の区切り文字を指定（GNU拡張）
      --total             合計を表示（GNU拡張）
      --help              このヘルプを表示
      --version           バージョン情報を表示

デフォルトでは3列で出力されます:
  列1: file1にのみある行
  列2: file2にのみある行
  列3: 両方にある行

例:
  comm file1 file2              3列すべてを表示
  comm -12 file1 file2          共通行のみ表示（両方にある行）
  comm -23 file1 file2          file1のみにある行を表示
  comm -13 file1 file2          file2のみにある行を表示
  comm -3 file1 file2           どちらか一方にのみある行を表示

globパターン対応:
  comm sorted*.txt other.txt    パターンにマッチしたファイルを比較
"#
    );
}

fn print_version() {
    eprintln!("comm (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn contains_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn pathbuf_to_string(path: PathBuf) -> String {
    path.to_string_lossy().to_string()
}

/// Windowsではシェルが pathname expansion を行わないため、
/// POSIX系シェルに近い挙動になるよう未展開の引数だけをここで展開する。
/// マッチしない場合は一般的な POSIX シェル同様、リテラルをそのまま残す。
fn expand_positional_arg(arg: &str) -> Result<Vec<String>, String> {
    if arg == "-" || !contains_glob_meta(arg) {
        return Ok(vec![arg.to_string()]);
    }

    let mut matches: Vec<String> = glob(arg)
        .map_err(|e| format!("globパターンエラー: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("glob展開エラー: {}", e))?
        .into_iter()
        .map(pathbuf_to_string)
        .collect();

    matches.sort_unstable();

    if matches.is_empty() {
        Ok(vec![arg.to_string()])
    } else {
        Ok(matches)
    }
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
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
            config.suppress_col1 = true;
        } else if arg == "-2" {
            config.suppress_col2 = true;
        } else if arg == "-3" {
            config.suppress_col3 = true;
        } else if arg == "-i" || arg == "--ignore-case" {
            config.ignore_case = true;
        } else if arg == "-z" || arg == "--zero-terminated" {
            config.zero_terminated = true;
        } else if arg == "--check-order" {
            config.check_order = true;
        } else if arg == "--nocheck-order" {
            config.nocheck_order = true;
        } else if arg == "--total" {
            config.show_total = true;
        } else if arg == "--output-delimiter" {
            i += 1;
            if i >= args.len() {
                return Err("--output-delimiter にはデリミタが必要です".to_string());
            }
            config.output_delimiter = args[i].clone();
        } else if arg.starts_with("--output-delimiter=") {
            config.output_delimiter = arg[19..].to_string();
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                positional.push(args[j].clone());
            }
            break;
        } else if arg.starts_with('-') && arg.len() > 1 && arg != "-" {
            // 複合オプション（-12, -23, -123など）
            for c in arg[1..].chars() {
                match c {
                    '1' => config.suppress_col1 = true,
                    '2' => config.suppress_col2 = true,
                    '3' => config.suppress_col3 = true,
                    'i' => config.ignore_case = true,
                    'z' => config.zero_terminated = true,
                    _ => return Err(format!("不明なオプション: -{}", c)),
                }
            }
        } else {
            positional.extend(expand_positional_arg(arg)?);
        }

        i += 1;
    }

    if positional.len() < 2 {
        return Err("比較する2つのファイルを指定してください".to_string());
    }

    if positional.len() > 2 {
        return Err("引数が多すぎます".to_string());
    }

    config.file1 = positional[0].clone();
    config.file2 = positional[1].clone();

    Ok(config)
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
            let file = File::open(path).map_err(|e| format!("comm: '{}': {}", path, e))?;
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

/// 比較用に行を正規化
fn normalize_for_compare(line: &str, ignore_case: bool) -> String {
    let trimmed = line.trim_end_matches(&['\n', '\r', '\0'][..]);
    if ignore_case {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// comm処理
fn comm(config: &Config) -> Result<(), String> {
    let mut reader1 = InputReader::new(&config.file1)?;
    let mut reader2 = InputReader::new(&config.file2)?;

    let mut line1 = String::new();
    let mut line2 = String::new();

    let mut has_line1 = reader1
        .read_line(&mut line1, config.zero_terminated)
        .map_err(|e| format!("comm: 読み込みエラー: {}", e))?
        > 0;
    let mut has_line2 = reader2
        .read_line(&mut line2, config.zero_terminated)
        .map_err(|e| format!("comm: 読み込みエラー: {}", e))?
        > 0;

    let line_terminator = if config.zero_terminated { "\0" } else { "\n" };
    let delim = &config.output_delimiter;

    // カウンタ（--total用）
    let mut count1 = 0usize; // file1のみ
    let mut count2 = 0usize; // file2のみ
    let mut count3 = 0usize; // 共通

    // ソート順チェック用
    let mut prev1: Option<String> = None;
    let mut prev2: Option<String> = None;

    while has_line1 || has_line2 {
        let norm1 = if has_line1 {
            normalize_for_compare(&line1, config.ignore_case)
        } else {
            String::new()
        };
        let norm2 = if has_line2 {
            normalize_for_compare(&line2, config.ignore_case)
        } else {
            String::new()
        };

        // ソート順チェック
        if config.check_order && !config.nocheck_order {
            if let Some(ref prev) = prev1 {
                if has_line1 && norm1 < *prev {
                    eprintln!("comm: ファイル1がソートされていません");
                }
            }
            if let Some(ref prev) = prev2 {
                if has_line2 && norm2 < *prev {
                    eprintln!("comm: ファイル2がソートされていません");
                }
            }
        }

        let cmp = if !has_line1 {
            Ordering::Greater
        } else if !has_line2 {
            Ordering::Less
        } else {
            norm1.cmp(&norm2)
        };

        match cmp {
            Ordering::Less => {
                // file1のみ
                count1 += 1;
                if !config.suppress_col1 {
                    let content = line1.trim_end_matches(&['\n', '\r', '\0'][..]);
                    print!("{}{}", content, line_terminator);
                }
                prev1 = Some(norm1);
                has_line1 = reader1
                    .read_line(&mut line1, config.zero_terminated)
                    .map_err(|e| format!("comm: 読み込みエラー: {}", e))?
                    > 0;
            }
            Ordering::Greater => {
                // file2のみ
                count2 += 1;
                if !config.suppress_col2 {
                    let content = line2.trim_end_matches(&['\n', '\r', '\0'][..]);
                    // 列1が抑制されていなければインデント
                    let indent = if config.suppress_col1 { "" } else { delim };
                    print!("{}{}{}", indent, content, line_terminator);
                }
                prev2 = Some(norm2);
                has_line2 = reader2
                    .read_line(&mut line2, config.zero_terminated)
                    .map_err(|e| format!("comm: 読み込みエラー: {}", e))?
                    > 0;
            }
            Ordering::Equal => {
                // 共通
                count3 += 1;
                if !config.suppress_col3 {
                    let content = line1.trim_end_matches(&['\n', '\r', '\0'][..]);
                    // インデント計算
                    let indent1 = if config.suppress_col1 { "" } else { delim };
                    let indent2 = if config.suppress_col2 { "" } else { delim };
                    print!("{}{}{}{}", indent1, indent2, content, line_terminator);
                }
                prev1 = Some(norm1);
                prev2 = Some(norm2);
                has_line1 = reader1
                    .read_line(&mut line1, config.zero_terminated)
                    .map_err(|e| format!("comm: 読み込みエラー: {}", e))?
                    > 0;
                has_line2 = reader2
                    .read_line(&mut line2, config.zero_terminated)
                    .map_err(|e| format!("comm: 読み込みエラー: {}", e))?
                    > 0;
            }
        }
    }

    // 合計表示
    if config.show_total {
        let indent1 = if config.suppress_col1 { "" } else { delim };
        let indent2 = if config.suppress_col2 { "" } else { delim };

        if !config.suppress_col1 {
            print!("{}", count1);
        }
        if !config.suppress_col2 {
            print!("{}{}", indent1, count2);
        }
        if !config.suppress_col3 {
            print!("{}{}{}", indent1, indent2, count3);
        }
        print!("{}total{}", delim, line_terminator);
    }

    Ok(())
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("comm: {}", e);
            eprintln!("詳しくは 'comm --help' を参照してください");
            process::exit(1);
        }
    };

    if let Err(e) = comm(&config) {
        eprintln!("{}", e);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_argument_without_meta_is_unchanged() {
        assert_eq!(
            expand_positional_arg("file.txt").unwrap(),
            vec!["file.txt".to_string()]
        );
    }

    #[test]
    fn stdin_marker_is_never_glob_expanded() {
        assert_eq!(expand_positional_arg("-").unwrap(), vec!["-".to_string()]);
    }

    #[test]
    fn unmatched_glob_remains_literal() {
        let result = expand_positional_arg("__comm_unmatched__*.txt").unwrap();
        assert_eq!(result, vec!["__comm_unmatched__*.txt".to_string()]);
    }
}
