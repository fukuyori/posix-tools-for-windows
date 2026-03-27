use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use glob;

#[derive(Default)]
struct Options {
    // POSIX オプション
    bytes: Option<String>,      // -b: バイト位置
    chars: Option<String>,      // -c: 文字位置（POSIXでは-bと同じだがGNU拡張でマルチバイト対応）
    fields: Option<String>,     // -f: フィールド
    delimiter: Option<char>,    // -d: 区切り文字（1文字のみ、デフォルトTAB）
    only_delimited: bool,       // -s: 区切り文字を含む行のみ
    no_split: bool,             // -n: マルチバイト文字を分割しない（-bと共に使用）
    
    // GNU拡張オプション
    output_delimiter: Option<String>, // --output-delimiter
    complement: bool,           // --complement: 選択を反転
    zero_terminated: bool,      // -z: null区切り
    
    show_help: bool,
    show_version: bool,
}

#[derive(Clone, Debug)]
enum Range {
    Single(usize),              // N
    From(usize),                // N-
    To(usize),                  // -M
    Between(usize, usize),      // N-M
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("cut: {}", e);
            std::process::exit(1);
        }
    };
    
    if opts.show_help {
        print_help();
        std::process::exit(0);
    }
    
    if opts.show_version {
        println!("cut 1.0.0 (Rust実装)");
        std::process::exit(0);
    }
    
    // モード確認
    let mode_count = [&opts.bytes, &opts.chars, &opts.fields]
        .iter()
        .filter(|x| x.is_some())
        .count();
    
    if mode_count == 0 {
        eprintln!("cut: -b, -c, -f のいずれかを指定してください");
        eprintln!("詳細は 'cut --help' を実行してください。");
        std::process::exit(1);
    }
    
    if mode_count > 1 {
        eprintln!("cut: -b, -c, -f は1つだけ指定してください");
        std::process::exit(1);
    }
    
    // -n は -b と共にのみ使用可能
    if opts.no_split && opts.bytes.is_none() {
        eprintln!("cut: -n は -b と共に指定してください");
        std::process::exit(1);
    }
    
    // -d と -s は -f と共にのみ使用可能（POSIX）
    if (opts.delimiter.is_some() || opts.only_delimited) && opts.fields.is_none() {
        eprintln!("cut: -d または -s は -f と共に指定してください");
        std::process::exit(1);
    }
    
    // 範囲をパース
    let spec = opts.bytes.as_ref()
        .or(opts.chars.as_ref())
        .or(opts.fields.as_ref())
        .unwrap();
    
    let ranges = match parse_ranges(spec) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cut: {}", e);
            std::process::exit(1);
        }
    };
    
    if ranges.is_empty() {
        eprintln!("cut: 範囲を指定してください");
        std::process::exit(1);
    }
    
    let mut exit_code = 0;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    
    // POSIX: ファイルが指定されない場合は標準入力
    let files: Vec<String> = if files.is_empty() {
        vec!["-".to_string()]
    } else {
        files
    };
    
    for file in &files {
        let result = if file == "-" {
            process_stdin(&opts, &ranges, &mut stdout)
        } else {
            process_file(file, &opts, &ranges, &mut stdout)
        };
        
        if let Err(e) = result {
            if file == "-" {
                eprintln!("cut: 標準入力: {}", format_error(&e));
            } else {
                eprintln!("cut: {}: {}", file, format_error(&e));
            }
            exit_code = 1;
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
        
        if end_of_opts || !arg.starts_with('-') || arg == "-" {
            files.push(arg.clone());
            i += 1;
            continue;
        }
        
        match arg.as_str() {
            "--" => end_of_opts = true,
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            "--complement" => opts.complement = true,
            "-s" | "--only-delimited" => opts.only_delimited = true,
            "-z" | "--zero-terminated" => opts.zero_terminated = true,
            "-n" => opts.no_split = true,
            "-b" | "--bytes" => {
                if let Some(val) = args.get(i + 1) {
                    if !val.starts_with('-') || val.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ',') {
                        opts.bytes = Some(val.clone());
                        i += 1;
                    } else {
                        return Err(format!("-b のリストがありません"));
                    }
                } else {
                    return Err(format!("-b のリストがありません"));
                }
            }
            "-c" | "--characters" => {
                if let Some(val) = args.get(i + 1) {
                    if !val.starts_with('-') || val.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ',') {
                        opts.chars = Some(val.clone());
                        i += 1;
                    } else {
                        return Err(format!("-c のリストがありません"));
                    }
                } else {
                    return Err(format!("-c のリストがありません"));
                }
            }
            "-f" | "--fields" => {
                if let Some(val) = args.get(i + 1) {
                    if !val.starts_with('-') || val.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ',') {
                        opts.fields = Some(val.clone());
                        i += 1;
                    } else {
                        return Err(format!("-f のリストがありません"));
                    }
                } else {
                    return Err(format!("-f のリストがありません"));
                }
            }
            "-d" | "--delimiter" => {
                if let Some(val) = args.get(i + 1) {
                    let chars: Vec<char> = val.chars().collect();
                    if chars.len() != 1 {
                        return Err(format!("区切り文字は1文字でなければなりません"));
                    }
                    opts.delimiter = Some(chars[0]);
                    i += 1;
                } else {
                    return Err(format!("-d のあとに区切り文字がありません"));
                }
            }
            "--output-delimiter" => {
                if let Some(val) = args.get(i + 1) {
                    opts.output_delimiter = Some(val.clone());
                    i += 1;
                }
            }
            // -bLIST, -cLIST, -fLIST 形式
            s if s.starts_with("-b") && s.len() > 2 => {
                opts.bytes = Some(s[2..].to_string());
            }
            s if s.starts_with("-c") && s.len() > 2 => {
                opts.chars = Some(s[2..].to_string());
            }
            s if s.starts_with("-f") && s.len() > 2 => {
                opts.fields = Some(s[2..].to_string());
            }
            // -dDELIM 形式
            s if s.starts_with("-d") && s.len() > 2 => {
                let delim = &s[2..];
                let chars: Vec<char> = delim.chars().collect();
                if chars.len() != 1 {
                    return Err(format!("区切り文字は1文字でなければなりません"));
                }
                opts.delimiter = Some(chars[0]);
            }
            // --bytes=LIST 等
            s if s.starts_with("--bytes=") => {
                opts.bytes = Some(s.trim_start_matches("--bytes=").to_string());
            }
            s if s.starts_with("--characters=") => {
                opts.chars = Some(s.trim_start_matches("--characters=").to_string());
            }
            s if s.starts_with("--fields=") => {
                opts.fields = Some(s.trim_start_matches("--fields=").to_string());
            }
            s if s.starts_with("--delimiter=") => {
                let delim = s.trim_start_matches("--delimiter=");
                let chars: Vec<char> = delim.chars().collect();
                if chars.len() != 1 {
                    return Err(format!("区切り文字は1文字でなければなりません"));
                }
                opts.delimiter = Some(chars[0]);
            }
            s if s.starts_with("--output-delimiter=") => {
                opts.output_delimiter = Some(s.trim_start_matches("--output-delimiter=").to_string());
            }
            s if s.starts_with("--") => {
                return Err(format!("不明なオプション '{}'", s));
            }
            // 複合短縮オプション（-sn など）
            s => {
                for c in s.chars().skip(1) {
                    match c {
                        's' => opts.only_delimited = true,
                        'n' => opts.no_split = true,
                        'z' => opts.zero_terminated = true,
                        _ => return Err(format!("不明なオプション '-{}'", c)),
                    }
                }
            }
        }
        
        i += 1;
    }
    
    // glob展開
    let files = expand_globs(files);
    
    Ok((opts, files))
}

