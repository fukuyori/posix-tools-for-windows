use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process;

use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use glob::{glob_with, MatchOptions};

#[derive(Debug)]
struct Config {
    /// 入力ファイル
    input_file: Option<String>,
    /// 出力ファイル
    output_file: Option<String>,
    /// 出現回数を前置（-c）
    count: bool,
    /// 重複行のみ表示（-d）
    repeated: bool,
    /// 重複していない行のみ表示（-u）
    unique: bool,
    /// 先頭Nフィールドをスキップ（-f N）
    skip_fields: usize,
    /// 先頭N文字をスキップ（-s N）
    skip_chars: usize,
    /// 比較する文字数（-w N）
    check_chars: Option<usize>,
    /// 大文字小文字を無視（-i）
    ignore_case: bool,
    /// ゼロ終端（-z）
    zero_terminated: bool,
    /// 重複行をすべて表示（-D）
    all_repeated: bool,
    /// -D のグループ区切り方法
    group_separator: GroupSeparator,
    /// グループ表示（--group）
    group: bool,
    /// --group のグループ区切り方法
    group_mode: GroupMode,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GroupSeparator {
    None,
    Prepend,
    Separate,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GroupMode {
    Separate,
    Prepend,
    Append,
    Both,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            input_file: None,
            output_file: None,
            count: false,
            repeated: false,
            unique: false,
            skip_fields: 0,
            skip_chars: 0,
            check_chars: None,
            ignore_case: false,
            zero_terminated: false,
            all_repeated: false,
            group_separator: GroupSeparator::None,
            group: false,
            group_mode: GroupMode::Separate,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: uniq [オプション]... [入力ファイル [出力ファイル]]
隣接する同一行を除去して入力から出力へ書き出します。
入力ファイルが指定されない場合、または - の場合は標準入力を読み込みます。
出力ファイルが指定されない場合は標準出力に書き出します。

オプション:
  -c, --count             出現回数を行頭に付加
  -d, --repeated          重複している行のみ表示（各グループ1行）
  -D                      重複している行をすべて表示
      --all-repeated[=METHOD]  -D と同様、METHOD でグループを区切る
                          METHOD: none（デフォルト）, prepend, separate
  -f, --skip-fields=N     先頭のNフィールドを比較から除外
  -i, --ignore-case       大文字小文字を区別しない
  -s, --skip-chars=N      先頭のN文字を比較から除外
  -u, --unique            重複していない行のみ表示
  -w, --check-chars=N     行の先頭N文字のみ比較
  -z, --zero-terminated   行区切りを改行ではなくNULに
      --group[=METHOD]    空行でグループを区切って表示
                          METHOD: separate（デフォルト）, prepend, append, both
      --help              このヘルプを表示
      --version           バージョン情報を表示

フィールドは空白区切りで、先頭の空白もフィールドの一部とみなされます。

注意: uniqは隣接する行のみ比較します。重複をすべて除去するには、
先にsortでソートしてください: sort file.txt | uniq

例:
  uniq file.txt                    隣接する重複行を除去
  uniq -c file.txt                 出現回数付きで表示
  uniq -d file.txt                 重複行のみ表示
  uniq -u file.txt                 ユニークな行のみ表示
  sort file.txt | uniq             全重複を除去（ソート後にuniq）
  sort file.txt | uniq -c | sort -rn  出現頻度順に表示

globパターン対応:
  uniq *.txt                       シェル展開に近い形で内部展開
                                  Windowsではファイル名の大文字小文字を区別しない
"#
    );
}

fn print_version() {
    eprintln!("uniq (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn glob_match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        ..MatchOptions::new()
    }
}

/// glob展開
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if has_glob_meta(pattern) {
        let mut paths: Vec<String> = glob_with(pattern, glob_match_options())
            .map_err(|e| format!("globパターンエラー: {}", e))?
            .filter_map(Result::ok)
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        paths.sort_by_cached_key(|path| path.to_ascii_lowercase());

        if paths.is_empty() {
            Err(format!(
                "パターン '{}' に一致するファイルがありません",
                pattern
            ))
        } else {
            Ok(paths)
        }
    } else {
        Ok(vec![pattern.to_string()])
    }
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    parse_args_from(args)
}

