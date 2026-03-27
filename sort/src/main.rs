use std::cmp::Ordering;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process;

use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use glob::{glob_with, MatchOptions};

/// ソートのキー指定
#[derive(Clone, Debug)]
struct SortKey {
    /// 開始フィールド（1始まり）
    start_field: usize,
    /// 開始フィールド内の文字位置（1始まり、0は未指定）
    start_char: usize,
    /// 終了フィールド（1始まり、0は行末まで）
    end_field: usize,
    /// 終了フィールド内の文字位置（1始まり、0は未指定）
    end_char: usize,
    /// このキーに適用するオプション
    options: KeyOptions,
}

#[derive(Clone, Debug, Default)]
struct KeyOptions {
    numeric: bool,               // -n: 数値ソート
    human_numeric: bool,         // -h: 人間可読数値（1K, 1M等）
    general_numeric: bool,       // -g: 一般数値（浮動小数点）
    month: bool,                 // -M: 月名ソート
    reverse: bool,               // -r: 逆順
    ignore_case: bool,           // -f: 大文字小文字無視
    dictionary: bool,            // -d: 辞書順（英数字と空白のみ）
    ignore_nonprinting: bool,    // -i: 非印字文字無視
    ignore_leading_blanks: bool, // -b: 先頭空白無視
    version: bool,               // -V: バージョン番号ソート
}

#[derive(Clone, Debug)]
struct Config {
    /// 入力ファイル
    files: Vec<String>,
    /// ソートキー
    keys: Vec<SortKey>,
    /// グローバルオプション
    global_options: KeyOptions,
    /// フィールド区切り文字
    field_separator: Option<char>,
    /// 出力ファイル（-o）
    output_file: Option<String>,
    /// 重複削除（-u）
    unique: bool,
    /// ソート済み確認（-c）
    check: bool,
    /// 厳密なソート済み確認（-C）
    check_silent: bool,
    /// 安定ソート（-s）
    stable: bool,
    /// マージのみ（-m）
    merge: bool,
    /// ゼロ終端（-z）
    zero_terminated: bool,
    /// デバッグモード（--debug）
    debug: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            files: Vec::new(),
            keys: Vec::new(),
            global_options: KeyOptions::default(),
            field_separator: None,
            output_file: None,
            unique: false,
            check: false,
            check_silent: false,
            stable: false,
            merge: false,
            zero_terminated: false,
            debug: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: sort [オプション]... [ファイル]...
ファイルの全行を連結してソートし、標準出力に書き出します。
ファイルが指定されない場合、または - の場合は標準入力を読み込みます。

順序オプション:
  -b, --ignore-leading-blanks  先頭の空白を無視
  -d, --dictionary-order       空白と英数字のみ比較
  -f, --ignore-case            大文字小文字を区別しない
  -g, --general-numeric-sort   一般的な数値としてソート
  -i, --ignore-nonprinting     印字可能文字のみ比較
  -M, --month-sort             月名でソート（JAN < FEB < ... < DEC）
  -h, --human-numeric-sort     人間可読数値でソート（2K, 1G等）
  -n, --numeric-sort           文字列を数値としてソート
  -R, --random-sort            ランダムにソート（未実装）
  -V, --version-sort           バージョン番号としてソート
  -r, --reverse                逆順にソート

その他のオプション:
  -c, --check                  ソート済みか確認、ソートしない
  -C, --check=silent           -c と同様だがエラーメッセージを出さない
  -k, --key=KEYDEF             キー定義に従ってソート
  -m, --merge                  ソート済みファイルをマージ、ソートしない
  -o, --output=FILE            結果をFILEに出力
  -s, --stable                 安定ソート（同値の順序を保持）
  -t, --field-separator=SEP    フィールド区切り文字をSEPに設定
  -u, --unique                 重複行を削除（-cと併用時は厳密な順序を確認）
  -z, --zero-terminated        行区切りを改行ではなくNULに
      --debug                  ソートに使用したキーを表示
      --help                   このヘルプを表示
      --version                バージョン情報を表示

KEYDEF は F[.C][OPTS][,F[.C][OPTS]] の形式で、ソート位置を指定します。
F はフィールド番号、C はフィールド内の文字位置（共に1始まり）。
終了位置が省略されると行末まで。OPTS は bdfiMnhrV の組み合わせで、
グローバルオプションを上書きします。-b は開始/終了位置に個別適用。

例:
  sort -t: -k3,3n /etc/passwd   3番目のフィールドを数値としてソート
  sort -k1,1 -k2,2n             1番目を文字、2番目を数値でソート
  sort -n -r                    数値として逆順ソート

globパターン対応:
  sort *.txt                    カレントディレクトリの全txtファイルをソート

Windows版の注意:
  Windows上でもPOSIXに近い挙動を優先し、比較時は既定で大文字小文字を区別します。
  ただしファイル名の glob 展開だけは Windows に合わせて大文字小文字を区別しません。
"#
    );
}