/// POSIXシェルに近いglob展開を行う。
/// Windowsではシェルが展開しないため、内部で展開して Linux と揃える。
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    
    let options = glob::MatchOptions {
        // POSIX のシェル展開に寄せて大文字小文字は区別する。
        case_sensitive: true,
        ..Default::default()
    };
    
    for pattern in raw_files {
        let glob_pattern = prepare_glob_pattern(&pattern);

        // "-" は標準入力なのでそのまま
        if glob_pattern == "-" {
            result.push(pattern);
            continue;
        }
        
        // POSIX glob のメタ文字（*, ?, [...]）を含む場合は展開する
        if has_glob_meta(&glob_pattern) {
            match glob::glob_with(&glob_pattern, options) {
                Ok(paths) => {
                    let mut matches = Vec::new();
                    for entry in paths {
                        if let Ok(path) = entry {
                            matches.push(normalize_path(path));
                        }
                    }

                    if matches.is_empty() {
                        // マッチなしの場合は元のパターンをそのまま（エラー表示用）
                        result.push(pattern);
                    } else {
                        matches.sort();
                        result.extend(matches);
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

fn prepare_glob_pattern(pattern: &str) -> String {
    #[cfg(windows)]
    {
        pattern.replace('\\', "/")
    }

    #[cfg(not(windows))]
    {
        pattern.to_string()
    }
}

fn has_glob_meta(pattern: &str) -> bool {
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\\' if !cfg!(windows) => {
                let _ = chars.next();
            }
            '*' | '?' if !in_class => return true,
            '[' if !in_class => in_class = true,
            ']' if in_class => return true,
            _ => {}
        }
    }

    false
}

fn normalize_path(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn print_help() {
    println!(r#"使い方: cut -b LIST [オプション] [ファイル]...
       cut -c LIST [オプション] [ファイル]...
       cut -f LIST [オプション] [ファイル]...

各行から選択した部分を切り出して表示します。

モード（いずれか1つを指定、必須）:
  -b, --bytes=LIST        バイト位置で選択
  -c, --characters=LIST   文字位置で選択（マルチバイト対応）
  -f, --fields=LIST       フィールドで選択

POSIXオプション:
  -d, --delimiter=DELIM   フィールド区切り文字（デフォルト: TAB）
                          1文字のみ指定可能
  -n                      マルチバイト文字を分割しない（-bと共に使用）
  -s, --only-delimited    区切り文字を含まない行を出力しない（-fと共に使用）

GNU拡張オプション:
      --complement        選択を反転（指定以外を出力）
      --output-delimiter=STR
                          出力時の区切り文字
  -z, --zero-terminated   行区切りをNULLにする
      --help              このヘルプを表示
      --version           バージョンを表示

LIST の形式（1から始まる）:
  N       N番目のバイト/文字/フィールドのみ
  N-      N番目から末尾まで
  N-M     N番目からM番目まで
  -M      先頭からM番目まで
  複数指定はカンマで区切る（例: 1,3,5-7）

例:
  cut -f1 file.txt           最初のフィールド
  cut -f1,3 file.txt         1番目と3番目のフィールド
  cut -f1-3 file.txt         1〜3番目のフィールド
  cut -d, -f2 file.csv       カンマ区切りの2番目
  cut -c1-10 file.txt        最初の10文字
  cut -b1-100 file.txt       最初の100バイト
  cut -f2- file.txt          2番目以降のフィールド
  cut --complement -f1 file  1番目以外のフィールド"#);
}

fn parse_ranges(spec: &str) -> Result<Vec<Range>, String> {
    let mut ranges = Vec::new();
    
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        
        let range = parse_single_range(part)?;
        ranges.push(range);
    }
    
    if ranges.is_empty() {
        return Err("範囲が指定されていません".to_string());
    }
    
    Ok(ranges)
}

fn parse_single_range(s: &str) -> Result<Range, String> {
    if s.contains('-') {
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        let start = parts[0].trim();
        let end = parts[1].trim();
        
        match (start.is_empty(), end.is_empty()) {
            (true, true) => {
                Err("無効な範囲指定です".to_string())
            }
            (true, false) => {
                let n: usize = end.parse()
                    .map_err(|_| format!("無効なフィールド値 '{}'", end))?;
                if n == 0 {
                    return Err("フィールドと位置は1から始まります".to_string());
                }
                Ok(Range::To(n))
            }
            (false, true) => {
                let n: usize = start.parse()
                    .map_err(|_| format!("無効なフィールド値 '{}'", start))?;
                if n == 0 {
                    return Err("フィールドと位置は1から始まります".to_string());
                }
                Ok(Range::From(n))
            }
            (false, false) => {
                let n: usize = start.parse()
                    .map_err(|_| format!("無効なフィールド値 '{}'", start))?;
                let m: usize = end.parse()
                    .map_err(|_| format!("無効なフィールド値 '{}'", end))?;
                if n == 0 || m == 0 {
                    return Err("フィールドと位置は1から始まります".to_string());
                }
                if n > m {
                    return Err(format!("無効な減少範囲です: {}-{}", n, m));
                }
                Ok(Range::Between(n, m))
            }
        }
    } else {
        let n: usize = s.parse()
            .map_err(|_| format!("無効なフィールド値 '{}'", s))?;
        if n == 0 {
            return Err("フィールドと位置は1から始まります".to_string());
        }
        Ok(Range::Single(n))
    }
}

/// 範囲を展開してインデックスリストに変換
fn expand_ranges(ranges: &[Range], max_len: usize, complement: bool) -> Vec<usize> {
    let mut indices: Vec<usize> = Vec::new();
    
    for range in ranges {
        match range {
            Range::Single(n) => {
                if *n >= 1 && *n <= max_len {
                    indices.push(*n);
                }
            }
            Range::From(n) => {
                for i in *n..=max_len {
                    indices.push(i);
                }
            }
            Range::To(n) => {
                for i in 1..=(*n).min(max_len) {
                    indices.push(i);
                }
            }
            Range::Between(n, m) => {
                for i in *n..=(*m).min(max_len) {
                    indices.push(i);
                }
            }
        }
    }
    
    // ソートして重複を除去
    indices.sort();
    indices.dedup();
    
    if complement {
        let all: Vec<usize> = (1..=max_len).collect();
        all.into_iter().filter(|i| !indices.contains(i)).collect()
    } else {
        indices
    }
}

fn process_stdin<W: Write>(opts: &Options, ranges: &[Range], writer: &mut W) -> io::Result<()> {
    let stdin = io::stdin();
    let reader = stdin.lock();
    process_reader(reader, opts, ranges, writer)
}

fn process_file<W: Write>(
    path: &str,
    opts: &Options,
    ranges: &[Range],
    writer: &mut W,
) -> io::Result<()> {
    let path = Path::new(path);
    
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "ディレクトリです",
        ));
    }
    
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    process_reader(reader, opts, ranges, writer)
}

