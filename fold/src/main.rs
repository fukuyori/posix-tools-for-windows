// fold - 行を指定幅で折り返す
// POSIX.1-2017準拠 + GNU拡張

use std::env;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;

use encoding_rs::{EUC_JP, ISO_2022_JP, SHIFT_JIS, UTF_8};
use glob;

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    width: usize,        // -w: 折り返し幅（デフォルト: 80）
    bytes: bool,         // -b: バイト単位で折り返し
    spaces: bool,        // -s: 空白で折り返し（単語を分割しない）
    
    show_help: bool,
    show_version: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("fold: {}", e);
            eprintln!("詳細は 'fold --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("fold (Rust版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張");
        std::process::exit(0);
    }

    // glob展開
    let files = expand_globs(files);

    let mut exit_code = 0;

    if files.is_empty() {
        // 標準入力から読み込み
        if let Err(e) = fold_stdin(&opts) {
            eprintln!("fold: 標準入力: {}", format_error(&e));
            exit_code = 1;
        }
    } else {
        for file in &files {
            if file == "-" {
                if let Err(e) = fold_stdin(&opts) {
                    eprintln!("fold: 標準入力: {}", format_error(&e));
                    exit_code = 1;
                }
            } else {
                if let Err(e) = fold_file(file, &opts) {
                    eprintln!("fold: '{}': {}", file, format_error(&e));
                    exit_code = 1;
                }
            }
        }
    }

    std::process::exit(exit_code);
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options {
        width: 80,
        ..Default::default()
    };
    let mut files = Vec::new();
    let mut i = 1;
    let mut end_of_opts = false;

    while i < args.len() {
        let arg = &args[i];

        if end_of_opts {
            files.push(arg.clone());
            i += 1;
            continue;
        }

        if arg == "--" {
            end_of_opts = true;
            i += 1;
            continue;
        }

        // ロングオプション
        if arg.starts_with("--") {
            match arg.as_str() {
                "--bytes" => opts.bytes = true,
                "--spaces" => opts.spaces = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                s if s.starts_with("--width=") => {
                    let val = s.trim_start_matches("--width=");
                    opts.width = val.parse().map_err(|_| format!("無効な幅: '{}'", val))?;
                    if opts.width == 0 {
                        return Err("幅は1以上である必要があります".to_string());
                    }
                }
                "--width" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--width' には引数が必要です".to_string());
                    }
                    opts.width = args[i].parse().map_err(|_| format!("無効な幅: '{}'", args[i]))?;
                    if opts.width == 0 {
                        return Err("幅は1以上である必要があります".to_string());
                    }
                }
                _ => return Err(format!("不明なオプション: '{}'", arg)),
            }
            i += 1;
            continue;
        }

        // 短縮オプション
        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;

            // -NUM 形式（例: -80）
            if chars[0].is_ascii_digit() {
                let num_str: String = chars.iter().collect();
                opts.width = num_str.parse().map_err(|_| format!("無効な幅: '{}'", num_str))?;
                if opts.width == 0 {
                    return Err("幅は1以上である必要があります".to_string());
                }
                i += 1;
                continue;
            }

            while j < chars.len() {
                match chars[j] {
                    'b' => opts.bytes = true,
                    's' => opts.spaces = true,
                    'w' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.width = rest.parse().map_err(|_| format!("無効な幅: '{}'", rest))?;
                            if opts.width == 0 {
                                return Err("幅は1以上である必要があります".to_string());
                            }
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-w' には引数が必要です".to_string());
                            }
                            opts.width = args[i].parse().map_err(|_| format!("無効な幅: '{}'", args[i]))?;
                            if opts.width == 0 {
                                return Err("幅は1以上である必要があります".to_string());
                            }
                            break;
                        }
                    }
                    _ => return Err(format!("不正なオプション: '-{}'", chars[j])),
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        files.push(arg.clone());
        i += 1;
    }

    Ok((opts, files))
}

/// glob展開 (POSIX準拠に近づける)
///
/// Linux と同じ動作を狙い、Windows 上では `/` もファイル区切りとして扱えるように補完する。
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();

    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    for pattern in raw_files {
        if pattern == "-" {
            result.push(pattern);
            continue;
        }

        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            let candidates: Vec<String> = if cfg!(windows) && pattern.contains('/') {
                vec![pattern.clone(), pattern.replace('/', "\\")] // Windowsでは/も許容される
            } else {
                vec![pattern.clone()]
            };

            let mut matched_any = false;

            for pat in candidates {
                match glob::glob_with(&pat, options) {
                    Ok(paths) => {
                        for entry in paths {
                            if let Ok(path) = entry {
                                result.push(path.to_string_lossy().to_string());
                                matched_any = true;
                            }
                        }
                    }
                    Err(_) => {
                        // 変換候補の1つが無効でも、別候補を試す
                    }
                }
            }

            if !matched_any {
                result.push(pattern);
            }
        } else {
            result.push(pattern);
        }
    }

    result
}

