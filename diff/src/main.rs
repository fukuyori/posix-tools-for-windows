use std::cmp::max;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::process;

use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use glob::glob;

#[derive(Debug, Clone, Copy, PartialEq)]
enum OutputFormat {
    Normal,    // デフォルト（ed風）
    Context,   // -c: コンテキスト形式
    Unified,   // -u: ユニファイド形式
    Ed,        // -e: edスクリプト
    Rcs,       // -n: RCS形式
    SideBySide,// -y: 横並び
}

#[derive(Debug)]
struct Config {
    /// 最初のファイル
    file1: String,
    /// 2番目のファイル
    file2: String,
    /// 出力形式
    format: OutputFormat,
    /// コンテキスト行数（-C, -U）
    context_lines: usize,
    /// 空白の違いを無視（-b）
    ignore_space_change: bool,
    /// すべての空白を無視（-w）
    ignore_all_space: bool,
    /// 大文字小文字を無視（-i）
    ignore_case: bool,
    /// 空行を無視（-B）
    ignore_blank_lines: bool,
    /// タブをスペースに展開（-t）
    expand_tabs: bool,
    /// 行末の空白を無視
    ignore_trailing_space: bool,
    /// 再帰的比較（-r）
    recursive: bool,
    /// 同一ファイルを報告（-s）
    report_identical: bool,
    /// 異なることのみ報告（-q）
    brief: bool,
    /// 新規ファイルを空として扱う（-N）
    treat_absent_as_empty: bool,
    /// ラベル（--label）
    label1: Option<String>,
    label2: Option<String>,
    /// 横並び表示幅（-W）
    width: usize,
    /// 共通行を抑制（--suppress-common-lines）
    suppress_common: bool,
    /// カラー出力
    color: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            file1: String::new(),
            file2: String::new(),
            format: OutputFormat::Normal,
            context_lines: 3,
            ignore_space_change: false,
            ignore_all_space: false,
            ignore_case: false,
            ignore_blank_lines: false,
            expand_tabs: false,
            ignore_trailing_space: false,
            recursive: false,
            report_identical: false,
            brief: false,
            treat_absent_as_empty: false,
            label1: None,
            label2: None,
            width: 130,
            suppress_common: false,
            color: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: diff [オプション]... ファイル1 ファイル2
2つのファイルを行ごとに比較します。

出力形式:
  -c, -C NUM, --context[=NUM]   コンテキスト形式で出力（デフォルト3行）
  -u, -U NUM, --unified[=NUM]   ユニファイド形式で出力（デフォルト3行）
  -e, --ed                      edスクリプト形式で出力
  -n, --rcs                     RCS形式で出力
  -y, --side-by-side            横並びで出力
  -W, --width=NUM               横並び時の出力幅（デフォルト130）
      --suppress-common-lines   横並び時に共通行を表示しない

比較オプション:
  -i, --ignore-case             大文字小文字を無視
  -b, --ignore-space-change     空白の量の変化を無視
  -w, --ignore-all-space        すべての空白を無視
  -B, --ignore-blank-lines      空行を無視
  -Z, --ignore-trailing-space   行末の空白を無視

その他のオプション:
  -q, --brief                   ファイルが異なるかどうかのみ報告
  -s, --report-identical-files  同一ファイルを報告
  -r, --recursive               ディレクトリを再帰的に比較
  -N, --new-file                存在しないファイルを空として扱う
  -t, --expand-tabs             タブをスペースに展開
      --label=LABEL             ファイルラベルを指定
      --color[=WHEN]            カラー出力（auto, always, never）
      --help                    このヘルプを表示
      --version                 バージョン情報を表示

終了ステータス:
  0  ファイルが同一
  1  ファイルが異なる
  2  エラー発生

例:
  diff file1.txt file2.txt              通常形式で比較
  diff -u file1.txt file2.txt           ユニファイド形式で比較
  diff -c file1.txt file2.txt           コンテキスト形式で比較
  diff -y file1.txt file2.txt           横並びで比較
  diff -r dir1 dir2                     ディレクトリを再帰比較
  diff -u old.txt new.txt > patch.diff  パッチファイルを作成

globパターン対応:
  diff *.txt.old *.txt                  パターンにマッチしたファイルを比較
"#
    );
}