fn parse_args_from(args: Vec<String>) -> Result<Config, String> {
    let mut config = Config::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    let mut end_of_options = false;

    while i < args.len() {
        let arg = &args[i];

        if end_of_options {
            let expanded = expand_glob(arg)?;
            positional.extend(expanded);
        } else if arg == "--" {
            end_of_options = true;
        } else if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-c" || arg == "--count" {
            config.count = true;
        } else if arg == "-d" || arg == "--repeated" {
            config.repeated = true;
        } else if arg == "-D" {
            config.all_repeated = true;
        } else if arg.starts_with("--all-repeated") {
            config.all_repeated = true;
            if let Some(method) = arg.strip_prefix("--all-repeated=") {
                config.group_separator = match method {
                    "none" => GroupSeparator::None,
                    "prepend" => GroupSeparator::Prepend,
                    "separate" => GroupSeparator::Separate,
                    _ => return Err(format!("--all-repeated の無効な引数: '{}'", method)),
                };
            }
        } else if arg.starts_with("--group") {
            config.group = true;
            if let Some(method) = arg.strip_prefix("--group=") {
                config.group_mode = match method {
                    "separate" => GroupMode::Separate,
                    "prepend" => GroupMode::Prepend,
                    "append" => GroupMode::Append,
                    "both" => GroupMode::Both,
                    _ => return Err(format!("--group の無効な引数: '{}'", method)),
                };
            }
        } else if arg == "-f" || arg == "--skip-fields" {
            i += 1;
            if i >= args.len() {
                return Err("-f オプションには数値が必要です".to_string());
            }
            config.skip_fields = args[i]
                .parse()
                .map_err(|_| format!("無効なフィールド数: '{}'", args[i]))?;
        } else if arg.starts_with("-f") && arg.len() > 2 {
            config.skip_fields = arg[2..]
                .parse()
                .map_err(|_| format!("無効なフィールド数: '{}'", &arg[2..]))?;
        } else if arg.starts_with("--skip-fields=") {
            config.skip_fields = arg[14..]
                .parse()
                .map_err(|_| format!("無効なフィールド数: '{}'", &arg[14..]))?;
        } else if arg == "-s" || arg == "--skip-chars" {
            i += 1;
            if i >= args.len() {
                return Err("-s オプションには数値が必要です".to_string());
            }
            config.skip_chars = args[i]
                .parse()
                .map_err(|_| format!("無効な文字数: '{}'", args[i]))?;
        } else if arg.starts_with("-s") && arg.len() > 2 {
            config.skip_chars = arg[2..]
                .parse()
                .map_err(|_| format!("無効な文字数: '{}'", &arg[2..]))?;
        } else if arg.starts_with("--skip-chars=") {
            config.skip_chars = arg[13..]
                .parse()
                .map_err(|_| format!("無効な文字数: '{}'", &arg[13..]))?;
        } else if arg == "-w" || arg == "--check-chars" {
            i += 1;
            if i >= args.len() {
                return Err("-w オプションには数値が必要です".to_string());
            }
            config.check_chars = Some(
                args[i]
                    .parse()
                    .map_err(|_| format!("無効な文字数: '{}'", args[i]))?,
            );
        } else if arg.starts_with("-w") && arg.len() > 2 {
            config.check_chars = Some(
                arg[2..]
                    .parse()
                    .map_err(|_| format!("無効な文字数: '{}'", &arg[2..]))?,
            );
        } else if arg.starts_with("--check-chars=") {
            config.check_chars = Some(
                arg[14..]
                    .parse()
                    .map_err(|_| format!("無効な文字数: '{}'", &arg[14..]))?,
            );
        } else if arg == "-i" || arg == "--ignore-case" {
            config.ignore_case = true;
        } else if arg == "-u" || arg == "--unique" {
            config.unique = true;
        } else if arg == "-z" || arg == "--zero-terminated" {
            config.zero_terminated = true;
        } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            // 複合オプション（-cd 等）または旧形式（+N, -N）
            let opt_str = &arg[1..];

            // 旧形式の数値オプションをチェック
            if opt_str.chars().all(|c| c.is_ascii_digit()) {
                // -N 形式（フィールドスキップ）は非推奨だが対応
                config.skip_fields = opt_str
                    .parse()
                    .map_err(|_| format!("無効なフィールド数: '{}'", opt_str))?;
            } else {
                for c in opt_str.chars() {
                    match c {
                        'c' => config.count = true,
                        'd' => config.repeated = true,
                        'D' => config.all_repeated = true,
                        'i' => config.ignore_case = true,
                        'u' => config.unique = true,
                        'z' => config.zero_terminated = true,
                        _ => return Err(format!("不明なオプション: -{}", c)),
                    }
                }
            }
        } else if arg.starts_with('+') && arg[1..].chars().all(|c| c.is_ascii_digit()) {
            // +N 形式（文字スキップ）は非推奨だが対応
            config.skip_chars = arg[1..]
                .parse()
                .map_err(|_| format!("無効な文字数: '{}'", &arg[1..]))?;
        } else if arg == "-" {
            positional.push("-".to_string());
        } else if arg.starts_with('-') {
            return Err(format!("不明なオプション: {}", arg));
        } else {
            // 位置引数（ファイル名）- glob展開
            let expanded = expand_glob(arg)?;
            positional.extend(expanded);
        }

        i += 1;
    }

    // 位置引数の処理
    if !positional.is_empty() {
        config.input_file = Some(positional[0].clone());
    }
    if positional.len() > 1 {
        config.output_file = Some(positional[1].clone());
    }
    if positional.len() > 2 {
        return Err("引数が多すぎます".to_string());
    }

    // オプションの競合チェック
    if config.group && (config.repeated || config.unique || config.all_repeated) {
        return Err("--group は -d, -D, -u と併用できません".to_string());
    }

    if config.count && config.all_repeated {
        return Err("-c と -D は併用できません".to_string());
    }

    Ok(config)
}

