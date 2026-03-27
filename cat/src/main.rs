use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;

use glob;

// Windows用: stdin/stdoutをバイナリモードに設定し、コンソール判定
#[cfg(windows)]
mod windows_console {
    extern "C" {
        fn _setmode(fd: i32, mode: i32) -> i32;
    }
    const O_BINARY: i32 = 0x8000;
    
    pub fn set_binary_mode() {
        unsafe {
            // stdin (fd=0), stdout (fd=1) をバイナリモードに
            _setmode(0, O_BINARY);
            _setmode(1, O_BINARY);
        }
    }
}

#[cfg(windows)]
use windows_console::*;

#[cfg(not(windows))]
fn set_binary_mode() {
    // Unix系では何もしない
}

#[derive(Default, Clone)]
struct Options {
    // POSIX オプション
    unbuffered: bool,          // -u: バッファリングしない（POSIX必須）
    
    // GNU拡張オプション
    number: bool,              // -n: 行番号を表示
    number_nonblank: bool,     // -b: 空行以外に行番号
    show_ends: bool,           // -E: 行末に $ を表示
    show_tabs: bool,           // -T: タブを ^I で表示
    show_nonprinting: bool,    // -v: 非表示文字を表示
    squeeze_blank: bool,       // -s: 連続空行を1行に
    show_all: bool,            // -A: -vET と同じ
    show_help: bool,
    show_version: bool,
}

impl Options {
    /// オプション処理が必要かどうか
    fn needs_processing(&self) -> bool {
        self.number || self.number_nonblank || self.show_ends 
            || self.show_tabs || self.show_nonprinting || self.squeeze_blank
    }
}

/// 出力先を抽象化
enum Output {
    /// 通常のバイナリ出力
    Binary(io::StdoutLock<'static>),
}

impl Output {
    fn new() -> Self {
        // リークしてスタティックライフタイムを得る（プログラム終了時に解放）
        let stdout = Box::leak(Box::new(io::stdout()));
        Output::Binary(stdout.lock())
    }
    
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Output::Binary(w) => w.write_all(bytes),
        }
    }
    
    fn write_str(&mut self, s: &str) -> io::Result<()> {
        match self {
            Output::Binary(w) => w.write_all(s.as_bytes()),
        }
    }
    
    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> io::Result<()> {
        match self {
            Output::Binary(w) => w.write_fmt(fmt),
        }
    }
    
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Output::Binary(w) => w.flush(),
        }
    }
    
    fn newline(&mut self) -> io::Result<()> {
        match self {
            Output::Binary(w) => writeln!(w),
        }
    }
}

fn main() {
    // Windowsでstdin/stdoutをバイナリモードに設定
    set_binary_mode();
    
    let args: Vec<String> = env::args().collect();
    let (opts, files, had_parse_error) = parse_args(&args);
    
    if opts.show_help {
        print_help();
        std::process::exit(0);
    }
    
    if opts.show_version {
        println!("cat 1.0.0 (Rust実装)");
        std::process::exit(0);
    }

    if had_parse_error {
        std::process::exit(1);
    }
    
    let mut exit_code = 0;
    
    // POSIX: ファイルが指定されない場合は標準入力を読む
    let files: Vec<String> = if files.is_empty() {
        vec!["-".to_string()]
    } else {
        files
    };
    
    // 行番号は全ファイル通して連番
    let mut line_number = 1u64;
    
    // 出力先を作成
    let mut output = Output::new();
    
    for file in &files {
        let result = if file == "-" {
            cat_stdin(&opts, &mut line_number, &mut output)
        } else {
            cat_file(file, &opts, &mut line_number, &mut output)
        };
        
        if let Err(e) = result {
            // POSIX: エラーがあっても残りのファイルは処理を続ける
            if file == "-" {
                eprintln!("cat: 標準入力: {}", format_error(&e));
            } else {
                eprintln!("cat: {}: {}", file, format_error(&e));
            }
            exit_code = 1;
        }
    }
    
    // 最終フラッシュ
    let _ = output.flush();
    
    std::process::exit(exit_code);
}

/// io::Errorを日本語メッセージに変換
fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        io::ErrorKind::InvalidData => "無効なデータです".to_string(),
        io::ErrorKind::UnexpectedEof => "予期しないファイル終端です".to_string(),
        io::ErrorKind::BrokenPipe => "パイプが壊れています".to_string(),
        _ => e.to_string(),
    }
}

