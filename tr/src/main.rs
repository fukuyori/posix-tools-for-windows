use std::env;
use std::io::{self, Read, Write};
use std::process;

#[derive(Debug)]
struct Config {
    /// SET1
    set1: String,
    /// SET2（オプション）
    set2: Option<String>,
    /// 補集合（-c, -C）
    complement: bool,
    /// 削除（-d）
    delete: bool,
    /// 連続文字を圧縮（-s）
    squeeze: bool,
    /// 切り詰め（-t）: SET1をSET2の長さに切り詰める
    truncate: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            set1: String::new(),
            set2: None,
            complement: false,
            delete: false,
            squeeze: false,
            truncate: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: tr [オプション]... SET1 [SET2]
標準入力から読み込み、バイト単位で文字の変換・削除を行い、標準出力に書き出します。

オプション:
  -c, -C, --complement    SET1の補集合を使用
  -d, --delete            SET1のバイトを削除（変換しない）
  -s, --squeeze-repeats   SET2（または-dなしの場合SET1）の連続する同一バイトを1つに圧縮
  -t, --truncate-set1     SET1をSET2の長さに切り詰めてから変換
      --help              このヘルプを表示
      --version           バージョン情報を表示

SET の指定方法:
  文字列          リテラル文字列（UTF-8引数はそのバイト列として扱う）
  CHAR1-CHAR2     CHAR1からCHAR2までの範囲（例: a-z, 0-9）
  [CHAR*]         SET2で使用、CHAR をSET1と同じ長さまで繰り返す
  [CHAR*REPEAT]   CHAR を REPEAT 回繰り返す
  [:alnum:]       英数字
  [:alpha:]       英字
  [:blank:]       水平空白（スペースとタブ）
  [:cntrl:]       制御文字
  [:digit:]       数字
  [:graph:]       印字可能文字（空白を除く）
  [:lower:]       小文字
  [:print:]       印字可能文字（空白を含む）
  [:punct:]       句読点
  [:space:]       空白文字（スペース、タブ、改行など）
  [:upper:]       大文字
  [:xdigit:]      16進数字
  [=CHAR=]        CHAR と等価な文字（現在はCHAR自身のみ）
  \NNN            8進数でバイトを指定
  \\              バックスラッシュ
  \a              ベル（BEL）
  \b              バックスペース
  \f              フォームフィード
  \n              改行
  \r              復帰
  \t              タブ
  \v              垂直タブ

例:
  tr 'a-z' 'A-Z'          小文字を大文字に変換
  tr -d '\r'              CRを削除（Windows改行をUnix改行に）
  tr -s ' '               連続するスペースを1つに圧縮
  tr -d '[:cntrl:]'       制御文字をすべて削除
  tr '[:lower:]' '[:upper:]'  小文字を大文字に変換
  tr -cs '[:alnum:]' '\n'     単語ごとに改行（英数字以外を改行に変換・圧縮）
  tr -d '\000-\037'       制御文字を削除（8進数指定）

注意:
  trは標準入力のみ処理します。ファイル名引数や glob 展開は扱いません。
  ファイルを処理するには:
    tr 'a-z' 'A-Z' < file.txt
    Get-Content file.txt -Raw | tr 'a-z' 'A-Z'
"#
    );
}

fn print_version() {
    eprintln!("tr (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn parse_args() -> Result<Config, String> {
    parse_args_from(env::args().skip(1))
}

fn parse_args_from<I>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let mut config = Config::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            positional.extend(args[i + 1..].iter().cloned());
            break;
        } else if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-c" || arg == "-C" || arg == "--complement" {
            config.complement = true;
        } else if arg == "-d" || arg == "--delete" {
            config.delete = true;
        } else if arg == "-s" || arg == "--squeeze-repeats" {
            config.squeeze = true;
        } else if arg == "-t" || arg == "--truncate-set1" {
            config.truncate = true;
        } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            for c in arg[1..].chars() {
                match c {
                    'c' | 'C' => config.complement = true,
                    'd' => config.delete = true,
                    's' => config.squeeze = true,
                    't' => config.truncate = true,
                    _ => return Err(format!("不明なオプション: -{}", c)),
                }
            }
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("不明なオプション: {}", arg));
        } else {
            positional.push(arg.clone());
        }

        i += 1;
    }

    if positional.is_empty() {
        return Err("SET1 が指定されていません".to_string());
    }

    config.set1 = positional[0].clone();

    if positional.len() > 1 {
        config.set2 = Some(positional[1].clone());
    }

    if positional.len() > 2 {
        return Err("引数が多すぎます".to_string());
    }

    if config.delete && config.squeeze && config.set2.is_none() {
        return Err("-d と -s を併用する場合、SET2 が必要です".to_string());
    }

    if !config.delete && !config.squeeze && config.set2.is_none() {
        return Err("変換にはSET2が必要です".to_string());
    }

    Ok(config)
}