fn print_version() {
    eprintln!("diff (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

/// glob展開（単一ファイルのみ）
fn expand_glob_single(pattern: &str) -> Result<String, String> {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        let paths: Vec<String> = glob(pattern)
            .map_err(|e| format!("globパターンエラー: {}", e))?
            .filter_map(Result::ok)
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        if paths.is_empty() {
            Err(format!(
                "パターン '{}' に一致するファイルがありません",
                pattern
            ))
        } else if paths.len() > 1 {
            Err(format!(
                "パターン '{}' が複数のファイルに一致します",
                pattern
            ))
        } else {
            Ok(paths[0].clone())
        }
    } else {
        Ok(pattern.to_string())
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
        } else if arg == "-c" || arg == "--context" {
            config.format = OutputFormat::Context;
        } else if arg.starts_with("-C") {
            config.format = OutputFormat::Context;
            let num = if arg.len() > 2 {
                &arg[2..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("-C にはコンテキスト行数が必要です".to_string());
                }
                &args[i]
            };
            config.context_lines = num
                .parse()
                .map_err(|_| format!("無効なコンテキスト行数: '{}'", num))?;
        } else if arg.starts_with("--context=") {
            config.format = OutputFormat::Context;
            config.context_lines = arg[10..]
                .parse()
                .map_err(|_| format!("無効なコンテキスト行数: '{}'", &arg[10..]))?;
        } else if arg == "-u" || arg == "--unified" {
            config.format = OutputFormat::Unified;
        } else if arg.starts_with("-U") {
            config.format = OutputFormat::Unified;
            let num = if arg.len() > 2 {
                &arg[2..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("-U にはコンテキスト行数が必要です".to_string());
                }
                &args[i]
            };
            config.context_lines = num
                .parse()
                .map_err(|_| format!("無効なコンテキスト行数: '{}'", num))?;
        } else if arg.starts_with("--unified=") {
            config.format = OutputFormat::Unified;
            config.context_lines = arg[10..]
                .parse()
                .map_err(|_| format!("無効なコンテキスト行数: '{}'", &arg[10..]))?;
        } else if arg == "-e" || arg == "--ed" {
            config.format = OutputFormat::Ed;
        } else if arg == "-n" || arg == "--rcs" {
            config.format = OutputFormat::Rcs;
        } else if arg == "-y" || arg == "--side-by-side" {
            config.format = OutputFormat::SideBySide;
        } else if arg == "-W" || arg.starts_with("--width") {
            let num = if arg == "-W" {
                i += 1;
                if i >= args.len() {
                    return Err("-W には幅が必要です".to_string());
                }
                &args[i]
            } else if arg.starts_with("--width=") {
                &arg[8..]
            } else {
                i += 1;
                if i >= args.len() {
                    return Err("--width には幅が必要です".to_string());
                }
                &args[i]
            };
            config.width = num
                .parse()
                .map_err(|_| format!("無効な幅: '{}'", num))?;
        } else if arg == "--suppress-common-lines" {
            config.suppress_common = true;
        } else if arg == "-i" || arg == "--ignore-case" {
            config.ignore_case = true;
        } else if arg == "-b" || arg == "--ignore-space-change" {
            config.ignore_space_change = true;
        } else if arg == "-w" || arg == "--ignore-all-space" {
            config.ignore_all_space = true;
        } else if arg == "-B" || arg == "--ignore-blank-lines" {
            config.ignore_blank_lines = true;
        } else if arg == "-Z" || arg == "--ignore-trailing-space" {
            config.ignore_trailing_space = true;
        } else if arg == "-t" || arg == "--expand-tabs" {
            config.expand_tabs = true;
        } else if arg == "-q" || arg == "--brief" {
            config.brief = true;
        } else if arg == "-s" || arg == "--report-identical-files" {
            config.report_identical = true;
        } else if arg == "-r" || arg == "--recursive" {
            config.recursive = true;
        } else if arg == "-N" || arg == "--new-file" {
            config.treat_absent_as_empty = true;
        } else if arg.starts_with("--label=") {
            let label = arg[8..].to_string();
            if config.label1.is_none() {
                config.label1 = Some(label);
            } else {
                config.label2 = Some(label);
            }
        } else if arg == "--label" {
            i += 1;
            if i >= args.len() {
                return Err("--label にはラベルが必要です".to_string());
            }
            let label = args[i].clone();
            if config.label1.is_none() {
                config.label1 = Some(label);
            } else {
                config.label2 = Some(label);
            }
        } else if arg == "--color" || arg == "--color=always" || arg == "--color=auto" {
            config.color = true;
        } else if arg == "--color=never" {
            config.color = false;
        } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            // 複合オプション
            for c in arg[1..].chars() {
                match c {
                    'c' => config.format = OutputFormat::Context,
                    'u' => config.format = OutputFormat::Unified,
                    'e' => config.format = OutputFormat::Ed,
                    'n' => config.format = OutputFormat::Rcs,
                    'y' => config.format = OutputFormat::SideBySide,
                    'i' => config.ignore_case = true,
                    'b' => config.ignore_space_change = true,
                    'w' => config.ignore_all_space = true,
                    'B' => config.ignore_blank_lines = true,
                    'Z' => config.ignore_trailing_space = true,
                    't' => config.expand_tabs = true,
                    'q' => config.brief = true,
                    's' => config.report_identical = true,
                    'r' => config.recursive = true,
                    'N' => config.treat_absent_as_empty = true,
                    _ => return Err(format!("不明なオプション: -{}", c)),
                }
            }
        } else if arg == "--" {
            // オプション終了
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

    if positional.len() < 2 {
        return Err("比較する2つのファイルを指定してください".to_string());
    }

    if positional.len() > 2 {
        return Err("引数が多すぎます".to_string());
    }

    config.file1 = expand_glob_single(&positional[0])?;
    config.file2 = expand_glob_single(&positional[1])?;

    Ok(config)
}

/// エンコーディング自動検出
fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return encoding_rs::UTF_8;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16LE;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return encoding_rs::UTF_8;
    }

    let mut sjis_score = 0i32;
    let mut eucjp_score = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if (0x81..=0x9F).contains(&b) || (0xE0..=0xFC).contains(&b) {
            if i + 1 < bytes.len() {
                let b2 = bytes[i + 1];
                if (0x40..=0x7E).contains(&b2) || (0x80..=0xFC).contains(&b2) {
                    sjis_score += 1;
                    i += 2;
                    continue;
                }
            }
        }
        if (0xA1..=0xFE).contains(&b) {
            if i + 1 < bytes.len() && (0xA1..=0xFE).contains(&bytes[i + 1]) {
                eucjp_score += 1;
                i += 2;
                continue;
            }
        }
        i += 1;
    }

    if eucjp_score > sjis_score && eucjp_score > 0 {
        encoding_rs::EUC_JP
    } else if sjis_score > 0 {
        encoding_rs::SHIFT_JIS
    } else {
        encoding_rs::UTF_8
    }
}