fn print_version() {
    eprintln!("sort (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

/// キー定義をパース（例: "1.2,3.4nbr" -> SortKey）
fn parse_key_def(s: &str, global_opts: &KeyOptions) -> Result<SortKey, String> {
    let mut key = SortKey {
        start_field: 1,
        start_char: 0,
        end_field: 0,
        end_char: 0,
        options: global_opts.clone(),
    };

    // カンマで開始と終了を分割
    let parts: Vec<&str> = s.splitn(2, ',').collect();

    // 開始位置をパース
    let (start_pos, start_opts) = parse_position(parts[0])?;
    key.start_field = start_pos.0;
    key.start_char = start_pos.1;
    apply_key_options(&mut key.options, &start_opts);

    // 終了位置をパース
    if parts.len() > 1 {
        let (end_pos, end_opts) = parse_position(parts[1])?;
        key.end_field = end_pos.0;
        key.end_char = end_pos.1;
        apply_key_options(&mut key.options, &end_opts);
    }

    if key.start_field == 0 {
        return Err("フィールド番号は1以上である必要があります".to_string());
    }

    Ok(key)
}

/// 位置指定をパース（例: "3.2nr" -> ((3, 2), "nr")）
fn parse_position(s: &str) -> Result<((usize, usize), String), String> {
    let mut field = 0usize;
    let mut char_pos = 0usize;
    let mut opts = String::new();
    let mut in_field = true;
    let mut num_str = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            if opts.is_empty() {
                num_str.push(c);
            } else {
                return Err(format!("無効なキー定義: {}", s));
            }
        } else if c == '.' {
            if in_field && !num_str.is_empty() {
                field = num_str
                    .parse()
                    .map_err(|_| format!("無効なフィールド番号: {}", num_str))?;
                num_str.clear();
                in_field = false;
            } else {
                return Err(format!("無効なキー定義: {}", s));
            }
        } else if "bdfgiMnhrRV".contains(c) {
            if !num_str.is_empty() {
                if in_field {
                    field = num_str
                        .parse()
                        .map_err(|_| format!("無効なフィールド番号: {}", num_str))?;
                } else {
                    char_pos = num_str
                        .parse()
                        .map_err(|_| format!("無効な文字位置: {}", num_str))?;
                }
                num_str.clear();
            }
            opts.push(c);
        } else {
            return Err(format!("無効な文字 '{}' がキー定義に含まれています", c));
        }
    }

    if !num_str.is_empty() {
        if in_field {
            field = num_str
                .parse()
                .map_err(|_| format!("無効なフィールド番号: {}", num_str))?;
        } else {
            char_pos = num_str
                .parse()
                .map_err(|_| format!("無効な文字位置: {}", num_str))?;
        }
    }

    Ok(((field, char_pos), opts))
}