fn parse_escape(bytes: &[u8], index: usize) -> Option<(u8, usize)> {
    let next = *bytes.get(index)?;
    match next {
        b'\\' => Some((b'\\', index + 1)),
        b'a' => Some((0x07, index + 1)),
        b'b' => Some((0x08, index + 1)),
        b'f' => Some((0x0C, index + 1)),
        b'n' => Some((b'\n', index + 1)),
        b'r' => Some((b'\r', index + 1)),
        b't' => Some((b'\t', index + 1)),
        b'v' => Some((0x0B, index + 1)),
        b'0'..=b'7' => {
            let mut octal = vec![next];
            let mut new_index = index + 1;

            for _ in 0..2 {
                match bytes.get(new_index) {
                    Some(b'0'..=b'7') => {
                        octal.push(bytes[new_index]);
                        new_index += 1;
                    }
                    _ => break,
                }
            }

            let code = u8::from_str_radix(std::str::from_utf8(&octal).ok()?, 8).ok()?;
            Some((code, new_index))
        }
        b'x' => {
            let mut hex = Vec::new();
            let mut new_index = index + 1;

            for _ in 0..2 {
                match bytes.get(new_index) {
                    Some(value) if value.is_ascii_hexdigit() => {
                        hex.push(*value);
                        new_index += 1;
                    }
                    _ => break,
                }
            }

            if hex.is_empty() {
                Some((b'x', new_index))
            } else {
                let code = u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?;
                Some((code, new_index))
            }
        }
        other => Some((other, index + 1)),
    }
}

fn find_ascii_sequence(bytes: &[u8], start: usize, sequence: &[u8]) -> Option<usize> {
    if sequence.is_empty() || start >= bytes.len() {
        return None;
    }

    bytes[start..]
        .windows(sequence.len())
        .position(|window| window == sequence)
        .map(|offset| start + offset)
}

fn expand_char_class(class_name: &str) -> Vec<u8> {
    match class_name {
        "alnum" => (b'0'..=b'9')
            .chain(b'A'..=b'Z')
            .chain(b'a'..=b'z')
            .collect(),
        "alpha" => (b'A'..=b'Z').chain(b'a'..=b'z').collect(),
        "blank" => vec![b' ', b'\t'],
        "cntrl" => (0u8..32).chain(std::iter::once(127u8)).collect(),
        "digit" => (b'0'..=b'9').collect(),
        "graph" => (33u8..=126u8).collect(),
        "lower" => (b'a'..=b'z').collect(),
        "print" => (32u8..=126u8).collect(),
        "punct" => b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".to_vec(),
        "space" => vec![b' ', b'\t', b'\n', b'\r', 0x0B, 0x0C],
        "upper" => (b'A'..=b'Z').collect(),
        "xdigit" => (b'0'..=b'9')
            .chain(b'A'..=b'F')
            .chain(b'a'..=b'f')
            .collect(),
        _ => Vec::new(),
    }
}

