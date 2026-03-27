use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::process;

use glob::{glob_with, MatchOptions};

#[derive(Debug)]
struct Config {
    /// 入力ファイル
    files: Vec<String>,
    /// デリミタ（-d）
    delimiters: Vec<char>,
    /// シリアルモード（-s）
    serial: bool,
    /// ゼロ終端（-z, GNU拡張）
    zero_terminated: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            files: Vec::new(),
            delimiters: vec!['\t'],
            serial: false,
            zero_terminated: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: paste [オプション]... [ファイル]...
複数のファイルの対応する行を結合して出力します。

ファイルが指定されない場合、または - が指定された場合、標準入力を読み込みます。

オプション:
  -d, --delimiters=LIST   タブの代わりにLISTの文字をデリミタとして使用
                          複数文字を指定すると順番に循環して使用
  -s, --serial            ファイルごとに全行を1行にまとめる
  -z, --zero-terminated   行末をNUL文字として扱う（GNU拡張）
      --help              このヘルプを表示
      --version           バージョン情報を表示

デリミタの特殊文字:
  \n    改行
  \t    タブ
  \\    バックスラッシュ
  \0    空文字（デリミタなし）

例:
  paste file1 file2           file1とfile2の各行をタブで結合
  paste -d, file1 file2       カンマで結合
  paste -d'\t\n' a b c        タブと改行を交互に使用
  paste -s file1              file1の全行を1行にまとめる
  paste - - < file            入力を2列に整形
  paste -d: /etc/passwd       パスワードファイルの各フィールドを表示

globパターン対応:
  paste *.txt                 すべての.txtファイルを結合
  paste '[ab]*.txt'           Windowsでも内部でglob展開

globの挙動:
  シェルの pathname expansion に近いルールで展開します
  マッチしないパターンはそのままファイル名として扱います
  先頭が . のファイルは、パターン側にも . がある場合のみ一致します
"#
    );
}

fn print_version() {
    eprintln!("paste (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

/// デリミタ文字列をパース
fn parse_delimiters(s: &str) -> Vec<char> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('0') => result.push('\0'), // 空デリミタ
                Some(other) => result.push(other),
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }

    if result.is_empty() {
        result.push('\t');
    }

    result
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

/// シェルの pathname expansion に近い挙動で glob 展開する
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if pattern == "-" {
        return Ok(vec!["-".to_string()]);
    }

    if !has_glob_meta(pattern) {
        return Ok(vec![pattern.to_string()]);
    }

    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };

    let entries = match glob_with(pattern, options) {
        Ok(entries) => entries,
        Err(_) => return Ok(vec![pattern.to_string()]),
    };

    let mut paths: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if paths.is_empty() {
        Ok(vec![pattern.to_string()])
    } else {
        paths.sort_unstable();
        Ok(paths)
    }
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut config = Config::default();
    let mut i = 0;
    let mut has_delimiter = false;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-d" || arg == "--delimiters" {
            i += 1;
            if i >= args.len() {
                return Err("-d にはデリミタが必要です".to_string());
            }
            config.delimiters = parse_delimiters(&args[i]);
            has_delimiter = true;
        } else if arg.starts_with("--delimiters=") {
            config.delimiters = parse_delimiters(&arg[13..]);
            has_delimiter = true;
        } else if arg.starts_with("-d") && arg.len() > 2 {
            config.delimiters = parse_delimiters(&arg[2..]);
            has_delimiter = true;
        } else if arg == "-s" || arg == "--serial" {
            config.serial = true;
        } else if arg == "-z" || arg == "--zero-terminated" {
            config.zero_terminated = true;
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                let expanded = expand_glob(&args[j])?;
                config.files.extend(expanded);
            }
            break;
        } else if arg.starts_with('-') && arg.len() > 1 && arg != "-" {
            // 複合オプション
            let mut chars = arg[1..].chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    's' => config.serial = true,
                    'z' => config.zero_terminated = true,
                    'd' => {
                        let rest: String = chars.collect();
                        if rest.is_empty() {
                            i += 1;
                            if i >= args.len() {
                                return Err("-d にはデリミタが必要です".to_string());
                            }
                            config.delimiters = parse_delimiters(&args[i]);
                        } else {
                            config.delimiters = parse_delimiters(&rest);
                        }
                        has_delimiter = true;
                        break;
                    }
                    _ => return Err(format!("不明なオプション: -{}", c)),
                }
            }
        } else {
            let expanded = expand_glob(arg)?;
            config.files.extend(expanded);
        }

        i += 1;
    }

    // ファイルが指定されていない場合は標準入力
    if config.files.is_empty() {
        config.files.push("-".to_string());
    }

    // デリミタが指定されていない場合はタブ
    if !has_delimiter {
        config.delimiters = vec!['\t'];
    }

    Ok(config)
}

/// ファイルリーダーのトレイト
enum InputReader {
    File(BufReader<File>),
    Stdin,
}

/// 標準入力用の共有バッファ
struct StdinReader {
    reader: BufReader<io::Stdin>,
}

impl StdinReader {
    fn new() -> Self {
        StdinReader {
            reader: BufReader::new(io::stdin()),
        }
    }