/// ファイルを行に読み込む
fn read_lines(path: &str) -> Result<Vec<String>, String> {
    if path == "-" {
        let stdin = io::stdin();
        let reader = stdin.lock();
        return reader
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("標準入力の読み込みエラー: {}", e));
    }

    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("diff: {}: そのようなファイルやディレクトリはありません", path));
    }

    if p.is_dir() {
        return Err(format!("diff: {}: ディレクトリです", path));
    }

    let mut raw_bytes = Vec::new();
    File::open(p)
        .map_err(|e| format!("diff: {}: {}", path, e))?
        .read_to_end(&mut raw_bytes)
        .map_err(|e| format!("diff: {}: {}", path, e))?;

    let encoding = detect_encoding(&raw_bytes);
    let file = File::open(p).map_err(|e| format!("diff: {}: {}", path, e))?;
    let decoder = DecodeReaderBytesBuilder::new()
        .encoding(Some(encoding))
        .build(file);
    let reader = BufReader::new(decoder);

    reader
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("diff: {}: {}", path, e))
}

/// 比較用に行を正規化
fn normalize_line(line: &str, config: &Config) -> String {
    let mut result = line.to_string();

    if config.expand_tabs {
        result = result.replace('\t', "        ");
    }

    if config.ignore_trailing_space {
        result = result.trim_end().to_string();
    }

    if config.ignore_all_space {
        result = result.chars().filter(|c| !c.is_whitespace()).collect();
    } else if config.ignore_space_change {
        // 連続する空白を1つに
        let mut prev_space = false;
        result = result
            .chars()
            .filter_map(|c| {
                if c.is_whitespace() {
                    if prev_space {
                        None
                    } else {
                        prev_space = true;
                        Some(' ')
                    }
                } else {
                    prev_space = false;
                    Some(c)
                }
            })
            .collect();
        result = result.trim().to_string();
    }

    if config.ignore_case {
        result = result.to_lowercase();
    }

    result
}