fn process_reader<R: BufRead, W: Write>(
    reader: R,
    opts: &Options,
    ranges: &[Range],
    writer: &mut W,
) -> io::Result<()> {
    let line_terminator = if opts.zero_terminated { b'\0' } else { b'\n' };
    
    // バイナリセーフな行読み込み
    let mut lines = ByteLines::new(reader, line_terminator);
    
    while let Some(line_result) = lines.next() {
        let line_bytes = line_result?;
        
        if opts.bytes.is_some() {
            process_bytes(&line_bytes, opts, ranges, writer)?;
        } else if opts.chars.is_some() {
            process_chars(&line_bytes, opts, ranges, writer)?;
        } else if opts.fields.is_some() {
            process_fields(&line_bytes, opts, ranges, writer)?;
        }
    }
    
    Ok(())
}

fn process_bytes<W: Write>(
    line: &[u8],
    opts: &Options,
    ranges: &[Range],
    writer: &mut W,
) -> io::Result<()> {
    let line_terminator = if opts.zero_terminated { b'\0' } else { b'\n' };
    
    if opts.no_split {
        // -n: マルチバイト文字を分割しない
        // UTF-8として解釈し、文字境界を尊重
        let text = String::from_utf8_lossy(line);
        let chars: Vec<char> = text.chars().collect();
        
        // バイト位置から文字を選択（マルチバイト文字は分割しない）
        let mut byte_to_char: Vec<Option<usize>> = vec![None; line.len() + 1];
        let mut byte_pos = 0;
        for (char_idx, c) in chars.iter().enumerate() {
            let char_len = c.len_utf8();
            for _ in 0..char_len {
                if byte_pos < byte_to_char.len() {
                    byte_to_char[byte_pos] = Some(char_idx);
                }
                byte_pos += 1;
            }
        }
        
        let indices = expand_ranges(ranges, line.len(), opts.complement);
        let mut selected_chars: Vec<usize> = Vec::new();
        
        for &idx in &indices {
            if idx > 0 && idx <= line.len() {
                if let Some(char_idx) = byte_to_char[idx - 1] {
                    if !selected_chars.contains(&char_idx) {
                        selected_chars.push(char_idx);
                    }
                }
            }
        }
        
        selected_chars.sort();
        let output: String = selected_chars.iter()
            .filter_map(|&i| chars.get(i).copied())
            .collect();
        
        writer.write_all(output.as_bytes())?;
    } else {
        // 通常のバイト選択
        let indices = expand_ranges(ranges, line.len(), opts.complement);
        
        for &idx in &indices {
            if idx > 0 && idx <= line.len() {
                writer.write_all(&[line[idx - 1]])?;
            }
        }
    }
    
    writer.write_all(&[line_terminator])?;
    Ok(())
}