/// オプション文字列をKeyOptionsに適用
fn apply_key_options(opts: &mut KeyOptions, opt_str: &str) {
    for c in opt_str.chars() {
        match c {
            'b' => opts.ignore_leading_blanks = true,
            'd' => opts.dictionary = true,
            'f' => opts.ignore_case = true,
            'g' => opts.general_numeric = true,
            'h' => opts.human_numeric = true,
            'i' => opts.ignore_nonprinting = true,
            'M' => opts.month = true,
            'n' => opts.numeric = true,
            'r' => opts.reverse = true,
            'V' => opts.version = true,
            _ => {}
        }
    }
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut config = Config::default();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "--debug" {
            config.debug = true;
        } else if arg == "-b" || arg == "--ignore-leading-blanks" {
            config.global_options.ignore_leading_blanks = true;
        } else if arg == "-d" || arg == "--dictionary-order" {
            config.global_options.dictionary = true;
        } else if arg == "-f" || arg == "--ignore-case" {
            config.global_options.ignore_case = true;
        } else if arg == "-g" || arg == "--general-numeric-sort" {
            config.global_options.general_numeric = true;
        } else if arg == "-h" || arg == "--human-numeric-sort" {
            config.global_options.human_numeric = true;
        } else if arg == "-i" || arg == "--ignore-nonprinting" {
            config.global_options.ignore_nonprinting = true;
        } else if arg == "-M" || arg == "--month-sort" {
            config.global_options.month = true;
        } else if arg == "-n" || arg == "--numeric-sort" {
            config.global_options.numeric = true;
        } else if arg == "-r" || arg == "--reverse" {
            config.global_options.reverse = true;
        } else if arg == "-V" || arg == "--version-sort" {
            config.global_options.version = true;
        } else if arg == "-c" || arg == "--check" {
            config.check = true;
        } else if arg == "-C" || arg == "--check=silent" || arg == "--check=quiet" {
            config.check_silent = true;
        } else if arg == "-m" || arg == "--merge" {
            config.merge = true;
        } else if arg == "-s" || arg == "--stable" {
            config.stable = true;
        } else if arg == "-u" || arg == "--unique" {
            config.unique = true;
        } else if arg == "-z" || arg == "--zero-terminated" {
            config.zero_terminated = true;
        } else if arg == "-k" || arg.starts_with("--key=") {
            let key_def = if arg == "-k" {
                i += 1;
                if i >= args.len() {
                    return Err("-k オプションにはキー定義が必要です".to_string());
                }
                &args[i]
            } else {
                &arg[6..]
            };
            let key = parse_key_def(key_def, &config.global_options)?;
            config.keys.push(key);
        } else if arg == "-t" || arg.starts_with("--field-separator=") {
            let sep = if arg == "-t" {
                i += 1;
                if i >= args.len() {
                    return Err("-t オプションには区切り文字が必要です".to_string());
                }
                &args[i]
            } else {
                &arg[18..]
            };
            if sep.is_empty() {
                return Err("区切り文字が空です".to_string());
            }
            config.field_separator =
                Some(sep.chars().next().expect("sep is not empty after check"));
        } else if arg == "-o" || arg.starts_with("--output=") {
            let output = if arg == "-o" {
                i += 1;
                if i >= args.len() {
                    return Err("-o オプションには出力ファイル名が必要です".to_string());
                }
                args[i].clone()
            } else {
                arg[9..].to_string()
            };
            config.output_file = Some(output);
        } else if arg.starts_with("-t") && arg.len() > 2 && !arg.starts_with("--") {
            // -t: 形式（区切り文字が直接続く）
            let sep = &arg[2..];
            if sep.is_empty() {
                return Err("区切り文字が空です".to_string());
            }
            config.field_separator =
                Some(sep.chars().next().expect("sep is not empty after check"));
        } else if arg.starts_with("-k") && arg.len() > 2 && !arg.starts_with("--") {
            // -k1,2n 形式（キー定義が直接続く）
            let key_def = &arg[2..];
            let key = parse_key_def(key_def, &config.global_options)?;
            config.keys.push(key);
        } else if arg.starts_with("-o") && arg.len() > 2 && !arg.starts_with("--") {
            // -oFILE 形式
            let output = &arg[2..];
            config.output_file = Some(output.to_string());
        } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            // 複合オプション（-nr 等）
            for c in arg[1..].chars() {
                match c {
                    'b' => config.global_options.ignore_leading_blanks = true,
                    'd' => config.global_options.dictionary = true,
                    'f' => config.global_options.ignore_case = true,
                    'g' => config.global_options.general_numeric = true,
                    'h' => config.global_options.human_numeric = true,
                    'i' => config.global_options.ignore_nonprinting = true,
                    'M' => config.global_options.month = true,
                    'n' => config.global_options.numeric = true,
                    'r' => config.global_options.reverse = true,
                    'V' => config.global_options.version = true,
                    'c' => config.check = true,
                    'C' => config.check_silent = true,
                    'm' => config.merge = true,
                    's' => config.stable = true,
                    'u' => config.unique = true,
                    'z' => config.zero_terminated = true,
                    _ => return Err(format!("不明なオプション: -{}", c)),
                }
            }
        } else if arg == "-" {
            config.files.push("-".to_string());
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("不明なオプション: {}", arg));
        } else {
            // ファイル名（glob展開）
            let expanded = expand_glob(arg)?;
            config.files.extend(expanded);
        }

        i += 1;
    }

    // ファイルが指定されなければ標準入力
    if config.files.is_empty() {
        config.files.push("-".to_string());
    }

    // キーが指定されなければ行全体をキーとする
    if config.keys.is_empty() {
        config.keys.push(SortKey {
            start_field: 1,
            start_char: 0,
            end_field: 0,
            end_char: 0,
            options: config.global_options.clone(),
        });
    }

    Ok(config)
}