/// LCS（最長共通部分列）を使った差分計算
#[derive(Debug, Clone, Copy, PartialEq)]
enum DiffOp {
    Equal,
    Insert,
    Delete,
}

#[derive(Debug, Clone)]
struct DiffLine {
    op: DiffOp,
    line1_num: Option<usize>,
    line2_num: Option<usize>,
    content: String,
}

fn compute_diff(lines1: &[String], lines2: &[String], config: &Config) -> Vec<DiffLine> {
    let n = lines1.len();
    let m = lines2.len();

    // 正規化した行で比較
    let norm1: Vec<String> = lines1.iter().map(|l| normalize_line(l, config)).collect();
    let norm2: Vec<String> = lines2.iter().map(|l| normalize_line(l, config)).collect();

    // 空行を無視する場合のフィルタリング
    let skip1: Vec<bool> = if config.ignore_blank_lines {
        norm1.iter().map(|l| l.trim().is_empty()).collect()
    } else {
        vec![false; n]
    };
    let skip2: Vec<bool> = if config.ignore_blank_lines {
        norm2.iter().map(|l| l.trim().is_empty()).collect()
    } else {
        vec![false; m]
    };

    // Myers差分アルゴリズム（簡略版：DP）
    // dp[i][j] = lines1[0..i]とlines2[0..j]のLCS長
    let mut dp = vec![vec![0usize; m + 1]; n + 1];

    for i in 1..=n {
        for j in 1..=m {
            if skip1[i - 1] || skip2[j - 1] {
                dp[i][j] = max(dp[i - 1][j], dp[i][j - 1]);
            } else if norm1[i - 1] == norm2[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = max(dp[i - 1][j], dp[i][j - 1]);
            }
        }
    }

    // バックトラックして差分を生成
    let mut diff = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && !skip1[i - 1] && !skip2[j - 1] && norm1[i - 1] == norm2[j - 1] {
            diff.push(DiffLine {
                op: DiffOp::Equal,
                line1_num: Some(i),
                line2_num: Some(j),
                content: lines1[i - 1].clone(),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            diff.push(DiffLine {
                op: DiffOp::Insert,
                line1_num: None,
                line2_num: Some(j),
                content: lines2[j - 1].clone(),
            });
            j -= 1;
        } else if i > 0 {
            diff.push(DiffLine {
                op: DiffOp::Delete,
                line1_num: Some(i),
                line2_num: None,
                content: lines1[i - 1].clone(),
            });
            i -= 1;
        }
    }

    diff.reverse();
    diff
}

/// 差分をハンクにグループ化
#[derive(Debug)]
struct Hunk {
    start1: usize,
    count1: usize,
    start2: usize,
    count2: usize,
    lines: Vec<DiffLine>,
}

fn group_into_hunks(diff: &[DiffLine], context: usize) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<Hunk> = None;
    let mut context_buffer: Vec<DiffLine> = Vec::new();
    let mut last_change_idx: Option<usize> = None;

    for (idx, line) in diff.iter().enumerate() {
        match line.op {
            DiffOp::Equal => {
                if let Some(ref mut hunk) = current_hunk {
                    // ハンク内の等しい行
                    hunk.lines.push(line.clone());

                    // コンテキスト行数を超えたらハンクを閉じるか確認
                    if let Some(last_idx) = last_change_idx {
                        if idx - last_idx > context * 2 {
                            // 次の変更との間隔が大きいのでハンクを閉じる
                            // 後ろのコンテキスト行を残す
                            let trim_count = hunk.lines.len() - context - (idx - last_idx - context);
                            if trim_count > 0 && hunk.lines.len() > context {
                                hunk.lines.truncate(hunk.lines.len() - (idx - last_idx - context));
                            }
                            
                            // カウントを再計算
                            hunk.count1 = hunk.lines.iter()
                                .filter(|l| l.op != DiffOp::Insert)
                                .count();
                            hunk.count2 = hunk.lines.iter()
                                .filter(|l| l.op != DiffOp::Delete)
                                .count();
                            
                            if let Some(hunk) = current_hunk.take() {
                                hunks.push(hunk);
                            }
                            context_buffer.clear();
                            last_change_idx = None;
                        }
                    }
                } else {
                    // ハンク外のコンテキストバッファ
                    context_buffer.push(line.clone());
                    if context_buffer.len() > context {
                        context_buffer.remove(0);
                    }
                }
            }
            DiffOp::Insert | DiffOp::Delete => {
                last_change_idx = Some(idx);
                
                match current_hunk.as_mut() {
                    Some(hunk) => {
                        hunk.lines.push(line.clone());
                    }
                    None => {
                        // 新しいハンクを開始
                        let start1 = context_buffer.first()
                            .and_then(|l| l.line1_num)
                            .unwrap_or_else(|| line.line1_num.unwrap_or(1));
                        let start2 = context_buffer.first()
                            .and_then(|l| l.line2_num)
                            .unwrap_or_else(|| line.line2_num.unwrap_or(1));
                        
                        let mut hunk = Hunk {
                            start1,
                            count1: 0,
                            start2,
                            count2: 0,
                            lines: context_buffer.clone(),
                        };
                        context_buffer.clear();
                        hunk.lines.push(line.clone());
                        current_hunk = Some(hunk);
                    }
                }
            }
        }
    }

    // 最後のハンクを閉じる
    if let Some(mut hunk) = current_hunk {
        // 末尾のコンテキスト行を制限
        let mut trailing_context = 0;
        for line in hunk.lines.iter().rev() {
            if line.op == DiffOp::Equal {
                trailing_context += 1;
            } else {
                break;
            }
        }
        if trailing_context > context {
            for _ in 0..(trailing_context - context) {
                hunk.lines.pop();
            }
        }
        
        hunk.count1 = hunk.lines.iter()
            .filter(|l| l.op != DiffOp::Insert)
            .count();
        hunk.count2 = hunk.lines.iter()
            .filter(|l| l.op != DiffOp::Delete)
            .count();
        
        hunks.push(hunk);
    }

    hunks
}