fn parse_args(args: &[String]) -> (Options, Vec<String>, bool) {
    let mut opts = Options::default();
    let mut raw_files = Vec::new();
    let mut end_of_opts = false;
    let mut had_error = false;
    
    for arg in args.iter().skip(1) {
        if end_of_opts || !arg.starts_with('-') || arg == "-" {
            raw_files.push(arg.clone());
            continue;
        }
        
        match arg.as_str() {
            "--" => end_of_opts = true,
            "--number" => opts.number = true,
            "--number-nonblank" => opts.number_nonblank = true,
            "--show-ends" => opts.show_ends = true,
            "--show-tabs" => opts.show_tabs = true,
            "--show-nonprinting" => opts.show_nonprinting = true,
            "--squeeze-blank" => opts.squeeze_blank = true,
            "--show-all" => opts.show_all = true,
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            s if s.starts_with("--") => {
                eprintln!("cat: 不明なオプション '{}'", s);
                had_error = true;
            }
            s => {
                for c in s.chars().skip(1) {
                    match c {
                        'u' => opts.unbuffered = true,  // POSIX必須
                        'n' => opts.number = true,
                        'b' => opts.number_nonblank = true,
                        'E' => opts.show_ends = true,
                        'T' => opts.show_tabs = true,
                        'v' => opts.show_nonprinting = true,
                        's' => opts.squeeze_blank = true,
                        'A' => opts.show_all = true,
                        'e' => {
                            opts.show_nonprinting = true;
                            opts.show_ends = true;
                        }
                        't' => {
                            opts.show_nonprinting = true;
                            opts.show_tabs = true;
                        }
                        'h' => opts.show_help = true,
                        _ => {
                            eprintln!("cat: 不明なオプション '-{}'", c);
                            had_error = true;
                        }
                    }
                }
            }
        }
    }
    
    // -A は -vET と同じ
    if opts.show_all {
        opts.show_nonprinting = true;
        opts.show_ends = true;
        opts.show_tabs = true;
    }
    
    // -b は -n を上書き
    if opts.number_nonblank {
        opts.number = false;
    }
    
    // Windowsではシェル展開されないため、内部でglob展開する
    let files = expand_globs(raw_files);
    
    (opts, files, had_error)
}

fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    #[cfg(not(windows))]
    {
        raw_files
    }

    #[cfg(windows)]
    {
        let mut result = Vec::new();
        let options = glob::MatchOptions {
            case_sensitive: false,
            require_literal_separator: true,
            require_literal_leading_dot: true,
            ..Default::default()
        };

        for pattern in raw_files {
            if pattern == "-" {
                result.push(pattern);
                continue;
            }

            if contains_glob_meta(&pattern) {
                match glob::glob_with(&pattern, options) {
                    Ok(paths) => {
                        let mut matched_paths = Vec::new();
                        for entry in paths {
                            if let Ok(path) = entry {
                                matched_paths.push(path.to_string_lossy().to_string());
                            }
                        }

                        if matched_paths.is_empty() {
                            result.push(pattern);
                        } else {
                            matched_paths.sort_by_cached_key(|path| path.to_ascii_lowercase());
                            result.extend(matched_paths);
                        }
                    }
                    Err(_) => result.push(pattern),
                }
            } else {
                result.push(pattern);
            }
        }

        result
    }
}

fn contains_glob_meta(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

fn normalize_path_error(path: &str, err: io::Error) -> io::Error {
    #[cfg(windows)]
    {
        if contains_glob_meta(path) {
            return io::Error::new(io::ErrorKind::NotFound, "No such file or directory");
        }
    }

    err
}

fn print_help() {
    println!(r#"使い方: cat [オプション] [ファイル]...

ファイルを連結して標準出力に出力します。

POSIXオプション:
  -u                   出力をバッファリングしない

GNU拡張オプション:
  -A, --show-all       -vET と同じ
  -b, --number-nonblank
                       空行以外に行番号を付ける
  -e                   -vE と同じ
  -E, --show-ends      行末に $ を表示
  -n, --number         すべての行に行番号を付ける
  -s, --squeeze-blank  連続する空行を1行にまとめる
  -t                   -vT と同じ
  -T, --show-tabs      タブを ^I で表示
  -v, --show-nonprinting
                       非表示文字を ^ や M- 表記で表示（タブと改行以外）
      --help           このヘルプを表示
      --version        バージョンを表示

Windowsではワイルドカード引数を内部で展開し、POSIXシェルに近い動作に合わせます。
マッチしないパターンはそのまま扱われ、通常のファイルエラーになります。

ファイルが指定されない場合、または - が指定された場合は標準入力を読みます。

例:
  cat file.txt              ファイルを表示
  cat file1.txt file2.txt   複数ファイルを連結
  cat -n file.txt           行番号付きで表示
  cat -A file.txt           すべての特殊文字を表示
  cat                       標準入力を表示（Ctrl+D/Ctrl+Z で終了）"#);
}

fn cat_file(path: &str, opts: &Options, line_number: &mut u64, output: &mut Output) -> io::Result<()> {
    let path = Path::new(path);
    
    // POSIX: ディレクトリはエラー
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "ディレクトリです",
        ));
    }
    
    let file = File::open(path).map_err(|e| normalize_path_error(path.to_string_lossy().as_ref(), e))?;
    
    if opts.needs_processing() {
        let reader = BufReader::new(file);
        process_lines(reader, opts, line_number, output)
    } else {
        // オプションなしの場合はバイナリコピー
        raw_copy(file, opts.unbuffered, output)
    }
}

fn cat_stdin(opts: &Options, line_number: &mut u64, output: &mut Output) -> io::Result<()> {
    let stdin = io::stdin();
    
    if opts.needs_processing() {
        let reader = stdin.lock();
        process_lines(reader, opts, line_number, output)
    } else {
        // オプションなしの場合はバイナリコピー
        raw_copy(stdin.lock(), opts.unbuffered, output)
    }
}