/// glob展開
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        let options = MatchOptions {
            case_sensitive: false,
            require_literal_separator: false,
            require_literal_leading_dot: false,
        };

        let mut paths: Vec<String> = glob_with(pattern, options)
            .map_err(|e| format!("globパターンエラー: {}", e))?
            .filter_map(Result::ok)
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        if paths.is_empty() {
            // 一致しない場合はリテラルのまま扱い、シェル未展開時のPOSIX系sortに近づける。
            Ok(vec![pattern.to_string()])
        } else {
            paths.sort_by(|a, b| compare_text(a, b, false));
            Ok(paths)
        }
    } else {
        Ok(vec![pattern.to_string()])
    }
}

fn compare_text(a: &str, b: &str, ignore_case: bool) -> Ordering {
    if ignore_case {
        let a_folded = a.to_lowercase();
        let b_folded = b.to_lowercase();
        a_folded.cmp(&b_folded)
    } else {
        a.cmp(b)
    }
}

/// ファイルから行を読み込む（エンコーディング自動検出）
fn read_lines(filename: &str, zero_term: bool) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();

    if filename == "-" {
        let stdin = io::stdin();
        let reader = stdin.lock();
        read_lines_from_reader(reader, zero_term, &mut lines)?;
    } else {
        let path = Path::new(filename);
        if !path.exists() {
            return Err(format!(
                "sort: '{}': そのようなファイルやディレクトリはありません",
                filename
            ));
        }

        let file = File::open(path).map_err(|e| format!("sort: '{}': {}", filename, e))?;

        // エンコーディング自動検出
        let mut raw_bytes = Vec::new();
        BufReader::new(&file)
            .read_to_end(&mut raw_bytes)
            .map_err(|e| format!("sort: '{}': 読み込みエラー: {}", filename, e))?;

        let encoding = detect_encoding(&raw_bytes);

        let file = File::open(path).map_err(|e| format!("sort: '{}': {}", filename, e))?;

        let decoder = DecodeReaderBytesBuilder::new()
            .encoding(Some(encoding))
            .build(file);
        let reader = BufReader::new(decoder);

        read_lines_from_reader(reader, zero_term, &mut lines)?;
    }

    Ok(lines)
}

fn read_lines_from_reader<R: BufRead>(
    reader: R,
    zero_term: bool,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    if zero_term {
        let mut buffer = Vec::new();
        for byte_result in reader.bytes() {
            let byte = byte_result.map_err(|e| format!("読み込みエラー: {}", e))?;
            if byte == 0 {
                let line = String::from_utf8_lossy(&buffer).to_string();
                lines.push(line);
                buffer.clear();
            } else {
                buffer.push(byte);
            }
        }
        if !buffer.is_empty() {
            let line = String::from_utf8_lossy(&buffer).to_string();
            lines.push(line);
        }
    } else {
        for line_result in reader.lines() {
            let line = line_result.map_err(|e| format!("読み込みエラー: {}", e))?;
            lines.push(line);
        }
    }
    Ok(())
}