fn print_help() {
    println!(r#"使い方: fold [オプション]... [ファイル]...

各ファイルの行を指定された幅で折り返して標準出力に書き出します。
ファイルが指定されない場合、または - の場合は標準入力を読み込みます。
UTF-8, Shift_JIS, EUC-JP を自動判定します。

POSIX標準オプション:
  -b, --bytes           バイト単位でカウント（デフォルトは文字単位）
  -s, --spaces          空白位置で折り返し（単語を分割しない）
  -w, --width=WIDTH     最大幅を指定（デフォルト: 80）

GNU拡張:
  -WIDTH                -w WIDTH と同等（例: -60）
      --help            このヘルプを表示して終了
      --version         バージョン情報を表示して終了

文字幅について:
  デフォルトでは表示幅を考慮します:
  - ASCII文字: 1幅
  - 全角文字（漢字、ひらがな等）: 2幅
  - タブ: 次の8の倍数位置まで
  
  -b オプション指定時はバイト単位でカウントします。

終了ステータス:
  0  正常終了
  1  エラー発生
  2  オプションエラー

例:
  fold file.txt              80文字幅で折り返し
  fold -w 40 file.txt        40文字幅で折り返し
  fold -60 file.txt          60文字幅で折り返し
  fold -s file.txt           単語を分割せずに折り返し
  fold -b file.txt           バイト単位で折り返し
  fold -sw 72 file.txt       72文字幅で単語を分割せずに折り返し
  cat file | fold -w 60      パイプ入力
  fold *.txt                 複数ファイル"#);
}

fn fold_stdin(opts: &Options) -> io::Result<()> {
    let stdin = io::stdin();
    let mut buffer = Vec::new();
    stdin.lock().read_to_end(&mut buffer)?;
    
    let content = decode_to_utf8(&buffer);
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    
    fold_content(&content, opts, &mut stdout)
}

fn fold_file(path: &str, opts: &Options) -> io::Result<()> {
    let path = Path::new(path);
    
    if path.is_dir() {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "ディレクトリです"));
    }
    
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer)?;
    
    let content = decode_to_utf8(&buffer);
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    
    fold_content(&content, opts, &mut stdout)
}

fn fold_content<W: Write>(content: &str, opts: &Options, writer: &mut W) -> io::Result<()> {
    // 改行コードを検出
    let line_ending = if content.contains("\r\n") {
        "\r\n"
    } else if content.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    
    let lines: Vec<&str> = if line_ending == "\r\n" {
        content.split("\r\n").collect()
    } else {
        content.split(line_ending.chars().next().unwrap()).collect()
    };
    
    let ends_with_newline = content.ends_with('\n') || content.ends_with('\r');
    
    for (i, line) in lines.iter().enumerate() {
        // 最後の空行はスキップ（改行で終わっている場合の空要素）
        if i == lines.len() - 1 && line.is_empty() && ends_with_newline {
            break;
        }
        
        fold_line(line, opts, writer)?;
        
        // 最後の行以外、または元々改行で終わっていた場合は改行を出力
        if i < lines.len() - 1 || ends_with_newline {
            write!(writer, "{}", line_ending)?;
        }
    }
    
    Ok(())
}

fn fold_line<W: Write>(line: &str, opts: &Options, writer: &mut W) -> io::Result<()> {
    if line.is_empty() {
        return Ok(());
    }
    
    // 改行を除去（既にsplitで除去されている）
    let line = line.trim_end_matches('\r');
    
    if opts.bytes {
        fold_line_bytes(line, opts, writer)
    } else {
        fold_line_chars(line, opts, writer)
    }
}

/// バイト単位で折り返し
fn fold_line_bytes<W: Write>(line: &str, opts: &Options, writer: &mut W) -> io::Result<()> {
    let bytes = line.as_bytes();
    let width = opts.width;
    
    if bytes.len() <= width {
        write!(writer, "{}", line)?;
        return Ok(());
    }
    
    let mut pos = 0;
    
    while pos < bytes.len() {
        let end = (pos + width).min(bytes.len());
        
        if opts.spaces && end < bytes.len() {
            // 空白位置を探す
            if let Some(space_pos) = find_last_space_byte(&bytes[pos..end]) {
                let actual_end = pos + space_pos + 1;
                write!(writer, "{}", String::from_utf8_lossy(&bytes[pos..actual_end]))?;
                writeln!(writer)?;
                pos = actual_end;
                // 先頭の空白をスキップ
                while pos < bytes.len() && bytes[pos] == b' ' {
                    pos += 1;
                }
                continue;
            }
        }
        
        // UTF-8のマルチバイト文字の途中で切らないように調整
        let mut actual_end = end;
        while actual_end > pos && !is_utf8_char_boundary(bytes, actual_end) {
            actual_end -= 1;
        }
        
        if actual_end == pos {
            actual_end = end;
        }
        
        write!(writer, "{}", String::from_utf8_lossy(&bytes[pos..actual_end]))?;
        
        if actual_end < bytes.len() {
            writeln!(writer)?;
        }
        pos = actual_end;
    }
    
    Ok(())
}