/// エンコーディング自動検出
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

/// ファイルから行を読み込む
fn read_lines(filename: Option<&str>, zero_term: bool) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();

    match filename {
        None | Some("-") => {
            let stdin = io::stdin();
            let reader = stdin.lock();
            read_lines_from_reader(reader, zero_term, &mut lines)?;
        }
        Some(fname) => {
            let path = Path::new(fname);
            if !path.exists() {
                return Err(format!(
                    "uniq: '{}': そのようなファイルやディレクトリはありません",
                    fname
                ));
            }

            let file = File::open(path).map_err(|e| format!("uniq: '{}': {}", fname, e))?;

            // エンコーディング自動検出
            let mut raw_bytes = Vec::new();
            BufReader::new(&file)
                .read_to_end(&mut raw_bytes)
                .map_err(|e| format!("uniq: '{}': 読み込みエラー: {}", fname, e))?;

            let encoding = detect_encoding(&raw_bytes);

            let file = File::open(path).map_err(|e| format!("uniq: '{}': {}", fname, e))?;

            let decoder = DecodeReaderBytesBuilder::new()
                .encoding(Some(encoding))
                .build(file);
            let reader = BufReader::new(decoder);

            read_lines_from_reader(reader, zero_term, &mut lines)?;
        }
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

/// 比較用のキーを抽出
fn extract_key(line: &str, config: &Config) -> String {
    let mut result = line;

    // フィールドをスキップ
    if config.skip_fields > 0 {
        let mut chars = line.chars().peekable();
        let mut fields_skipped = 0;

        while fields_skipped < config.skip_fields {
            // 空白をスキップ
            while chars.peek().map_or(false, |c| c.is_whitespace()) {
                chars.next();
            }
            // 非空白をスキップ
            while chars.peek().map_or(false, |c| !c.is_whitespace()) {
                chars.next();
            }
            fields_skipped += 1;
        }

        let remaining: String = chars.collect();
        result = Box::leak(remaining.into_boxed_str());
    }

    // 文字をスキップ
    if config.skip_chars > 0 {
        let chars: Vec<char> = result.chars().collect();
        if config.skip_chars < chars.len() {
            let remaining: String = chars[config.skip_chars..].iter().collect();
            result = Box::leak(remaining.into_boxed_str());
        } else {
            result = "";
        }
    }

    // 比較する文字数を制限
    let result = if let Some(n) = config.check_chars {
        let chars: Vec<char> = result.chars().collect();
        if n < chars.len() {
            chars[..n].iter().collect()
        } else {
            result.to_string()
        }
    } else {
        result.to_string()
    };

    // 大文字小文字を無視
    if config.ignore_case {
        result.to_lowercase()
    } else {
        result
    }
}

/// 2つの行が等しいか比較
fn lines_equal(a: &str, b: &str, config: &Config) -> bool {
    extract_key(a, config) == extract_key(b, config)
}

/// 出力を作成
fn create_writer(output_file: Option<&str>) -> Result<Box<dyn Write>, String> {
    match output_file {
        Some(path) => {
            let file = File::create(path).map_err(|e| format!("uniq: '{}': {}", path, e))?;
            Ok(Box::new(BufWriter::new(file)))
        }
        None => Ok(Box::new(BufWriter::new(io::stdout().lock()))),
    }
}

/// メイン処理
fn process(config: &Config) -> Result<(), String> {
    let lines = read_lines(config.input_file.as_deref(), config.zero_terminated)?;
    let mut writer = create_writer(config.output_file.as_deref())?;
    let terminator = if config.zero_terminated { '\0' } else { '\n' };

    if lines.is_empty() {
        return Ok(());
    }

    // グループ化: 隣接する同一行をグループにまとめる
    let mut groups: Vec<Vec<&str>> = Vec::new();
    let mut current_group: Vec<&str> = vec![&lines[0]];

    for line in lines.iter().skip(1) {
        if lines_equal(current_group[0], line, config) {
            current_group.push(line);
        } else {
            groups.push(current_group);
            current_group = vec![line];
        }
    }
    groups.push(current_group);

    // 出力
    let mut first_group = true;

    for group in &groups {
        let count = group.len();
        let is_repeated = count > 1;
        let line = group[0];

        // フィルタリング
        let should_output = if config.unique && config.repeated {
            // -u と -d は両方指定すると何も出力しない
            false
        } else if config.unique {
            !is_repeated
        } else if config.repeated || config.all_repeated {
            is_repeated
        } else if config.group {
            true
        } else {
            true
        };

        if !should_output {
            continue;
        }

        // --group モード
        if config.group {
            // 前のグループとの区切り
            if !first_group
                && (config.group_mode == GroupMode::Separate
                    || config.group_mode == GroupMode::Prepend
                    || config.group_mode == GroupMode::Both)
            {
                write!(writer, "{}", terminator).map_err(|e| format!("書き込みエラー: {}", e))?;
            }

            // グループの行を出力
            for l in group {
                write!(writer, "{}{}", l, terminator)
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }

            first_group = false;
        } else if config.all_repeated {
            // -D: 重複行をすべて表示
            if !first_group
                && (config.group_separator == GroupSeparator::Separate
                    || config.group_separator == GroupSeparator::Prepend)
            {
                write!(writer, "{}", terminator).map_err(|e| format!("書き込みエラー: {}", e))?;
            }

            for l in group {
                write!(writer, "{}{}", l, terminator)
                    .map_err(|e| format!("書き込みエラー: {}", e))?;
            }

            first_group = false;
        } else if config.count {
            // -c: カウント付き
            write!(writer, "{:7} {}{}", count, line, terminator)
                .map_err(|e| format!("書き込みエラー: {}", e))?;
        } else {
            // 通常出力
            write!(writer, "{}{}", line, terminator)
                .map_err(|e| format!("書き込みエラー: {}", e))?;
        }
    }

    // --group の最後の空行
    if config.group
        && (config.group_mode == GroupMode::Append || config.group_mode == GroupMode::Both)
    {
        write!(writer, "{}", terminator).map_err(|e| format!("書き込みエラー: {}", e))?;
    }

    writer
        .flush()
        .map_err(|e| format!("書き込みエラー: {}", e))?;

    Ok(())
}

fn main() {
    match parse_args() {
        Ok(config) => {
            if let Err(e) = process(&config) {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("uniq: {}", e);
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("uniq-tests-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn expand_glob_matches_case_insensitively_on_windows_style_filesystems() {
        let dir = create_temp_dir("glob-case");
        let upper = dir.join("Alpha.TXT");
        let lower = dir.join("beta.txt");
        fs::write(&upper, b"alpha").unwrap();
        fs::write(&lower, b"beta").unwrap();

        let pattern = dir.join("*.txt");
        let matches = expand_glob(&pattern.to_string_lossy()).unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], upper.to_string_lossy());
        assert_eq!(matches[1], lower.to_string_lossy());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_args_stops_option_parsing_after_double_dash() {
        let config = parse_args_from(vec!["--".into(), "-literal.txt".into()]).unwrap();

        assert_eq!(config.input_file.as_deref(), Some("-literal.txt"));
    }

    #[test]
    fn parse_args_rejects_extra_positionals_after_glob_expansion() {
        let dir = create_temp_dir("glob-extra");
        fs::write(dir.join("a.txt"), b"a").unwrap();
        fs::write(dir.join("b.txt"), b"b").unwrap();
        fs::write(dir.join("out.txt"), b"out").unwrap();

        let pattern = dir.join("*.txt").to_string_lossy().to_string();
        let err = parse_args_from(vec![pattern]).unwrap_err();

        assert_eq!(err, "引数が多すぎます");

        fs::remove_dir_all(dir).unwrap();
    }
}