fn expand_set(set_str: &str, other_set_len: Option<usize>) -> Result<Vec<u8>, String> {
    let bytes = set_str.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if let Some((escaped, new_index)) = parse_escape(bytes, i + 1) {
                    result.push(escaped);
                    i = new_index;
                } else {
                    result.push(b'\\');
                    i += 1;
                }
            }
            b'[' => {
                if bytes.get(i + 1) == Some(&b':') {
                    if let Some(end) = find_ascii_sequence(bytes, i + 2, b":]") {
                        let class_name = std::str::from_utf8(&bytes[i + 2..end])
                            .map_err(|_| "文字クラス名が不正です".to_string())?;
                        let expanded = expand_char_class(class_name);
                        if expanded.is_empty() {
                            return Err(format!("無効な文字クラス: [:{}:]", class_name));
                        }
                        result.extend(expanded);
                        i = end + 2;
                        continue;
                    }
                } else if bytes.get(i + 1) == Some(&b'=') {
                    if let Some(end) = find_ascii_sequence(bytes, i + 2, b"=]") {
                        let equiv = *bytes.get(i + 2).unwrap_or(&b'[');
                        result.push(equiv);
                        i = end + 2;
                        continue;
                    }
                } else if let Some(close) = bytes[i + 1..].iter().position(|&b| b == b']') {
                    let close = i + 1 + close;
                    if let Some(star) = bytes[i + 1..close].iter().position(|&b| b == b'*') {
                        let star = i + 1 + star;
                        let repeat_char = if star == i + 1 { b'*' } else { bytes[i + 1] };
                        let repeat_slice = &bytes[star + 1..close];
                        let repeat_count = if repeat_slice.is_empty() {
                            other_set_len.unwrap_or(1).saturating_sub(result.len())
                        } else if repeat_slice.first() == Some(&b'0') {
                            usize::from_str_radix(
                                std::str::from_utf8(repeat_slice)
                                    .map_err(|_| "繰り返し回数が不正です".to_string())?,
                                8,
                            )
                            .map_err(|_| {
                                format!(
                                    "無効な繰り返し回数: {}",
                                    String::from_utf8_lossy(repeat_slice)
                                )
                            })?
                        } else {
                            std::str::from_utf8(repeat_slice)
                                .map_err(|_| "繰り返し回数が不正です".to_string())?
                                .parse()
                                .map_err(|_| {
                                    format!(
                                        "無効な繰り返し回数: {}",
                                        String::from_utf8_lossy(repeat_slice)
                                    )
                                })?
                        };

                        result.extend(std::iter::repeat_n(repeat_char, repeat_count));
                        i = close + 1;
                        continue;
                    }
                }

                result.push(b'[');
                i += 1;
            }
            b'-' if !result.is_empty() && i + 1 < bytes.len() => {
                let start = *result.last().unwrap();
                let (end, new_index) = if bytes[i + 1] == b'\\' {
                    parse_escape(bytes, i + 2).unwrap_or((b'-', i + 2))
                } else {
                    (bytes[i + 1], i + 2)
                };

                if end == b'-' || end < start {
                    result.push(b'-');
                    if end != b'-' {
                        result.push(end);
                    }
                } else {
                    for value in (start + 1)..=end {
                        result.push(value);
                    }
                }

                i = new_index;
            }
            byte => {
                result.push(byte);
                i += 1;
            }
        }
    }

    Ok(result)
}

fn create_translation_table(set1: &[u8], set2: &[u8], truncate: bool) -> Vec<(u8, u8)> {
    let effective_set1 = if truncate && set1.len() > set2.len() {
        &set1[..set2.len()]
    } else {
        set1
    };

    effective_set1
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            let replacement = if i < set2.len() {
                set2[i]
            } else {
                *set2.last().unwrap_or(&value)
            };
            (value, replacement)
        })
        .collect()
}

fn build_lookup(values: &[u8]) -> [bool; 256] {
    let mut lookup = [false; 256];
    for &value in values {
        lookup[value as usize] = true;
    }
    lookup
}

fn invert_lookup(lookup: &[bool; 256]) -> [bool; 256] {
    let mut inverted = [false; 256];
    for (index, value) in lookup.iter().enumerate() {
        inverted[index] = !value;
    }
    inverted
}