fn process_chars<W: Write>(
    line: &[u8],
    opts: &Options,
    ranges: &[Range],
    writer: &mut W,
) -> io::Result<()> {
    let line_terminator = if opts.zero_terminated { b'\0' } else { b'\n' };
    
    // UTF-8として解釈
    let text = String::from_utf8_lossy(line);
    let chars: Vec<char> = text.chars().collect();
    let indices = expand_ranges(ranges, chars.len(), opts.complement);
    
    let output: String = indices
        .iter()
        .filter_map(|&i| chars.get(i - 1).copied())
        .collect();
    
    writer.write_all(output.as_bytes())?;
    writer.write_all(&[line_terminator])?;
    
    Ok(())
}

fn process_fields<W: Write>(
    line: &[u8],
    opts: &Options,
    ranges: &[Range],
    writer: &mut W,
) -> io::Result<()> {
    let line_terminator = if opts.zero_terminated { b'\0' } else { b'\n' };
    let delimiter = opts.delimiter.unwrap_or('\t');
    
    // UTF-8として解釈
    let text = String::from_utf8_lossy(line);
    
    // 区切り文字を含まない行
    if !text.contains(delimiter) {
        if !opts.only_delimited {
            writer.write_all(line)?;
            writer.write_all(&[line_terminator])?;
        }
        return Ok(());
    }
    
    let fields: Vec<&str> = text.split(delimiter).collect();
    let indices = expand_ranges(ranges, fields.len(), opts.complement);
    
    let selected: Vec<&str> = indices
        .iter()
        .filter_map(|&i| fields.get(i - 1).copied())
        .collect();
    
    // 出力区切り文字を決定して出力
    if let Some(ref out_delim) = opts.output_delimiter {
        writer.write_all(selected.join(out_delim.as_str()).as_bytes())?;
    } else {
        // 入力区切り文字を使用
        let delim_str: String = delimiter.to_string();
        writer.write_all(selected.join(&delim_str).as_bytes())?;
    }
    
    writer.write_all(&[line_terminator])?;
    
    Ok(())
}