/// 文字（表示幅）単位で折り返し
fn fold_line_chars<W: Write>(line: &str, opts: &Options, writer: &mut W) -> io::Result<()> {
    let width = opts.width;
    let chars: Vec<char> = line.chars().collect();
    
    if chars.is_empty() {
        return Ok(());
    }
    
    let mut pos = 0;
    let mut current_col = 0;
    let mut line_start = 0;
    let mut last_space_pos: Option<usize> = None;
    
    while pos < chars.len() {
        let c = chars[pos];
        let char_width = get_char_width(c, current_col);
        
        // 空白位置を記録
        if opts.spaces && c == ' ' {
            last_space_pos = Some(pos);
        }
        
        if current_col + char_width > width {
            // 幅を超える
            if opts.spaces && last_space_pos.is_some() && last_space_pos.unwrap() > line_start {
                // 空白で折り返し
                let space_pos = last_space_pos.unwrap();
                let output: String = chars[line_start..=space_pos].iter().collect();
                write!(writer, "{}", output.trim_end())?;
                writeln!(writer)?;
                
                // 空白をスキップ
                pos = space_pos + 1;
                while pos < chars.len() && chars[pos] == ' ' {
                    pos += 1;
                }
                line_start = pos;
                current_col = 0;
                last_space_pos = None;
            } else {
                // 現在位置で折り返し
                if pos > line_start {
                    let output: String = chars[line_start..pos].iter().collect();
                    write!(writer, "{}", output)?;
                    writeln!(writer)?;
                }
                line_start = pos;
                current_col = 0;
                last_space_pos = None;
            }
        } else {
            current_col += char_width;
            pos += 1;
        }
    }
    
    // 残りを出力
    if line_start < chars.len() {
        let output: String = chars[line_start..].iter().collect();
        write!(writer, "{}", output)?;
    }
    
    Ok(())
}

fn find_last_space_byte(bytes: &[u8]) -> Option<usize> {
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b' ' {
            return Some(i);
        }
    }
    None
}

fn is_utf8_char_boundary(bytes: &[u8], index: usize) -> bool {
    if index >= bytes.len() {
        return true;
    }
    // UTF-8の継続バイトは10xxxxxxの形式
    (bytes[index] & 0b1100_0000) != 0b1000_0000
}

fn get_char_width(c: char, current_col: usize) -> usize {
    match c {
        '\t' => {
            // タブは次の8の倍数位置まで
            8 - (current_col % 8)
        }
        '\r' => 0,
        _ if c.is_control() => 0,
        _ if is_wide_char(c) => 2,
        _ => 1,
    }
}

fn is_wide_char(c: char) -> bool {
    let cp = c as u32;
    
    // ASCII
    if cp <= 0x7F {
        return false;
    }
    
    // 半角カタカナ
    if (0xFF61..=0xFF9F).contains(&cp) {
        return false;
    }
    
    // CJK統合漢字
    if (0x4E00..=0x9FFF).contains(&cp) {
        return true;
    }
    // CJK統合漢字拡張A
    if (0x3400..=0x4DBF).contains(&cp) {
        return true;
    }
    // CJK統合漢字拡張B-G
    if (0x20000..=0x3134F).contains(&cp) {
        return true;
    }
    // ひらがな
    if (0x3040..=0x309F).contains(&cp) {
        return true;
    }
    // カタカナ
    if (0x30A0..=0x30FF).contains(&cp) {
        return true;
    }
    // 全角英数字・記号
    if (0xFF01..=0xFF60).contains(&cp) {
        return true;
    }
    // CJK記号・句読点
    if (0x3000..=0x303F).contains(&cp) {
        return true;
    }
    // 韓国語ハングル
    if (0xAC00..=0xD7AF).contains(&cp) {
        return true;
    }
    // ハングル字母
    if (0x1100..=0x11FF).contains(&cp) {
        return true;
    }
    // 囲みCJK文字・月
    if (0x3200..=0x32FF).contains(&cp) {
        return true;
    }
    // CJK互換文字
    if (0x3300..=0x33FF).contains(&cp) {
        return true;
    }
    // 全角形
    if (0xFFE0..=0xFFE6).contains(&cp) {
        return true;
    }
    
    false
}