/// バイナリデータをそのままコピー（POSIX準拠の基本動作）
fn raw_copy<R: Read>(mut reader: R, unbuffered: bool, output: &mut Output) -> io::Result<()> {
    if unbuffered {
        // -u: 1バイトずつ読み書き
        let mut byte = [0u8; 1];
        loop {
            match reader.read(&mut byte)? {
                0 => break,
                _ => {
                    output.write_all(&byte)?;
                    output.flush()?;
                }
            }
        }
    } else {
        // バッファ付きコピー
        let mut buffer = [0u8; 8192];
        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            output.write_all(&buffer[..bytes_read])?;
        }
        output.flush()?;
    }
    
    Ok(())
}

/// 行単位で処理（-n, -b, -E, -T, -v, -s オプション用）
fn process_lines<R: BufRead>(reader: R, opts: &Options, line_number: &mut u64, output: &mut Output) -> io::Result<()> {
    let mut prev_blank = false;
    
    // バイナリセーフな行読み込み
    let mut lines = ByteLines::new(reader);
    
    while let Some(line_result) = lines.next() {
        let line = line_result?;
        let is_blank = line.is_empty();
        
        // -s: 連続空行をスキップ
        if opts.squeeze_blank && is_blank && prev_blank {
            continue;
        }
        prev_blank = is_blank;
        
        // 行番号
        if opts.number_nonblank {
            if !is_blank {
                output.write_fmt(format_args!("{:6}\t", *line_number))?;
                *line_number += 1;
            }
        } else if opts.number {
            output.write_fmt(format_args!("{:6}\t", *line_number))?;
            *line_number += 1;
        }
        
        // 行内容（バイト単位で処理）
        for &byte in &line {
            write_byte(output, byte, opts)?;
        }
        
        // 行末
        if opts.show_ends {
            output.write_str("$")?;
        }
        
        output.newline()?;
        
        if opts.unbuffered {
            output.flush()?;
        }
    }
    
    Ok(())
}

/// 1バイトを適切に出力
fn write_byte(output: &mut Output, byte: u8, opts: &Options) -> io::Result<()> {
    match byte {
        // タブ
        b'\t' => {
            if opts.show_tabs {
                output.write_str("^I")
            } else {
                output.write_all(&[byte])
            }
        }
        // 通常の表示可能文字（0x20-0x7E: スペース～チルダ）
        0x20..=0x7E => {
            output.write_all(&[byte])
        }
        // 制御文字やバイナリを処理
        _ if opts.show_nonprinting => {
            write_nonprinting(output, byte)
        }
        // そのまま出力
        _ => {
            output.write_all(&[byte])
        }
    }
}

/// 非表示文字を ^ や M- 表記で出力
fn write_nonprinting(output: &mut Output, byte: u8) -> io::Result<()> {
    match byte {
        // 制御文字 (0x00-0x1F) -> ^@ ~ ^_
        // ただしタブ(0x09)は呼び出し側で処理済み
        0x00..=0x1F => {
            output.write_fmt(format_args!("^{}", (byte + 64) as char))
        }
        // DEL (0x7F) -> ^?
        0x7F => {
            output.write_str("^?")
        }
        // 拡張ASCII (0x80-0x9F) -> M-^@ ~ M-^_
        0x80..=0x9F => {
            output.write_fmt(format_args!("M-^{}", (byte - 64) as char))
        }
        // 拡張ASCII (0xA0-0xFE) -> M-<space> ~ M-~
        0xA0..=0xFE => {
            output.write_fmt(format_args!("M-{}", (byte - 128) as char))
        }
        // 0xFF -> M-^?
        0xFF => {
            output.write_str("M-^?")
        }
        // 通常文字（ここには来ないはず）
        _ => {
            output.write_all(&[byte])
        }
    }
}

/// バイナリセーフな行イテレータ
/// 改行で分割するが、改行自体は含めない
struct ByteLines<R> {
    reader: R,
    buffer: Vec<u8>,
    finished: bool,
}

impl<R: Read> ByteLines<R> {
    fn new(reader: R) -> Self {
        ByteLines {
            reader,
            buffer: Vec::with_capacity(8192),
            finished: false,
        }
    }
    
    fn next(&mut self) -> Option<io::Result<Vec<u8>>> {
        if self.finished {
            return None;
        }
        
        self.buffer.clear();
        let mut byte = [0u8; 1];
        
        loop {
            match self.reader.read(&mut byte) {
                Ok(0) => {
                    // EOF
                    self.finished = true;
                    if self.buffer.is_empty() {
                        return None;
                    } else {
                        return Some(Ok(std::mem::take(&mut self.buffer)));
                    }
                }
                Ok(_) => {
                    if byte[0] == b'\n' {
                        return Some(Ok(std::mem::take(&mut self.buffer)));
                    } else {
                        self.buffer.push(byte[0]);
                    }
                }
                Err(e) => {
                    self.finished = true;
                    return Some(Err(e));
                }
            }
        }
    }
}