/// 通常形式で出力
fn output_normal(diff: &[DiffLine]) {
    let mut i = 0;
    while i < diff.len() {
        match diff[i].op {
            DiffOp::Equal => {
                i += 1;
            }
            DiffOp::Delete => {
                // 削除のみか、削除+挿入（変更）かを判断
                let del_start = i;
                while i < diff.len() && diff[i].op == DiffOp::Delete {
                    i += 1;
                }
                let del_end = i;

                let ins_start = i;
                while i < diff.len() && diff[i].op == DiffOp::Insert {
                    i += 1;
                }
                let ins_end = i;

                let del_lines: Vec<_> = diff[del_start..del_end].iter().collect();
                let ins_lines: Vec<_> = diff[ins_start..ins_end].iter().collect();

                if !del_lines.is_empty() && !ins_lines.is_empty() {
                    // 変更
                    let l1_start = del_lines.first()
                        .and_then(|l| l.line1_num)
                        .unwrap_or(1);
                    let l1_end = del_lines.last()
                        .and_then(|l| l.line1_num)
                        .unwrap_or(l1_start);
                    let l2_start = ins_lines.first()
                        .and_then(|l| l.line2_num)
                        .unwrap_or(1);
                    let l2_end = ins_lines.last()
                        .and_then(|l| l.line2_num)
                        .unwrap_or(l2_start);

                    if l1_start == l1_end && l2_start == l2_end {
                        println!("{}c{}", l1_start, l2_start);
                    } else if l1_start == l1_end {
                        println!("{}c{},{}", l1_start, l2_start, l2_end);
                    } else if l2_start == l2_end {
                        println!("{},{}c{}", l1_start, l1_end, l2_start);
                    } else {
                        println!("{},{}c{},{}", l1_start, l1_end, l2_start, l2_end);
                    }

                    for line in &del_lines {
                        println!("< {}", line.content);
                    }
                    println!("---");
                    for line in &ins_lines {
                        println!("> {}", line.content);
                    }
                } else if !del_lines.is_empty() {
                    // 削除のみ
                    let l1_start = del_lines.first()
                        .and_then(|l| l.line1_num)
                        .unwrap_or(1);
                    let l1_end = del_lines.last()
                        .and_then(|l| l.line1_num)
                        .unwrap_or(l1_start);
                    let l2_pos = if ins_start > 0 {
                        diff[ins_start - 1].line2_num.unwrap_or(0)
                    } else {
                        0
                    };

                    if l1_start == l1_end {
                        println!("{}d{}", l1_start, l2_pos);
                    } else {
                        println!("{},{}d{}", l1_start, l1_end, l2_pos);
                    }

                    for line in &del_lines {
                        println!("< {}", line.content);
                    }
                }
            }
            DiffOp::Insert => {
                let ins_start = i;
                while i < diff.len() && diff[i].op == DiffOp::Insert {
                    i += 1;
                }
                let ins_end = i;

                let ins_lines: Vec<_> = diff[ins_start..ins_end].iter().collect();
                let l2_start = ins_lines.first()
                    .and_then(|l| l.line2_num)
                    .unwrap_or(1);
                let l2_end = ins_lines.last()
                    .and_then(|l| l.line2_num)
                    .unwrap_or(l2_start);
                let l1_pos = if ins_start > 0 {
                    diff[ins_start - 1].line1_num.unwrap_or(0)
                } else {
                    0
                };

                if l2_start == l2_end {
                    println!("{}a{}", l1_pos, l2_start);
                } else {
                    println!("{}a{},{}", l1_pos, l2_start, l2_end);
                }

                for line in &ins_lines {
                    println!("> {}", line.content);
                }
            }
        }
    }
}