/// エンコーディング自動検出（UTF-8, Shift_JIS, EUC-JP）
fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    // BOMチェック
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return encoding_rs::UTF_8;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16LE;
    }

    // UTF-8として有効かチェック
    if std::str::from_utf8(bytes).is_ok() {
        return encoding_rs::UTF_8;
    }

    // 日本語エンコーディング検出（簡易版）
    let mut sjis_score = 0i32;
    let mut eucjp_score = 0i32;

    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        // Shift_JIS判定
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

        // EUC-JP判定
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

/// フィールド分割
fn split_fields(line: &str, separator: Option<char>) -> Vec<&str> {
    match separator {
        Some(sep) => line.split(sep).collect(),
        None => line.split_whitespace().collect(),
    }
}

/// キーに対応する部分文字列を抽出
fn extract_key<'a>(line: &'a str, key: &SortKey, separator: Option<char>) -> &'a str {
    let fields = split_fields(line, separator);

    if fields.is_empty() || key.start_field == 0 {
        return line;
    }

    let start_idx = key.start_field - 1;
    if start_idx >= fields.len() {
        return "";
    }

    // 終了フィールドが未指定または行末を超える場合
    let end_idx = if key.end_field == 0 || key.end_field > fields.len() {
        fields.len()
    } else {
        key.end_field
    };

    if start_idx >= end_idx {
        return "";
    }

    // 単一フィールドの場合
    if end_idx - start_idx == 1 {
        let field = fields[start_idx];
        let chars: Vec<char> = field.chars().collect();

        let start_char = if key.start_char > 0 {
            (key.start_char - 1).min(chars.len())
        } else {
            0
        };

        let end_char = if key.end_char > 0 {
            key.end_char.min(chars.len())
        } else {
            chars.len()
        };

        if start_char >= end_char {
            return "";
        }

        // 元の文字列内の位置を計算
        let byte_start: usize = chars[..start_char].iter().map(|c| c.len_utf8()).sum();
        let byte_end: usize = chars[..end_char].iter().map(|c| c.len_utf8()).sum();

        // フィールドの開始位置を見つける
        if let Some(field_start) = line.find(field) {
            return &line[field_start + byte_start..field_start + byte_end];
        }

        return field;
    }

    // 複数フィールドにまたがる場合
    // 元の文字列内でフィールド範囲を特定して返す
    let first_field = fields[start_idx];
    let last_field = fields[end_idx - 1];

    if let Some(start_pos) = line.find(first_field) {
        if let Some(last_pos) = line.rfind(last_field) {
            let end_pos = last_pos + last_field.len();
            return &line[start_pos..end_pos];
        }
    }

    first_field
}

/// 先頭空白を除去
fn trim_leading_blanks(s: &str) -> &str {
    s.trim_start()
}

/// 辞書順用にフィルタ（英数字と空白のみ）
fn filter_dictionary(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect()
}

/// 非印字文字を除去
fn filter_nonprinting(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// 数値としてパース（先頭の数値部分のみ）
fn parse_numeric(s: &str) -> f64 {
    let s = s.trim();

    // 符号と数値部分を抽出
    let mut chars = s.chars().peekable();
    let mut num_str = String::new();

    // 先頭の空白をスキップ
    while chars.peek().map_or(false, |c| c.is_whitespace()) {
        chars.next();
    }

    // 符号
    if chars.peek() == Some(&'-') || chars.peek() == Some(&'+') {
        num_str.push(chars.next().expect("peek confirmed Some"));
    }

    // 数字と小数点
    let mut has_dot = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num_str.push(chars.next().expect("peek confirmed Some"));
        } else if c == '.' && !has_dot {
            has_dot = true;
            num_str.push(chars.next().expect("peek confirmed Some"));
        } else {
            break;
        }
    }

    num_str.parse().unwrap_or(0.0)
}