// 文字コード関連
fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    // BOMチェック
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }
    
    // JIS（ISO-2022-JP）チェック - エスケープシーケンスで判定
    if contains_jis_escape(bytes) {
        return ISO_2022_JP;
    }

    // UTF-8として有効かチェック
    if std::str::from_utf8(bytes).is_ok() {
        return UTF_8;
    }

    let sjis_score = calc_sjis_score(bytes);
    let eucjp_score = calc_eucjp_score(bytes);

    if sjis_score > eucjp_score {
        SHIFT_JIS
    } else if eucjp_score > sjis_score {
        EUC_JP
    } else {
        SHIFT_JIS
    }
}

/// JISエスケープシーケンスが含まれているかチェック
fn contains_jis_escape(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 0x1B {  // ESC
            // JIS X 0208 開始: ESC $ B または ESC $ @
            if i + 2 < bytes.len() && bytes[i + 1] == b'$' && (bytes[i + 2] == b'B' || bytes[i + 2] == b'@') {
                return true;
            }
            // JIS X 0212 開始: ESC $ ( D
            if i + 3 < bytes.len() && bytes[i + 1] == b'$' && bytes[i + 2] == b'(' && bytes[i + 3] == b'D' {
                return true;
            }
            // ASCII 復帰: ESC ( B または ESC ( J
            if i + 2 < bytes.len() && bytes[i + 1] == b'(' && (bytes[i + 2] == b'B' || bytes[i + 2] == b'J') {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn calc_sjis_score(bytes: &[u8]) -> i32 {
    let mut score = 0;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            // ASCII
            i += 1;
        } else if (0x81..=0x9F).contains(&b) || (0xE0..=0xFC).contains(&b) {
            // Shift_JIS 2バイト文字
            if i + 1 < bytes.len() {
                let b2 = bytes[i + 1];
                if (0x40..=0x7E).contains(&b2) || (0x80..=0xFC).contains(&b2) {
                    score += 2; // 有効な2バイト文字にはより高いスコア
                    i += 2;
                } else {
                    score -= 1;
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else if (0xA1..=0xDF).contains(&b) {
            // 半角カナ
            score += 1;
            i += 1;
        } else if (0xF0..=0xFC).contains(&b) {
            // 機種依存文字領域（Shift_JISの特徴）
            if i + 1 < bytes.len() {
                let b2 = bytes[i + 1];
                if (0x40..=0x7E).contains(&b2) || (0x80..=0xFC).contains(&b2) {
                    score += 3; // 機種依存文字はShift_JISの強い特徴
                    i += 2;
                } else {
                    score -= 1;
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            score -= 1;
            i += 1;
        }
    }
    score
}

fn calc_eucjp_score(bytes: &[u8]) -> i32 {
    let mut score = 0;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            // ASCII
            i += 1;
        } else if (0xA1..=0xFE).contains(&b) {
            // EUC-JP 2バイト文字（JIS X 0208）
            if i + 1 < bytes.len() && (0xA1..=0xFE).contains(&bytes[i + 1]) {
                score += 2; // 有効な2バイト文字にはより高いスコア
                i += 2;
            } else {
                score -= 1;
                i += 1;
            }
        } else if b == 0x8E {
            // 半角カナ（SS2 + 1バイト）
            if i + 1 < bytes.len() && (0xA1..=0xDF).contains(&bytes[i + 1]) {
                score += 2;
                i += 2;
            } else {
                score -= 1;
                i += 1;
            }
        } else if b == 0x8F {
            // JIS X 0212 補助漢字（SS3 + 2バイト）
            if i + 2 < bytes.len() 
                && (0xA1..=0xFE).contains(&bytes[i + 1])
                && (0xA1..=0xFE).contains(&bytes[i + 2]) 
            {
                score += 2;
                i += 3;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_globs_plain_file_remains() {
        let input = vec!["README.md".to_string()];
        let expected = vec!["README.md".to_string()];
        assert_eq!(expand_globs(input), expected);
    }

    #[test]
    fn expand_globs_nonexistent_pattern_is_passed_through() {
        let input = vec!["unmatched_pattern_12345*.txt".to_string()];
        let expected = vec!["unmatched_pattern_12345*.txt".to_string()];
        assert_eq!(expand_globs(input), expected);
    }
}

fn decode_to_utf8(bytes: &[u8]) -> String {
    let encoding = detect_encoding(bytes);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        _ => {
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "そのようなファイルやディレクトリはありません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    _ => format!("{} (エラーコード: {})", e, code),
                };
            }
            e.to_string()
        }
    }
}