/// ユニファイド形式で出力
fn output_unified(hunks: &[Hunk], config: &Config) {
    let label1 = config.label1.as_ref().unwrap_or(&config.file1);
    let label2 = config.label2.as_ref().unwrap_or(&config.file2);

    println!("--- {}", label1);
    println!("+++ {}", label2);

    for hunk in hunks {
        println!(
            "@@ -{},{} +{},{} @@",
            hunk.start1, hunk.count1, hunk.start2, hunk.count2
        );

        for line in &hunk.lines {
            match line.op {
                DiffOp::Equal => println!(" {}", line.content),
                DiffOp::Delete => println!("-{}", line.content),
                DiffOp::Insert => println!("+{}", line.content),
            }
        }
    }
}

/// コンテキスト形式で出力
fn output_context(hunks: &[Hunk], config: &Config) {
    let label1 = config.label1.as_ref().unwrap_or(&config.file1);
    let label2 = config.label2.as_ref().unwrap_or(&config.file2);

    println!("*** {}", label1);
    println!("--- {}", label2);

    for hunk in hunks {
        println!("***************");
        
        // file1側
        println!("*** {},{} ****", hunk.start1, hunk.start1 + hunk.count1 - 1);
        let has_changes1 = hunk.lines.iter().any(|l| l.op == DiffOp::Delete);
        if has_changes1 {
            for line in &hunk.lines {
                match line.op {
                    DiffOp::Equal => println!("  {}", line.content),
                    DiffOp::Delete => println!("- {}", line.content),
                    DiffOp::Insert => {}
                }
            }
        }

        // file2側
        println!("--- {},{} ----", hunk.start2, hunk.start2 + hunk.count2 - 1);
        let has_changes2 = hunk.lines.iter().any(|l| l.op == DiffOp::Insert);
        if has_changes2 {
            for line in &hunk.lines {
                match line.op {
                    DiffOp::Equal => println!("  {}", line.content),
                    DiffOp::Insert => println!("+ {}", line.content),
                    DiffOp::Delete => {}
                }
            }
        }
    }
}