/// 人間可読数値をパース（1K, 2M, 3G等）
fn parse_human_numeric(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }

    let (num_part, suffix) = {
        let s = s.trim();
        let mut num_end = s.len();
        for (i, c) in s.char_indices().rev() {
            if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' {
                num_end = i + c.len_utf8();
                break;
            }
        }
        if num_end == s.len() {
            // 数字のみ
            let last_char = s.chars().last().expect("s is not empty after check");
            if last_char.is_ascii_alphabetic() {
                (&s[..s.len() - 1], Some(last_char))
            } else {
                (s, None)
            }
        } else {
            (&s[..num_end], s.chars().last())
        }
    };

    let base: f64 = num_part.trim().parse().unwrap_or(0.0);

    let multiplier = match suffix {
        Some('K') | Some('k') => 1024.0,
        Some('M') | Some('m') => 1024.0 * 1024.0,
        Some('G') | Some('g') => 1024.0 * 1024.0 * 1024.0,
        Some('T') | Some('t') => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        Some('P') | Some('p') => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        Some('E') | Some('e') => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };

    base * multiplier
}

/// 月名を数値に変換
fn month_to_num(s: &str) -> i32 {
    let s = s.trim().to_uppercase();
    let s = if s.len() >= 3 { &s[..3] } else { &s };

    match s {
        "JAN" => 1,
        "FEB" => 2,
        "MAR" => 3,
        "APR" => 4,
        "MAY" => 5,
        "JUN" => 6,
        "JUL" => 7,
        "AUG" => 8,
        "SEP" => 9,
        "OCT" => 10,
        "NOV" => 11,
        "DEC" => 12,
        _ => 0,
    }
}

/// バージョン番号比較
fn compare_version(a: &str, b: &str) -> Ordering {
    let parse_version = |s: &str| -> Vec<(i64, String)> {
        let mut result = Vec::new();
        let mut num = String::new();
        let mut text = String::new();

        for c in s.chars() {
            if c.is_ascii_digit() {
                if !text.is_empty() {
                    result.push((0, text.clone()));
                    text.clear();
                }
                num.push(c);
            } else {
                if !num.is_empty() {
                    result.push((num.parse().unwrap_or(0), String::new()));
                    num.clear();
                }
                text.push(c);
            }
        }

        if !num.is_empty() {
            result.push((num.parse().unwrap_or(0), String::new()));
        }
        if !text.is_empty() {
            result.push((0, text));
        }

        result
    };

    let va = parse_version(a);
    let vb = parse_version(b);

    for (pa, pb) in va.iter().zip(vb.iter()) {
        match pa.0.cmp(&pb.0) {
            Ordering::Equal => match pa.1.cmp(&pb.1) {
                Ordering::Equal => continue,
                other => return other,
            },
            other => return other,
        }
    }

    va.len().cmp(&vb.len())
}