/// io::Errorを日本語メッセージに変換
fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        _ => e.to_string(),
    }
}

/// バイナリセーフな行イテレータ
struct ByteLines<R> {
    reader: R,
    buffer: Vec<u8>,
    terminator: u8,
    finished: bool,
}

#[cfg(test)]
mod tests {
    use super::{expand_globs, has_glob_meta, prepare_glob_pattern};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detects_posix_glob_meta() {
        assert!(has_glob_meta("*.txt"));
        assert!(has_glob_meta("file?.txt"));
        assert!(has_glob_meta("file[0-9].txt"));
        assert!(!has_glob_meta("plain.txt"));
        if cfg!(windows) {
            assert!(has_glob_meta("file\\*.txt"));
        } else {
            assert!(!has_glob_meta("file\\*.txt"));
        }
    }

    #[test]
    fn expands_bracket_patterns_and_sorts_matches() {
        let dir = make_temp_dir("glob-brackets");
        fs::write(dir.join("b.txt"), b"b").unwrap();
        fs::write(dir.join("a.txt"), b"a").unwrap();

        let pattern = format!("{}/[ab].txt", dir.display()).replace('\\', "/");
        let expanded = expand_globs(vec![pattern]);
        let expected = vec![
            format!("{}/a.txt", dir.display()).replace('\\', "/"),
            format!("{}/b.txt", dir.display()).replace('\\', "/"),
        ];

        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(expanded, expected);
    }