/// edスクリプト形式で出力
fn output_ed(diff: &[DiffLine]) {
    // edスクリプトは逆順で出力（行番号がずれないように）
    let mut changes: Vec<(usize, usize, DiffOp, Vec<String>)> = Vec::new();
    let mut i = 0;

    while i < diff.len() {
        match diff[i].op {
            DiffOp::Equal => {
                i += 1;
            }
            DiffOp::Delete => {
                let start = diff[i].line1_num.unwrap_or(1);
                let mut end = start;
                let mut j = i + 1;
                while j < diff.len() && diff[j].op == DiffOp::Delete {
                    end = diff[j].line1_num.unwrap_or(end);
                    j += 1;
                }
                
                // 続く挿入をチェック
                let mut inserts = Vec::new();
                while j < diff.len() && diff[j].op == DiffOp::Insert {
                    inserts.push(diff[j].content.clone());
                    j += 1;
                }

                if !inserts.is_empty() {
                    changes.push((start, end, DiffOp::Equal, inserts)); // 変更
                } else {
                    changes.push((start, end, DiffOp::Delete, Vec::new()));
                }
                i = j;
            }
            DiffOp::Insert => {
                let pos = if i > 0 {
                    diff[i - 1].line1_num.unwrap_or(0)
                } else {
                    0
                };
                let mut inserts = Vec::new();
                while i < diff.len() && diff[i].op == DiffOp::Insert {
                    inserts.push(diff[i].content.clone());
                    i += 1;
                }
                changes.push((pos, pos, DiffOp::Insert, inserts));
            }
        }
    }

    // 逆順で出力
    for (start, end, op, lines) in changes.into_iter().rev() {
        match op {
            DiffOp::Delete => {
                if start == end {
                    println!("{}d", start);
                } else {
                    println!("{},{}d", start, end);
                }
            }
            DiffOp::Insert => {
                println!("{}a", start);
                for line in &lines {
                    println!("{}", line);
                }
                println!(".");
            }
            DiffOp::Equal => {
                // 変更（削除+挿入）
                if start == end {
                    println!("{}c", start);
                } else {
                    println!("{},{}c", start, end);
                }
                for line in &lines {
                    println!("{}", line);
                }
                println!(".");
            }
        }
    }
}

/// RCS形式で出力
fn output_rcs(diff: &[DiffLine]) {
    let mut i = 0;

    while i < diff.len() {
        match diff[i].op {
            DiffOp::Equal => {
                i += 1;
            }
            DiffOp::Delete => {
                let start = diff[i].line1_num.unwrap_or(1);
                let mut count = 0;
                while i < diff.len() && diff[i].op == DiffOp::Delete {
                    count += 1;
                    i += 1;
                }
                println!("d{} {}", start, count);
            }
            DiffOp::Insert => {
                let pos = if i > 0 {
                    diff[i - 1].line1_num.unwrap_or(0)
                } else {
                    0
                };
                let mut lines = Vec::new();
                while i < diff.len() && diff[i].op == DiffOp::Insert {
                    lines.push(diff[i].content.clone());
                    i += 1;
                }
                println!("a{} {}", pos, lines.len());
                for line in &lines {
                    println!("{}", line);
                }
            }
        }
    }
}

/// 横並び形式で出力
fn output_side_by_side(diff: &[DiffLine], config: &Config) {
    let col_width = (config.width - 3) / 2;

    // 差分を再構成（削除と挿入をペアにする）
    let mut i = 0;
    while i < diff.len() {
        if config.suppress_common && diff[i].op == DiffOp::Equal {
            i += 1;
            continue;
        }

        match diff[i].op {
            DiffOp::Equal => {
                let content = &diff[i].content;
                let left = if content.len() > col_width {
                    &content[..col_width]
                } else {
                    content
                };
                println!("{:width$}   {}", left, content, width = col_width);
                i += 1;
            }
            DiffOp::Delete => {
                // 削除の後に挿入があればペアにする
                let del_content = &diff[i].content;
                let left = if del_content.len() > col_width {
                    &del_content[..col_width]
                } else {
                    del_content
                };
                
                // 次が挿入かチェック
                if i + 1 < diff.len() && diff[i + 1].op == DiffOp::Insert {
                    let ins_content = &diff[i + 1].content;
                    println!("{:width$} | {}", left, ins_content, width = col_width);
                    i += 2;
                } else {
                    println!("{:width$} <", left, width = col_width);
                    i += 1;
                }
            }
            DiffOp::Insert => {
                let ins_content = &diff[i].content;
                println!("{:width$} > {}", "", ins_content, width = col_width);
                i += 1;
            }
        }
    }
}