    fn read_line(&mut self, buf: &mut String, zero_terminated: bool) -> io::Result<usize> {
        buf.clear();
        if zero_terminated {
            let mut byte = [0u8; 1];
            let mut count = 0;
            loop {
                match std::io::Read::read(&mut self.reader, &mut byte)? {
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
            self.reader.read_line(buf)
        }
    }
}

impl InputReader {
    fn new(path: &str) -> Result<Self, String> {
        if path == "-" {
            Ok(InputReader::Stdin)
        } else {
            let file = File::open(path)
                .map_err(|e| format!("paste: '{}': {}", path, e))?;
            Ok(InputReader::File(BufReader::new(file)))
        }
    }

    fn read_line(&mut self, buf: &mut String, zero_terminated: bool, stdin_reader: &mut Option<StdinReader>) -> io::Result<usize> {
        buf.clear();
        match self {
            InputReader::File(r) => {
                if zero_terminated {
                    let mut byte = [0u8; 1];
                    let mut count = 0;
                    loop {
                        match std::io::Read::read(r, &mut byte)? {
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
                    r.read_line(buf)
                }
            }
            InputReader::Stdin => {
                if let Some(ref mut sr) = stdin_reader {
                    sr.read_line(buf, zero_terminated)
                } else {
                    Ok(0)
                }
            }
        }
    }
}

/// 通常モード：各ファイルの対応する行を結合
fn paste_parallel(config: &Config) -> Result<(), String> {
    let mut readers: Vec<Option<InputReader>> = config
        .files
        .iter()
        .map(|f| InputReader::new(f).ok())
        .collect();

    // 標準入力を使用するかチェック
    let uses_stdin = config.files.iter().any(|f| f == "-");
    let mut stdin_reader = if uses_stdin {
        Some(StdinReader::new())
    } else {
        None
    };

    // 読み込めなかったファイルをチェック
    for (i, reader) in readers.iter().enumerate() {
        if reader.is_none() && config.files[i] != "-" {
            eprintln!(
                "paste: '{}': ファイルを開けません",
                config.files[i]
            );
        }
    }

    let mut line_buf = String::new();
    let line_terminator = if config.zero_terminated { '\0' } else { '\n' };

    loop {
        let mut any_data = false;
        let mut output = String::new();
        let mut delim_idx = 0;

        for (i, reader_opt) in readers.iter_mut().enumerate() {
            if i > 0 {
                let delim = config.delimiters[delim_idx % config.delimiters.len()];
                if delim != '\0' {
                    output.push(delim);
                }
                delim_idx += 1;
            }

            if let Some(ref mut reader) = reader_opt {
                match reader.read_line(&mut line_buf, config.zero_terminated, &mut stdin_reader) {
                    Ok(0) => {
                        // EOF
                    }
                    Ok(_) => {
                        any_data = true;
                        // 末尾の改行/NULを除去
                        let trimmed = line_buf.trim_end_matches(&['\n', '\r', '\0'][..]);
                        output.push_str(trimmed);
                    }
                    Err(e) => {
                        eprintln!("paste: 読み込みエラー: {}", e);
                    }
                }
            }
        }

        if !any_data {
            break;
        }

        output.push(line_terminator);
        print!("{}", output);
    }

    Ok(())
}

/// シリアルモード：各ファイルの全行を1行にまとめる
fn paste_serial(config: &Config) -> Result<(), String> {
    let line_terminator = if config.zero_terminated { '\0' } else { '\n' };

    // 標準入力を使用するかチェック
    let uses_stdin = config.files.iter().any(|f| f == "-");
    let mut stdin_reader = if uses_stdin {
        Some(StdinReader::new())
    } else {
        None
    };

    for file_path in &config.files {
        let mut reader = match InputReader::new(file_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}", e);
                continue;
            }
        };

        let mut line_buf = String::new();
        let mut first = true;
        let mut delim_idx = 0;

        loop {
            match reader.read_line(&mut line_buf, config.zero_terminated, &mut stdin_reader) {
                Ok(0) => break,
                Ok(_) => {
                    if !first {
                        let delim = config.delimiters[delim_idx % config.delimiters.len()];
                        if delim != '\0' {
                            print!("{}", delim);
                        }
                        delim_idx += 1;
                    }
                    first = false;

                    let trimmed = line_buf.trim_end_matches(&['\n', '\r', '\0'][..]);
                    print!("{}", trimmed);
                }
                Err(e) => {
                    eprintln!("paste: 読み込みエラー: {}", e);
                    break;
                }
            }
        }

        if !first {
            print!("{}", line_terminator);
        }
    }

    Ok(())
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("paste: {}", e);
            eprintln!("詳しくは 'paste --help' を参照してください");
            process::exit(1);
        }
    };

    let result = if config.serial {
        paste_serial(&config)
    } else {
        paste_parallel(&config)
    };

    if let Err(e) = result {
        eprintln!("{}", e);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::expand_glob;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("paste-test-{}-{}", std::process::id(), nanos));
        dir
    }

    fn touch(path: &Path) {
        fs::write(path, b"test").unwrap();
    }

    #[test]
    fn unmatched_glob_is_preserved_as_literal() {
        let pattern = "no-match-*.txt";
        assert_eq!(expand_glob(pattern).unwrap(), vec![pattern.to_string()]);
    }

    #[test]
    fn invalid_glob_is_preserved_as_literal() {
        let pattern = "broken[pattern";
        assert_eq!(expand_glob(pattern).unwrap(), vec![pattern.to_string()]);
    }

    #[test]
    fn glob_expansion_is_sorted_and_skips_dotfiles_without_explicit_dot() {
        let dir = unique_temp_dir();
        fs::create_dir(&dir).unwrap();

        touch(&dir.join("b.txt"));
        touch(&dir.join("a.txt"));
        touch(&dir.join(".hidden.txt"));

        let pattern = format!("{}\\*.txt", dir.display());
        let matches = expand_glob(&pattern).unwrap();

        assert_eq!(
            matches,
            vec![
                dir.join("a.txt").to_string_lossy().to_string(),
                dir.join("b.txt").to_string_lossy().to_string()
            ]
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