    #[test]
    fn keeps_directory_matches_so_runtime_reports_them() {
        let dir = make_temp_dir("glob-dirs");
        fs::create_dir(dir.join("alpha")).unwrap();

        let pattern = format!("{}/a*", dir.display()).replace('\\', "/");
        let expanded = expand_globs(vec![pattern]);
        let expected = vec![format!("{}/alpha", dir.display()).replace('\\', "/")];

        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(expanded, expected);
    }

    #[test]
    fn normalizes_windows_style_patterns_before_expansion() {
        if !cfg!(windows) {
            return;
        }

        let dir = make_temp_dir("glob-backslash");
        let nested = dir.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("a.txt"), b"a").unwrap();

        let pattern = format!("{}\\*.txt", nested.display());
        let expanded = expand_globs(vec![pattern]);
        let expected = vec![format!("{}/a.txt", nested.display()).replace('\\', "/")];

        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(expanded, expected);
    }

    #[test]
    fn prepares_windows_paths_for_glob() {
        if cfg!(windows) {
            assert_eq!(prepare_glob_pattern(r"src\*.txt"), "src/*.txt");
        } else {
            assert_eq!(prepare_glob_pattern(r"src\*.txt"), r"src\*.txt");
        }
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cut-{prefix}-{unique}"));
        fs::create_dir(&dir).unwrap();
        dir
    }
}

impl<R: BufRead> ByteLines<R> {
    fn new(reader: R, terminator: u8) -> Self {
        ByteLines {
            reader,
            buffer: Vec::with_capacity(8192),
            terminator,
            finished: false,
        }
    }
    
    fn next(&mut self) -> Option<io::Result<Vec<u8>>> {
        if self.finished {
            return None;
        }
        
        self.buffer.clear();
        
        match self.reader.read_until(self.terminator, &mut self.buffer) {
            Ok(0) => {
                self.finished = true;
                None
            }
            Ok(_) => {
                // 終端文字を削除
                if self.buffer.last() == Some(&self.terminator) {
                    self.buffer.pop();
                }
                // CRLFの場合はCRも削除
                if self.terminator == b'\n' && self.buffer.last() == Some(&b'\r') {
                    self.buffer.pop();
                }
                Some(Ok(std::mem::take(&mut self.buffer)))
            }
            Err(e) => {
                self.finished = true;
                Some(Err(e))
            }
        }
    }
}