/// ディレクトリの再帰比較
fn compare_directories(dir1: &str, dir2: &str, config: &Config) -> i32 {
    let mut exit_code = 0;

    let entries1: Vec<_> = fs::read_dir(dir1)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();

    let entries2: Vec<_> = fs::read_dir(dir2)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();

    let mut all_entries: Vec<_> = entries1.iter().chain(entries2.iter()).cloned().collect();
    all_entries.sort();
    all_entries.dedup();

    for entry in all_entries {
        let path1 = format!("{}/{}", dir1, entry);
        let path2 = format!("{}/{}", dir2, entry);

        let exists1 = Path::new(&path1).exists();
        let exists2 = Path::new(&path2).exists();

        if !exists1 && exists2 {
            if config.treat_absent_as_empty {
                let code = compare_files(&path1, &path2, config);
                if code > exit_code {
                    exit_code = code;
                }
            } else {
                println!("{} のみに存在: {}", dir2, entry);
                exit_code = 1;
            }
        } else if exists1 && !exists2 {
            if config.treat_absent_as_empty {
                let code = compare_files(&path1, &path2, config);
                if code > exit_code {
                    exit_code = code;
                }
            } else {
                println!("{} のみに存在: {}", dir1, entry);
                exit_code = 1;
            }
        } else {
            let is_dir1 = Path::new(&path1).is_dir();
            let is_dir2 = Path::new(&path2).is_dir();

            if is_dir1 && is_dir2 {
                let code = compare_directories(&path1, &path2, config);
                if code > exit_code {
                    exit_code = code;
                }
            } else if !is_dir1 && !is_dir2 {
                let code = compare_files(&path1, &path2, config);
                if code > exit_code {
                    exit_code = code;
                }
            } else {
                eprintln!("diff: {}: 種類が異なります", entry);
                exit_code = 1;
            }
        }
    }

    exit_code
}

/// 2つのファイルを比較
fn compare_files(file1: &str, file2: &str, config: &Config) -> i32 {
    let lines1 = if !Path::new(file1).exists() && config.treat_absent_as_empty {
        Vec::new()
    } else {
        match read_lines(file1) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{}", e);
                return 2;
            }
        }
    };

    let lines2 = if !Path::new(file2).exists() && config.treat_absent_as_empty {
        Vec::new()
    } else {
        match read_lines(file2) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{}", e);
                return 2;
            }
        }
    };

    let diff = compute_diff(&lines1, &lines2, config);
    let has_diff = diff.iter().any(|l| l.op != DiffOp::Equal);

    if config.brief {
        if has_diff {
            println!("ファイル {} と {} は異なります", file1, file2);
            return 1;
        } else {
            if config.report_identical {
                println!("ファイル {} と {} は同一です", file1, file2);
            }
            return 0;
        }
    }

    if !has_diff {
        if config.report_identical {
            println!("ファイル {} と {} は同一です", file1, file2);
        }
        return 0;
    }

    match config.format {
        OutputFormat::Normal => output_normal(&diff),
        OutputFormat::Unified => {
            let hunks = group_into_hunks(&diff, config.context_lines);
            output_unified(&hunks, config);
        }
        OutputFormat::Context => {
            let hunks = group_into_hunks(&diff, config.context_lines);
            output_context(&hunks, config);
        }
        OutputFormat::Ed => output_ed(&diff),
        OutputFormat::Rcs => output_rcs(&diff),
        OutputFormat::SideBySide => output_side_by_side(&diff, config),
    }

    1
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("diff: {}", e);
            eprintln!("詳しくは 'diff --help' を参照してください");
            process::exit(2);
        }
    };

    let is_dir1 = Path::new(&config.file1).is_dir();
    let is_dir2 = Path::new(&config.file2).is_dir();

    let exit_code = if is_dir1 && is_dir2 {
        if config.recursive {
            compare_directories(&config.file1, &config.file2, &config)
        } else {
            eprintln!("diff: {}: ディレクトリです", config.file1);
            2
        }
    } else if is_dir1 || is_dir2 {
        // 片方がディレクトリの場合
        let (dir, file) = if is_dir1 {
            (&config.file1, &config.file2)
        } else {
            (&config.file2, &config.file1)
        };
        let filename = match Path::new(file).file_name() {
            Some(name) => name.to_string_lossy(),
            None => {
                eprintln!("diff: {}: 無効なファイル名です", file);
                process::exit(2);
            }
        };
        let file_in_dir = format!("{}/{}", dir, filename);
        
        if is_dir1 {
            compare_files(&file_in_dir, file, &config)
        } else {
            compare_files(file, &file_in_dir, &config)
        }
    } else {
        compare_files(&config.file1, &config.file2, &config)
    };

    process::exit(exit_code);
}