/// 2つの行を比較
fn compare_lines(a: &str, b: &str, config: &Config) -> Ordering {
    for key in &config.keys {
        let key_a = extract_key(a, key, config.field_separator);
        let key_b = extract_key(b, key, config.field_separator);

        let opts = &key.options;

        // 先頭空白除去
        let key_a = if opts.ignore_leading_blanks {
            trim_leading_blanks(key_a)
        } else {
            key_a
        };
        let key_b = if opts.ignore_leading_blanks {
            trim_leading_blanks(key_b)
        } else {
            key_b
        };

        // フィルタリング
        let (key_a, key_b): (std::borrow::Cow<str>, std::borrow::Cow<str>) = if opts.dictionary {
            (
                filter_dictionary(key_a).into(),
                filter_dictionary(key_b).into(),
            )
        } else if opts.ignore_nonprinting {
            (
                filter_nonprinting(key_a).into(),
                filter_nonprinting(key_b).into(),
            )
        } else {
            (key_a.into(), key_b.into())
        };

        let cmp = if opts.numeric {
            let na = parse_numeric(&key_a);
            let nb = parse_numeric(&key_b);
            na.partial_cmp(&nb).unwrap_or(Ordering::Equal)
        } else if opts.general_numeric {
            let na: f64 = key_a.trim().parse().unwrap_or(f64::NEG_INFINITY);
            let nb: f64 = key_b.trim().parse().unwrap_or(f64::NEG_INFINITY);
            na.partial_cmp(&nb).unwrap_or(Ordering::Equal)
        } else if opts.human_numeric {
            let na = parse_human_numeric(&key_a);
            let nb = parse_human_numeric(&key_b);
            na.partial_cmp(&nb).unwrap_or(Ordering::Equal)
        } else if opts.month {
            let ma = month_to_num(&key_a);
            let mb = month_to_num(&key_b);
            ma.cmp(&mb)
        } else if opts.version {
            compare_version(&key_a, &key_b)
        } else {
            compare_text(&key_a, &key_b, opts.ignore_case)
        };

        let cmp = if opts.reverse { cmp.reverse() } else { cmp };

        if cmp != Ordering::Equal {
            return cmp;
        }
    }

    // 安定ソートでない場合、同値なら元の文字列で比較
    if !config.stable {
        compare_text(a, b, false)
    } else {
        Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_text_respects_case_sensitivity_setting() {
        assert_eq!(compare_text("Alpha", "alpha", false), Ordering::Less);
        assert_eq!(compare_text("Alpha", "alpha", true), Ordering::Equal);
        assert_eq!(compare_text("bravo", "CHARLIE", true), Ordering::Less);
    }

    #[test]
    fn unmatched_glob_pattern_is_treated_as_literal() {
        let expanded = expand_glob("__codex_no_match__*.txt").unwrap();
        assert_eq!(expanded, vec!["__codex_no_match__*.txt".to_string()]);
    }

    #[test]
    fn glob_is_case_insensitive_for_file_names() {
        let temp_dir = std::env::temp_dir().join(format!("sort_glob_test_{}", process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let file_path = temp_dir.join("MixedCase.TXT");
        std::fs::write(&file_path, b"test").unwrap();

        let pattern = temp_dir.join("*.txt");
        let expanded = expand_glob(&pattern.to_string_lossy()).unwrap();

        assert!(expanded.iter().any(|path| path.ends_with("MixedCase.TXT")));

        std::fs::remove_file(file_path).unwrap();
        std::fs::remove_dir(temp_dir).unwrap();
    }
}

/// ソート済みかチェック
fn check_sorted(lines: &[String], config: &Config) -> bool {
    for i in 1..lines.len() {
        let cmp = compare_lines(&lines[i - 1], &lines[i], config);
        let is_ordered = if config.unique {
            cmp == Ordering::Less
        } else {
            cmp != Ordering::Greater
        };

        if !is_ordered {
            if !config.check_silent {
                eprintln!(
                    "sort: {}:{}: 順序が乱れています: {}",
                    if config.files.len() == 1 {
                        &config.files[0]
                    } else {
                        "-"
                    },
                    i + 1,
                    &lines[i]
                );
            }
            return false;
        }
    }
    true
}

/// 出力
fn output_lines(lines: &[String], config: &Config) -> Result<(), String> {
    let stdout = io::stdout();
    let mut writer: Box<dyn Write> = if let Some(ref path) = config.output_file {
        let file = File::create(path).map_err(|e| format!("sort: '{}': {}", path, e))?;
        Box::new(BufWriter::new(file))
    } else {
        Box::new(BufWriter::new(stdout.lock()))
    };

    let terminator = if config.zero_terminated { '\0' } else { '\n' };
    let mut prev: Option<&String> = None;

    for line in lines {
        // 重複削除
        if config.unique {
            if let Some(p) = prev {
                if compare_lines(p, line, config) == Ordering::Equal {
                    continue;
                }
            }
        }

        write!(writer, "{}{}", line, terminator).map_err(|e| format!("書き込みエラー: {}", e))?;

        prev = Some(line);
    }

    writer
        .flush()
        .map_err(|e| format!("書き込みエラー: {}", e))?;
    Ok(())
}

fn run() -> Result<(), String> {
    let config = parse_args()?;

    // 全ファイルから行を読み込む
    let mut all_lines = Vec::new();
    for file in &config.files {
        let lines = read_lines(file, config.zero_terminated)?;
        all_lines.extend(lines);
    }

    // チェックモード
    if config.check || config.check_silent {
        let sorted = check_sorted(&all_lines, &config);
        process::exit(if sorted { 0 } else { 1 });
    }

    // ソート
    if config.stable {
        // 安定ソート
        all_lines.sort_by(|a, b| compare_lines(a, b, &config));
    } else {
        // 不安定ソート（より高速）
        all_lines.sort_unstable_by(|a, b| compare_lines(a, b, &config));
    }

    // 出力
    output_lines(&all_lines, &config)?;

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e);
        process::exit(1);
    }
}