fn transform(config: &Config, input: &[u8]) -> Result<Vec<u8>, String> {
    let set1 = expand_set(&config.set1, None)?;
    let set2 = config
        .set2
        .as_deref()
        .map(|value| expand_set(value, Some(set1.len())))
        .transpose()?;

    let raw_set1_lookup = build_lookup(&set1);
    let effective_set1_lookup = if config.complement {
        invert_lookup(&raw_set1_lookup)
    } else {
        raw_set1_lookup
    };

    let mut translate_active = [false; 256];
    let mut translate_to = [0u8; 256];
    for (index, slot) in translate_to.iter_mut().enumerate() {
        *slot = index as u8;
    }

    if !config.delete {
        if let Some(set2_values) = &set2 {
            let set1_for_table = if config.complement {
                (0u8..=255)
                    .filter(|value| effective_set1_lookup[*value as usize])
                    .collect::<Vec<_>>()
            } else {
                set1.clone()
            };

            for (source, target) in
                create_translation_table(&set1_for_table, set2_values, config.truncate)
            {
                translate_active[source as usize] = true;
                translate_to[source as usize] = target;
            }
        }
    }

    let squeeze_lookup = if config.squeeze {
        if config.delete {
            set2.as_deref().map(build_lookup).unwrap_or([false; 256])
        } else if let Some(set2_values) = &set2 {
            build_lookup(set2_values)
        } else {
            effective_set1_lookup
        }
    } else {
        [false; 256]
    };

    let mut output = Vec::with_capacity(input.len());
    let mut last_output = None;

    for &byte in input {
        let translated = if config.delete && effective_set1_lookup[byte as usize] {
            None
        } else if !config.delete && translate_active[byte as usize] {
            Some(translate_to[byte as usize])
        } else {
            Some(byte)
        };

        if let Some(out) = translated {
            if config.squeeze && squeeze_lookup[out as usize] && last_output == Some(out) {
                continue;
            }
            output.push(out);
            last_output = Some(out);
        }
    }

    Ok(output)
}

fn process(config: &Config) -> Result<(), String> {
    let mut input = Vec::new();
    io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| format!("読み込みエラー: {}", error))?;

    let output = transform(config, &input)?;

    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&output)
        .map_err(|error| format!("書き込みエラー: {}", error))?;
    stdout
        .flush()
        .map_err(|error| format!("書き込みエラー: {}", error))?;

    Ok(())
}

fn main() {
    match parse_args() {
        Ok(config) => {
            if let Err(error) = process(&config) {
                eprintln!("tr: {}", error);
                process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("tr: {}", error);
            eprintln!("詳しくは 'tr --help' を参照してください");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_set, parse_args_from, transform, Config};

    fn config(set1: &str, set2: Option<&str>) -> Config {
        Config {
            set1: set1.to_string(),
            set2: set2.map(str::to_string),
            complement: false,
            delete: false,
            squeeze: false,
            truncate: false,
        }
    }

    #[test]
    fn expands_ascii_range_and_class() {
        assert_eq!(expand_set("a-c", None).unwrap(), b"abc");
        assert_eq!(expand_set("[:digit:]", None).unwrap(), b"0123456789");
    }

    #[test]
    fn translates_bytes_without_text_decoding() {
        let cfg = config("a-z", Some("A-Z"));
        let output = transform(&cfg, b"hello\x00world").unwrap();
        assert_eq!(output, b"HELLO\x00WORLD");
    }

    #[test]
    fn deletes_and_squeezes_posix_style() {
        let mut cfg = config("[:digit:]", Some(" "));
        cfg.delete = true;
        cfg.squeeze = true;

        let output = transform(&cfg, b"a1  2   b33    c").unwrap();
        assert_eq!(output, b"a b c");
    }

    #[test]
    fn complements_against_all_bytes() {
        let mut cfg = config("A", Some("x"));
        cfg.complement = true;

        let output = transform(&cfg, b"A!\xFF").unwrap();
        assert_eq!(output, b"Axx");
    }

    #[test]
    fn honors_double_dash_for_operands() {
        let parsed =
            parse_args_from(vec!["--".to_string(), "-".to_string(), "x".to_string()]).unwrap();
        assert_eq!(parsed.set1, "-");
        assert_eq!(parsed.set2.as_deref(), Some("x"));
    }
}
