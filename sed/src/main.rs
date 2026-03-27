// sed - ストリームエディタ
// POSIX.1-2017準拠 + GNU拡張

use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8};
use fancy_regex::{Error as FancyRegexError, Regex, RegexBuilder};
use glob;

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    quiet: bool,              // -n: 自動出力を抑制
    expressions: Vec<String>, // -e: スクリプト式
    script_files: Vec<String>,// -f: スクリプトファイル
    
    // GNU拡張オプション
    in_place: bool,           // -i: インプレース編集
    in_place_backup: Option<String>, // -i SUFFIX: バックアップサフィックス
    extended_regex: bool,     // -E, -r: 拡張正規表現
    separate: bool,           // -s: ファイルを個別に処理
    unbuffered: bool,         // -u: バッファなし
    follow_symlinks: bool,    // --follow-symlinks: シンボリックリンクをたどる
    null_data: bool,          // -z: NUL区切り
    
    show_help: bool,
    show_version: bool,
}

#[derive(Debug, Clone)]
enum Address {
    Line(usize),
    LastLine,
    Regex(String),
    Step(usize, usize),  // first~step
}

#[derive(Debug, Clone)]
enum Command {
    Substitute { pattern: String, replacement: String, flags: SubstituteFlags },
    Translate { src: Vec<char>, dst: Vec<char> },
    Delete,
    DeleteFirstLine,
    Print,
    PrintFirstLine,
    PrintUnambiguous,
    PrintLineNum,
    Quit,
    QuitWithCode(i32),
    QuitSilentWithCode(i32),
    Next,
    NextAppend,
    Branch(String),
    Test(String),
    TestNot(String),
    Label(String),
    HoldReplace,
    HoldAppend,
    GetReplace,
    GetAppend,
    Exchange,
    Append(String),
    Insert(String),
    Change(String),
    ReadFile(String),
    WriteFile(String),
    Block(Vec<SedCommand>),
    // GNU拡張
    ZapPattern,    // z: パターンスペースをクリア
}

#[derive(Debug, Clone, Default)]
struct SubstituteFlags {
    global: bool,
    ignore_case: bool,
    print: bool,
    nth: Option<usize>,
    write_file: Option<String>,
    multiline: bool,  // m: マルチラインモード
}

#[derive(Debug, Clone)]
struct SedCommand {
    id: usize,
    addr1: Option<Address>,
    addr2: Option<Address>,
    negate: bool,
    command: Command,
}

struct SedState {
    pattern_space: String,
    hold_space: String,
    line_num: usize,
    substitution_made: bool,
    append_queue: Vec<String>,
    active_ranges: HashMap<usize, bool>,
}

impl SedState {
    fn new() -> Self {
        SedState {
            pattern_space: String::new(),
            hold_space: String::new(),
            line_num: 0,
            substitution_made: false,
            append_queue: Vec::new(),
            active_ranges: HashMap::new(),
        }
    }
}

struct InputLine {
    text: String,
    line_ending: &'static str,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("sed: {}", e);
            eprintln!("詳細は 'sed --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("sed (Rust版) 1.0.0");
        println!("POSIX.1-2017 寄り + GNU/Windows 拡張");
        std::process::exit(0);
    }

    // スクリプトを構築
    let mut script = String::new();
    for expr in &opts.expressions {
        if !script.is_empty() { script.push('\n'); }
        // Windowsでシングルクォートが残っている場合は除去
        let expr = expr.trim_matches('\'');
        script.push_str(expr);
    }

    for script_file in &opts.script_files {
        let script_path = resolve_path_case_insensitive(script_file);
        match fs::read_to_string(&script_path) {
            Ok(content) => {
                if !script.is_empty() { script.push('\n'); }
                script.push_str(&content);
            }
            Err(e) => {
                eprintln!("sed: '{}' を読み込めません: {}", script_file, format_error(&e));
                std::process::exit(1);
            }
        }
    }

    if script.is_empty() {
        eprintln!("sed: スクリプトが指定されていません");
        std::process::exit(2);
    }

    let mut commands = match parse_script(&script) {
        Ok(cmds) => cmds,
        Err(e) => {
            eprintln!("sed: {}", e);
            std::process::exit(1);
        }
    };
    assign_command_ids(&mut commands);

    let labels = build_label_index(&commands);

    // glob展開
    let files = expand_globs(files);

    if files.is_empty() {
        if let Err(e) = process_stdin(&commands, &labels, &opts) {
            eprintln!("sed: {}", format_error(&e));
            std::process::exit(1);
        }
    } else if !opts.in_place && !opts.separate {
        if let Err(e) = process_files_as_stream(&files, &commands, &labels, &opts) {
            eprintln!("sed: {}", format_error(&e));
            std::process::exit(1);
        }
    } else {
        let mut exit_code = 0;
        for file in &files {
            if let Err(e) = process_file(file, &commands, &labels, &opts) {
                eprintln!("sed: '{}': {}", file, format_error(&e));
                exit_code = 1;
            }
        }
        std::process::exit(exit_code);
    }
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options::default();
    let mut files = Vec::new();
    let mut i = 1;
    let mut has_script = false;
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

        match arg.as_str() {
            // POSIX標準オプション
            "-n" | "--quiet" | "--silent" => opts.quiet = true,
            "-e" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-e' には引数が必要です".to_string());
                }
                opts.expressions.push(args[i].clone());
                has_script = true;
            }
            "-f" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-f' には引数が必要です".to_string());
                }
                opts.script_files.push(args[i].clone());
                has_script = true;
            }
            // GNU拡張オプション
            "-E" | "-r" | "--regexp-extended" => opts.extended_regex = true,
            "-i" | "--in-place" => {
                opts.in_place = true;
                if let Some(next) = args.get(i + 1) {
                    if !next.starts_with('-') {
                        opts.in_place_backup = Some(next.clone());
                        i += 1;
                    }
                }
            }
            "-s" | "--separate" => opts.separate = true,
            "-u" | "--unbuffered" => opts.unbuffered = true,
            "-z" | "--null-data" => opts.null_data = true,
            "--follow-symlinks" => opts.follow_symlinks = true,
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            // -e付きの値
            s if s.starts_with("-e") && s.len() > 2 => {
                opts.expressions.push(s[2..].to_string());
                has_script = true;
            }
            // -f付きの値
            s if s.starts_with("-f") && s.len() > 2 => {
                opts.script_files.push(s[2..].to_string());
                has_script = true;
            }
            // -i付きのサフィックス
            s if s.starts_with("-i") && s.len() > 2 => {
                opts.in_place = true;
                opts.in_place_backup = Some(s[2..].to_string());
            }
            // --in-place=SUFFIX
            s if s.starts_with("--in-place=") => {
                opts.in_place = true;
                let suffix = s.trim_start_matches("--in-place=");
                if !suffix.is_empty() {
                    opts.in_place_backup = Some(suffix.to_string());
                }
            }
            // 複合短縮オプション (-nE など)
            s if s.starts_with('-') && !s.starts_with("--") && s.len() > 1 => {
                for c in s[1..].chars() {
                    match c {
                        'n' => opts.quiet = true,
                        'E' | 'r' => opts.extended_regex = true,
                        's' => opts.separate = true,
                        'u' => opts.unbuffered = true,
                        'z' => opts.null_data = true,
                        _ => return Err(format!("不明なオプション: -{}", c)),
                    }
                }
            }
            // 残りの引数
            _ => {
                if !has_script && opts.expressions.is_empty() && opts.script_files.is_empty() {
                    opts.expressions.push(arg.clone());
                    has_script = true;
                } else {
                    files.push(arg.clone());
                }
            }
        }
        i += 1;
    }
    
    Ok((opts, files))
}

fn windows_glob_options() -> glob::MatchOptions {
    glob::MatchOptions {
        case_sensitive: false,
        // POSIXシェルに寄せて、* や ? はディレクトリ境界をまたがない。
        require_literal_separator: true,
        // ドット始まりは、パターン側が明示したときだけマッチさせる。
        require_literal_leading_dot: true,
    }
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn resolve_existing_path(path: &str) -> Option<PathBuf> {
    glob::glob_with(path, windows_glob_options())
        .ok()?
        .flatten()
        .next()
}

fn resolve_path_case_insensitive(path: &str) -> PathBuf {
    if path == "-" {
        PathBuf::from(path)
    } else {
        resolve_existing_path(path).unwrap_or_else(|| PathBuf::from(path))
    }
}

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();

    for pattern in raw_files {
        // "-" は標準入力なのでそのまま
        if pattern == "-" {
            result.push(pattern);
            continue;
        }

        if has_glob_meta(&pattern) {
            match glob::glob_with(&pattern, windows_glob_options()) {
                Ok(paths) => {
                    let mut matches: Vec<String> = paths
                        .flatten()
                        .filter(|path| path.is_file())
                        .map(|path| path.to_string_lossy().to_string())
                        .collect();

                    matches.sort_by_cached_key(|path| path.to_ascii_lowercase());

                    if matches.is_empty() {
                        result.push(pattern);
                    } else {
                        result.extend(matches);
                    }
                }
                Err(_) => {
                    result.push(pattern);
                }
            }
        } else {
            result.push(resolve_path_case_insensitive(&pattern).to_string_lossy().to_string());
        }
    }

    result
}

fn print_help() {
    println!(r#"使い方: sed [オプション]... {{スクリプト}} [入力ファイル]...

ストリームエディタ - テキスト変換を行います。
Windows では入力ファイル名に対して内部 glob 展開を行い、既存パスは大文字小文字を区別せずに解決します。

POSIX標準オプション:
  -n, --quiet, --silent
                 パターンスペースの自動出力を抑制
  -e スクリプト  実行するコマンドを追加
  -f スクリプトファイル
                 スクリプトファイルの内容をコマンドとして追加

GNU拡張オプション:
  -E, -r, --regexp-extended
                 拡張正規表現を使用（POSIXのデフォルトは基本正規表現）
  -i[SUFFIX], --in-place[=SUFFIX]
                 ファイルをその場で編集（SUFFIXがあればバックアップ作成）
  -s, --separate ファイルを連続ストリームとしてではなく個別に処理
  -u, --unbuffered
                 入力ファイルから最小限のデータを読み込み、より頻繁に出力バッファをフラッシュ
  -z, --null-data
                 改行の代わりにNUL文字で行を区切る
      --follow-symlinks
                 -i使用時にシンボリックリンクをたどる
      --help     このヘルプを表示して終了
      --version  バージョン情報を表示して終了

-e または -f がない場合、最初の非オプション引数がsedスクリプトとして使用されます。
残りの引数は入力ファイル名として解釈されます。ファイル指定がない場合は標準入力を読み込みます。
Windows では `*`, `?`, `[]` を内部で展開します。先頭 `.` は明示した場合のみマッチし、既存ファイル名の解決は大小無視です。

アドレス:
  number         指定行番号
  $              最終行
  /regexp/       正規表現にマッチする行
  first~step     first行目から、step行ごと（GNU拡張）

コマンド:
  s/regexp/replacement/flags
                 置換（flags: g=全置換, p=出力, i=大小無視, N=N番目のみ, w FILE=ファイル出力）
  y/src/dst/     文字変換（trに似る）
  d              パターンスペースを削除
  D              パターンスペースの最初の行を削除
  p              パターンスペースを出力
  P              パターンスペースの最初の行を出力
  l              パターンスペースを曖昧さなく出力
  =              行番号を出力
  n              次の行を読み込み
  N              次の行をパターンスペースに追加
  q [exit-code]  終了
  Q [exit-code]  無出力で終了（GNU拡張）
  h H            パターンスペースをホールドスペースにコピー/追加
  g G            ホールドスペースをパターンスペースにコピー/追加
  x              パターンスペースとホールドスペースを交換
  a\text         textを後に追加
  i\text         textを前に挿入
  c\text         パターンスペースをtextで置換
  r file         ファイルの内容を追加
  w file         パターンスペースをファイルに書き込み
  b [label]      ラベルに分岐
  t [label]      置換成功時にラベルに分岐
  T [label]      置換失敗時にラベルに分岐（GNU拡張）
  :label         ラベル定義
  {{ commands }}   コマンドのグループ化
  z              パターンスペースをクリア（GNU拡張）

終了ステータス:
  0  正常終了
  1  処理エラー
  2  オプションエラー

例:
  sed 's/hello/world/' file.txt      helloをworldに置換
  sed -n '5p' file.txt               5行目のみ出力
  sed -n '1,10p' file.txt            1-10行目を出力
  sed '/pattern/d' file.txt          patternを含む行を削除
  sed -i.bak 's/old/new/g' *.txt     インプレース編集（.bakバックアップ）
  sed -E 's/[0-9]+/NUM/g' file.txt   拡張正規表現で数字をNUMに置換"#);
}

fn parse_script(script: &str) -> Result<Vec<SedCommand>, String> {
    let mut commands = Vec::new();
    let chars: Vec<char> = script.chars().collect();
    let mut pos = 0;

    while pos < chars.len() {
        skip_ws_nl(&chars, &mut pos);
        if pos >= chars.len() { break; }
        if chars[pos] == '#' {
            // コメント行をスキップ
            while pos < chars.len() && chars[pos] != '\n' { pos += 1; }
            continue;
        }
        if chars[pos] == ';' { pos += 1; continue; }
        commands.push(parse_sed_command(&chars, &mut pos)?);
    }
    normalize_empty_regexes(&mut commands)?;
    Ok(commands)
}

fn normalize_empty_regexes(commands: &mut [SedCommand]) -> Result<(), String> {
    fn normalize(commands: &mut [SedCommand], last_regex: &mut Option<String>) -> Result<(), String> {
        for command in commands {
            normalize_address(&mut command.addr1, last_regex)?;
            normalize_address(&mut command.addr2, last_regex)?;
            normalize_command_regex(&mut command.command, last_regex)?;
        }
        Ok(())
    }

    fn normalize_address(addr: &mut Option<Address>, last_regex: &mut Option<String>) -> Result<(), String> {
        if let Some(Address::Regex(pattern)) = addr {
            if pattern.is_empty() {
                let reused = last_regex
                    .clone()
                    .ok_or_else(|| "空の正規表現を再利用する対象がありません".to_string())?;
                *pattern = reused;
            } else {
                *last_regex = Some(pattern.clone());
            }
        }
        Ok(())
    }

    fn normalize_command_regex(command: &mut Command, last_regex: &mut Option<String>) -> Result<(), String> {
        match command {
            Command::Substitute { pattern, .. } => {
                if pattern.is_empty() {
                    let reused = last_regex
                        .clone()
                        .ok_or_else(|| "空の正規表現を再利用する対象がありません".to_string())?;
                    *pattern = reused;
                } else {
                    *last_regex = Some(pattern.clone());
                }
            }
            Command::Block(block) => normalize(block, last_regex)?,
            _ => {}
        }
        Ok(())
    }

    let mut last_regex = None;
    normalize(commands, &mut last_regex)
}

fn parse_sed_command(chars: &[char], pos: &mut usize) -> Result<SedCommand, String> {
    skip_ws(chars, pos);
    let addr1 = parse_address(chars, pos)?;
    skip_ws(chars, pos);
    let addr2 = if *pos < chars.len() && chars[*pos] == ',' {
        *pos += 1;
        skip_ws(chars, pos);
        Some(parse_address(chars, pos)?.unwrap_or(Address::LastLine))
    } else { None };
    skip_ws(chars, pos);
    let negate = if *pos < chars.len() && chars[*pos] == '!' {
        *pos += 1; skip_ws(chars, pos); true
    } else { false };
    let command = parse_cmd(chars, pos)?;
    Ok(SedCommand { id: 0, addr1, addr2, negate, command })
}

fn parse_address(chars: &[char], pos: &mut usize) -> Result<Option<Address>, String> {
    skip_ws(chars, pos);
    if *pos >= chars.len() { return Ok(None); }
    let c = chars[*pos];

    if c.is_ascii_digit() {
        let mut s = String::new();
        while *pos < chars.len() && chars[*pos].is_ascii_digit() {
            s.push(chars[*pos]); *pos += 1;
        }
        let n: usize = s.parse().map_err(|_| "無効な行番号です")?;
        if n == 0 {
            return Err("行番号は 1 以上でなければなりません".to_string());
        }
        // GNU拡張: first~step
        if *pos < chars.len() && chars[*pos] == '~' {
            *pos += 1;
            let mut ss = String::new();
            while *pos < chars.len() && chars[*pos].is_ascii_digit() {
                ss.push(chars[*pos]); *pos += 1;
            }
            let step: usize = ss.parse().map_err(|_| "無効なステップ値です")?;
            if step == 0 {
                return Err("ステップ値は 1 以上でなければなりません".to_string());
            }
            return Ok(Some(Address::Step(n, step)));
        }
        return Ok(Some(Address::Line(n)));
    }

    if c == '$' { *pos += 1; return Ok(Some(Address::LastLine)); }

    if c == '/' || c == '\\' {
        let delim = if c == '\\' {
            *pos += 1;
            if *pos >= chars.len() { return Err("区切り文字がありません".to_string()); }
            let d = chars[*pos]; *pos += 1; d
        } else { *pos += 1; '/' };
        let pat = read_delim(chars, pos, delim)?;
        return Ok(Some(Address::Regex(pat)));
    }
    Ok(None)
}

fn parse_cmd(chars: &[char], pos: &mut usize) -> Result<Command, String> {
    if *pos >= chars.len() { return Err("コマンドがありません".to_string()); }
    let c = chars[*pos]; *pos += 1;

    match c {
        's' => parse_subst(chars, pos),
        'y' => parse_trans(chars, pos),
        'd' => Ok(Command::Delete),
        'D' => Ok(Command::DeleteFirstLine),
        'p' => Ok(Command::Print),
        'P' => Ok(Command::PrintFirstLine),
        'l' => Ok(Command::PrintUnambiguous),
        '=' => Ok(Command::PrintLineNum),
        'q' => parse_quit(chars, pos, false),
        'Q' => parse_quit(chars, pos, true),
        'n' => Ok(Command::Next),
        'N' => Ok(Command::NextAppend),
        'h' => Ok(Command::HoldReplace),
        'H' => Ok(Command::HoldAppend),
        'g' => Ok(Command::GetReplace),
        'G' => Ok(Command::GetAppend),
        'x' => Ok(Command::Exchange),
        'z' => Ok(Command::ZapPattern),  // GNU拡張
        'b' => { skip_ws(chars, pos); Ok(Command::Branch(read_label(chars, pos))) }
        't' => { skip_ws(chars, pos); Ok(Command::Test(read_label(chars, pos))) }
        'T' => { skip_ws(chars, pos); Ok(Command::TestNot(read_label(chars, pos))) }
        ':' => { 
            skip_ws(chars, pos); 
            let l = read_label(chars, pos);
            if l.is_empty() { 
                Err("ラベル名が必要です".to_string()) 
            } else { 
                Ok(Command::Label(l)) 
            } 
        }
        'a' => Ok(Command::Append(read_text(chars, pos))),
        'i' => Ok(Command::Insert(read_text(chars, pos))),
        'c' => Ok(Command::Change(read_text(chars, pos))),
        'r' => { 
            skip_ws(chars, pos); 
            let f = read_fname(chars, pos);
            if f.is_empty() { 
                Err("ファイル名が必要です".to_string()) 
            } else { 
                Ok(Command::ReadFile(f)) 
            } 
        }
        'w' => { 
            skip_ws(chars, pos); 
            let f = read_fname(chars, pos);
            if f.is_empty() { 
                Err("ファイル名が必要です".to_string()) 
            } else { 
                Ok(Command::WriteFile(f)) 
            } 
        }
        '{' => {
            let mut blk = Vec::new();
            loop {
                skip_ws_nl(chars, pos);
                if *pos >= chars.len() { return Err("ブロックが閉じられていません".to_string()); }
                if chars[*pos] == '}' { *pos += 1; break; }
                if chars[*pos] == ';' { *pos += 1; continue; }
                blk.push(parse_sed_command(chars, pos)?);
            }
            Ok(Command::Block(blk))
        }
        '}' => Err("予期しない '}' です".to_string()),
        ';' | '\n' => { *pos -= 1; Err("コマンドがありません".to_string()) }
        _ => Err(format!("不明なコマンド: '{}'", c)),
    }
}

fn parse_quit(chars: &[char], pos: &mut usize, silent: bool) -> Result<Command, String> {
    skip_ws(chars, pos);
    let code = read_optional_number(chars, pos)?;
    ensure_command_terminator(chars, pos, "q")?;
    match (silent, code) {
        (false, None) => Ok(Command::Quit),
        (false, Some(code)) => Ok(Command::QuitWithCode(code)),
        (true, None) => Ok(Command::QuitSilentWithCode(0)),
        (true, Some(code)) => Ok(Command::QuitSilentWithCode(code)),
    }
}

fn read_optional_number(chars: &[char], pos: &mut usize) -> Result<Option<i32>, String> {
    let start = *pos;
    let mut digits = String::new();
    while *pos < chars.len() && chars[*pos].is_ascii_digit() {
        digits.push(chars[*pos]);
        *pos += 1;
    }
    if digits.is_empty() {
        *pos = start;
        return Ok(None);
    }
    digits
        .parse::<i32>()
        .map(Some)
        .map_err(|_| "終了コードが不正です".to_string())
}

fn ensure_command_terminator(chars: &[char], pos: &usize, command: &str) -> Result<(), String> {
    if *pos >= chars.len() || matches!(chars[*pos], ';' | '\n' | '}') {
        Ok(())
    } else {
        Err(format!("{} コマンドの後ろに余分な文字があります", command))
    }
}

fn parse_subst(chars: &[char], pos: &mut usize) -> Result<Command, String> {
    if *pos >= chars.len() { return Err("s コマンドに区切り文字がありません".to_string()); }
    let delim = chars[*pos]; *pos += 1;
    let pattern = read_delim(chars, pos, delim)?;
    let replacement = read_delim(chars, pos, delim)?;
    let mut flags = SubstituteFlags::default();
    
    while *pos < chars.len() {
        match chars[*pos] {
            'g' => { flags.global = true; *pos += 1; }
            'p' => { flags.print = true; *pos += 1; }
            'i' | 'I' => { flags.ignore_case = true; *pos += 1; }
            'm' | 'M' => { flags.multiline = true; *pos += 1; }
            '1'..='9' => { 
                let mut num_str = String::new();
                while *pos < chars.len() && chars[*pos].is_ascii_digit() {
                    num_str.push(chars[*pos]); 
                    *pos += 1;
                }
                flags.nth = num_str.parse().ok();
            }
            'w' => { 
                *pos += 1; 
                skip_ws(chars, pos);
                flags.write_file = Some(read_fname(chars, pos)); 
                break; 
            }
            ';' | '\n' | '}' | ' ' | '\t' => break,
            other => {
                return Err(format!("s コマンドのフラグが不正です: '{}'", other));
            }
        }
    }
    Ok(Command::Substitute { pattern, replacement, flags })
}

fn parse_trans(chars: &[char], pos: &mut usize) -> Result<Command, String> {
    if *pos >= chars.len() { return Err("y コマンドに区切り文字がありません".to_string()); }
    let delim = chars[*pos]; *pos += 1;
    let src_s = read_delim(chars, pos, delim)?;
    let dst_s = read_delim(chars, pos, delim)?;
    let src = expand_range(&src_s);
    let dst = expand_range(&dst_s);
    if src.len() != dst.len() { 
        return Err(format!("y コマンドの文字列の長さが一致しません ({} != {})", src.len(), dst.len())); 
    }
    Ok(Command::Translate { src, dst })
}

fn expand_range(s: &str) -> Vec<char> {
    let mut r = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if i + 2 < cs.len() && cs[i + 1] == '-' {
            for c in cs[i]..=cs[i + 2] { r.push(c); }
            i += 3;
        } else if cs[i] == '\\' && i + 1 < cs.len() {
            i += 1;
            r.push(match cs[i] { 'n' => '\n', 't' => '\t', 'r' => '\r', c => c });
            i += 1;
        } else { r.push(cs[i]); i += 1; }
    }
    r
}

fn skip_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && (chars[*pos] == ' ' || chars[*pos] == '\t') { *pos += 1; }
}

fn skip_ws_nl(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() { *pos += 1; }
}

fn read_delim(chars: &[char], pos: &mut usize, delim: char) -> Result<String, String> {
    let mut r = String::new();
    let mut esc = false;
    while *pos < chars.len() {
        let c = chars[*pos]; *pos += 1;
        if esc {
            match c { 
                'n' => r.push('\n'), 
                't' => r.push('\t'), 
                'r' => r.push('\r'),
                _ if c == delim => r.push(c), 
                _ => { r.push('\\'); r.push(c); } 
            }
            esc = false;
        } else if c == '\\' { esc = true; }
        else if c == delim { return Ok(r); }
        else { r.push(c); }
    }
    Err(format!("区切り文字 '{}' が見つかりません", delim))
}

fn read_label(chars: &[char], pos: &mut usize) -> String {
    let mut l = String::new();
    while *pos < chars.len() && !";}\n".contains(chars[*pos]) {
        l.push(chars[*pos]); *pos += 1;
    }
    l.trim().to_string()
}

fn read_fname(chars: &[char], pos: &mut usize) -> String {
    let mut f = String::new();
    while *pos < chars.len() && chars[*pos] != '\n' {
        f.push(chars[*pos]); *pos += 1;
    }
    f.trim_start().to_string()
}

fn read_text(chars: &[char], pos: &mut usize) -> String {
    while *pos < chars.len() && (chars[*pos] == ' ' || chars[*pos] == '\t') { *pos += 1; }
    if *pos < chars.len() && chars[*pos] == '\\' { *pos += 1; }
    if *pos < chars.len() && chars[*pos] == '\n' { *pos += 1; }
    let mut t = String::new();
    while *pos < chars.len() && chars[*pos] != '\n' {
        t.push(chars[*pos]); *pos += 1;
    }
    t
}

fn build_label_index(commands: &[SedCommand]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (i, cmd) in commands.iter().enumerate() {
        if let Command::Label(n) = &cmd.command { m.insert(n.clone(), i); }
        if let Command::Block(b) = &cmd.command { m.extend(build_label_index(b)); }
    }
    m
}

fn assign_command_ids(commands: &mut [SedCommand]) {
    fn assign(commands: &mut [SedCommand], next_id: &mut usize) {
        for command in commands {
            command.id = *next_id;
            *next_id += 1;
            if let Command::Block(block) = &mut command.command {
                assign(block, next_id);
            }
        }
    }

    let mut next_id = 0;
    assign(commands, &mut next_id);
}

fn process_stdin(commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> io::Result<()> {
    let mut buf = Vec::new();
    io::stdin().lock().read_to_end(&mut buf)?;
    let lines = decode_input_lines(&buf, opts);
    let mut out = io::stdout().lock();
    let mut state = SedState::new();
    process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)
}

fn process_files_as_stream(paths: &[String], commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> io::Result<()> {
    let mut lines = Vec::new();
    for path in paths {
        if path == "-" {
            let mut buf = Vec::new();
            io::stdin().lock().read_to_end(&mut buf)?;
            lines.extend(decode_input_lines(&buf, opts));
            continue;
        }

        let p = resolve_path_case_insensitive(path);
        if p.is_dir() {
            return Err(io::Error::new(io::ErrorKind::IsADirectory, "ディレクトリです"));
        }

        let mut buf = Vec::new();
        File::open(&p)?.read_to_end(&mut buf)?;
        lines.extend(decode_input_lines(&buf, opts));
    }

    let mut out = io::stdout().lock();
    let mut state = SedState::new();
    process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)
}

fn process_file(path: &str, commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> io::Result<()> {
    // "-" は標準入力
    if path == "-" {
        return process_stdin(commands, labels, opts);
    }

    let p = resolve_path_case_insensitive(path);
    if p.is_dir() { 
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "ディレクトリです")); 
    }

    let mut buf = Vec::new();
    File::open(&p)?.read_to_end(&mut buf)?;
    let lines = decode_input_lines(&buf, opts);

    if opts.in_place {
        if let Some(ref suf) = opts.in_place_backup {
            let backup_path = format!("{}{}", p.to_string_lossy(), suf);
            fs::copy(&p, &backup_path)?;
        }
        let mut out = Vec::new();
        let mut state = SedState::new();
        process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)?;
        File::create(&p)?.write_all(&out)?;
    } else {
        let mut out = io::stdout().lock();
        let mut state = SedState::new();
        process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)?;
    }
    Ok(())
}

fn decode_input_lines(bytes: &[u8], opts: &Options) -> Vec<InputLine> {
    let content = decode_to_utf8(bytes);
    if opts.null_data {
        content
            .split('\0')
            .map(|line| InputLine { text: line.to_string(), line_ending: "\0" })
            .collect()
    } else {
        let line_ending = detect_line_ending(&content);
        content
            .lines()
            .map(|line| InputLine { text: line.to_string(), line_ending })
            .collect()
    }
}

fn detect_line_ending(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "\r\n"
    } else if content.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

#[cfg(test)]
fn process_lines<W: Write>(lines: &[&str], commands: &[SedCommand], labels: &HashMap<String, usize>,
                           opts: &Options, w: &mut W, _line_ending: &str) -> io::Result<()> {
    let input_lines: Vec<InputLine> = lines
        .iter()
        .map(|line| InputLine { text: (*line).to_string(), line_ending: "\n" })
        .collect();
    let mut state = SedState::new();
    process_input_lines(&input_lines, commands, labels, opts, w, &mut state)
}

fn process_input_lines<W: Write>(lines: &[InputLine], commands: &[SedCommand], labels: &HashMap<String, usize>,
                                 opts: &Options, w: &mut W, state: &mut SedState) -> io::Result<()> {
    let total = lines.len();
    let mut iter = lines.iter().enumerate().peekable();

    while let Some((idx, line)) = iter.next() {
        state.line_num += 1;
        state.pattern_space = line.text.clone();
        state.substitution_made = false;
        state.append_queue.clear();
        let is_last = idx + 1 == total;
        let mut line_ending = line.line_ending;

        let result = exec_cmds(commands, state, labels, opts, w, &mut iter, is_last, &mut line_ending)?;

        match result {
            ExecResult::Continue => {
                if !opts.quiet { write!(w, "{}{}", state.pattern_space, line_ending)?; }
                for t in &state.append_queue { write!(w, "{}{}", t, line_ending)?; }
            }
            ExecResult::Delete => {
                for t in &state.append_queue { write!(w, "{}{}", t, line_ending)?; }
            }
            ExecResult::Quit => {
                if !opts.quiet { write!(w, "{}{}", state.pattern_space, line_ending)?; }
                for t in &state.append_queue { write!(w, "{}{}", t, line_ending)?; }
                break;
            }
            ExecResult::QuitSilent => break,
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum ExecResult { Continue, Delete, Quit, QuitSilent }

fn exec_cmds<'a, W, I>(commands: &[SedCommand], state: &mut SedState, labels: &HashMap<String, usize>,
                       opts: &Options, w: &mut W, iter: &mut std::iter::Peekable<I>, mut is_last: bool,
                       line_ending: &mut &'static str) -> io::Result<ExecResult>
where W: Write, I: Iterator<Item = (usize, &'a InputLine)> {
    let mut ci = 0;
    while ci < commands.len() {
        let cmd = &commands[ci];
        if !addr_match(cmd, state, is_last, opts) { ci += 1; continue; }

        match &cmd.command {
            Command::Substitute { pattern, replacement, flags } => {
                let (new, ok) = apply_subst(&state.pattern_space, pattern, replacement, flags, opts);
                state.pattern_space = new;
                if ok {
                    state.substitution_made = true;
                    if flags.print { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
                    if let Some(ref f) = flags.write_file { append_file(f, &state.pattern_space, line_ending)?; }
                }
            }
            Command::Translate { src, dst } => { 
                state.pattern_space = translate(&state.pattern_space, src, dst); 
            }
            Command::Delete => return Ok(ExecResult::Delete),
            Command::DeleteFirstLine => {
                if let Some(p) = state.pattern_space.find('\n') {
                    state.pattern_space = state.pattern_space[p + 1..].to_string();
                    ci = 0; continue;
                } else { return Ok(ExecResult::Delete); }
            }
            Command::Print => { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
            Command::PrintFirstLine => { 
                write!(w, "{}{}", state.pattern_space.lines().next().unwrap_or(""), *line_ending)?; 
            }
            Command::PrintUnambiguous => { 
                write!(w, "{}${}", esc_unambig(&state.pattern_space), *line_ending)?; 
            }
            Command::PrintLineNum => { write!(w, "{}{}", state.line_num, *line_ending)?; }
            Command::Quit => return Ok(ExecResult::Quit),
            Command::QuitWithCode(c) => {
                if !opts.quiet { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
                for t in &state.append_queue { write!(w, "{}{}", t, *line_ending)?; }
                std::process::exit(*c);
            }
            Command::QuitSilentWithCode(c) => std::process::exit(*c),
            Command::Next => {
                if !opts.quiet { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
                if let Some((i, l)) = iter.next() {
                    state.line_num = i + 1;
                    state.pattern_space = l.text.clone();
                    is_last = iter.peek().is_none();
                    *line_ending = l.line_ending;
                }
                else { return Ok(ExecResult::QuitSilent); }
            }
            Command::NextAppend => {
                if let Some((i, l)) = iter.next() {
                    state.line_num = i + 1;
                    state.pattern_space.push('\n');
                    state.pattern_space.push_str(&l.text);
                    is_last = iter.peek().is_none();
                    *line_ending = l.line_ending;
                } else { return Ok(ExecResult::Quit); }
            }
            Command::HoldReplace => { state.hold_space = state.pattern_space.clone(); }
            Command::HoldAppend => { 
                state.hold_space.push('\n'); 
                state.hold_space.push_str(&state.pattern_space); 
            }
            Command::GetReplace => { state.pattern_space = state.hold_space.clone(); }
            Command::GetAppend => { 
                state.pattern_space.push('\n'); 
                state.pattern_space.push_str(&state.hold_space); 
            }
            Command::Exchange => { std::mem::swap(&mut state.pattern_space, &mut state.hold_space); }
            Command::ZapPattern => { state.pattern_space.clear(); }
            Command::Branch(l) => {
                if l.is_empty() { return Ok(ExecResult::Continue); }
                if let Some(&t) = labels.get(l) { ci = t; continue; }
            }
            Command::Test(l) => {
                if state.substitution_made {
                    state.substitution_made = false;
                    if l.is_empty() { return Ok(ExecResult::Continue); }
                    if let Some(&t) = labels.get(l) { ci = t; continue; }
                }
            }
            Command::TestNot(l) => {
                if !state.substitution_made {
                    if l.is_empty() { return Ok(ExecResult::Continue); }
                    if let Some(&t) = labels.get(l) { ci = t; continue; }
                }
                state.substitution_made = false;
            }
            Command::Label(_) => {}
            Command::Append(t) => { state.append_queue.push(t.clone()); }
            Command::Insert(t) => { write!(w, "{}{}", t, *line_ending)?; }
            Command::Change(t) => { write!(w, "{}{}", t, *line_ending)?; return Ok(ExecResult::Delete); }
            Command::ReadFile(f) => { 
                if let Ok(c) = fs::read_to_string(resolve_path_case_insensitive(f)) { 
                    state.append_queue.push(c.trim_end().to_string()); 
                } 
            }
            Command::WriteFile(f) => { append_file(f, &state.pattern_space, line_ending)?; }
            Command::Block(b) => {
                let r = exec_cmds(b, state, labels, opts, w, iter, is_last, line_ending)?;
                if r != ExecResult::Continue { return Ok(r); }
            }
        }
        ci += 1;
    }
    Ok(ExecResult::Continue)
}

fn addr_match(cmd: &SedCommand, state: &mut SedState, is_last: bool, opts: &Options) -> bool {
    let m = match (&cmd.addr1, &cmd.addr2) {
        (None, None) => true,
        (None, Some(a)) => single_match(a, state.line_num, is_last, &state.pattern_space, opts),
        (Some(a), None) => single_match(a, state.line_num, is_last, &state.pattern_space, opts),
        (Some(start), Some(end)) => {
            let was_active = state.active_ranges.get(&cmd.id).copied().unwrap_or(false);
            if was_active {
                let end_matched = single_match(end, state.line_num, is_last, &state.pattern_space, opts);
                if end_matched {
                    state.active_ranges.remove(&cmd.id);
                }
                true
            } else if single_match(start, state.line_num, is_last, &state.pattern_space, opts) {
                if range_continues_after_start(end, state.line_num, is_last) {
                    state.active_ranges.insert(cmd.id, true);
                }
                true
            } else {
                false
            }
        }
    };
    if cmd.negate { !m } else { m }
}

fn range_continues_after_start(end: &Address, ln: usize, is_last: bool) -> bool {
    match end {
        Address::Line(n) => ln < *n,
        Address::LastLine => !is_last,
        Address::Regex(_) | Address::Step(_, _) => true,
    }
}

fn single_match(a: &Address, ln: usize, is_last: bool, line: &str, opts: &Options) -> bool {
    match a {
        Address::Line(n) => ln == *n,
        Address::LastLine => is_last,
        Address::Regex(p) => build_regex(p, false, false, opts)
            .map(|r| r.is_match(line).unwrap_or(false))
            .unwrap_or(false),
        Address::Step(f, s) => ln >= *f && *s > 0 && (ln - f) % s == 0,
    }
}

fn build_regex(pat: &str, ignore_case: bool, multiline: bool, opts: &Options) -> Result<Regex, FancyRegexError> {
    let base_pattern = if opts.extended_regex {
        pat.to_string()
    } else {
        bre_to_ere(pat)
    };
    let pattern = if multiline {
        format!("(?m:{base_pattern})")
    } else {
        base_pattern
    };

    RegexBuilder::new(&pattern)
        .case_insensitive(ignore_case)
        .build()
}

fn bre_to_ere(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut in_bracket = false;
    let mut bracket_start = false;

    while let Some(c) = chars.next() {
        if in_bracket {
            if c == '\\' {
                out.push(c);
                if let Some(next) = chars.next() {
                    out.push(next);
                    bracket_start = false;
                }
                continue;
            }

            out.push(c);
            if c == '[' && bracket_start {
                bracket_start = false;
                continue;
            }
            if c == ']' && !bracket_start {
                in_bracket = false;
            } else if bracket_start && c != '^' {
                bracket_start = false;
            }
            continue;
        }

        if c == '[' {
            in_bracket = true;
            bracket_start = true;
            out.push(c);
            continue;
        }

        if c == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    '(' | ')' | '{' | '}' => out.push(next),
                    _ => {
                        out.push('\\');
                        out.push(next);
                    }
                }
            } else {
                out.push('\\');
            }
            continue;
        }

        match c {
            '(' | ')' | '{' | '}' | '+' | '?' | '|' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }

    out
}

fn apply_subst(line: &str, pat: &str, repl: &str, flags: &SubstituteFlags, opts: &Options) -> (String, bool) {
    let re = match build_regex(pat, flags.ignore_case, flags.multiline, opts) {
        Ok(r) => r,
        Err(_) => return (line.to_string(), false),
    };
    
    let repl = conv_backref(repl);
    
    if flags.global {
        let r = re.replace_all(line, repl.as_str());
        (r.to_string(), r != line)
    } else if let Some(n) = flags.nth {
        let mut cnt = 0; 
        let mut last = 0; 
        let mut res = String::new(); 
        let mut ok = false;
        for m in re.find_iter(line).flatten() {
            cnt += 1;
            if cnt == n {
                res.push_str(&line[last..m.start()]);
                let replaced = re.replace(m.as_str(), repl.as_str());
                res.push_str(&replaced);
                last = m.end(); 
                ok = true; 
                break;
            }
        }
        res.push_str(&line[last..]);
        if ok { (res, true) } else { (line.to_string(), false) }
    } else {
        let r = re.replace(line, repl.as_str());
        (r.to_string(), r != line)
    }
}

fn conv_backref(s: &str) -> String {
    fn push_literal_replacement_char(out: &mut String, ch: char) {
        if ch == '$' {
            out.push_str("$$");
        } else {
            out.push(ch);
        }
    }

    let mut r = String::new();
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        if c == '\\' {
            if let Some(&n) = cs.peek() {
                match n {
                    '0'..='9' => { r.push('$'); r.push(n); cs.next(); }
                    '&' => { r.push('&'); cs.next(); }
                    '\\' => { r.push('\\'); cs.next(); }
                    'n' => { r.push('\n'); cs.next(); }
                    't' => { r.push('\t'); cs.next(); }
                    _ => { push_literal_replacement_char(&mut r, n); cs.next(); }
                }
            } else { r.push(c); }
        } else if c == '&' { r.push_str("$0"); }
        else { push_literal_replacement_char(&mut r, c); }
    }
    r
}

fn translate(s: &str, src: &[char], dst: &[char]) -> String {
    s.chars()
        .map(|c| src.iter().position(|&x| x == c).and_then(|p| dst.get(p).copied()).unwrap_or(c))
        .collect()
}

fn esc_unambig(s: &str) -> String {
    let mut r = String::new();
    for c in s.chars() {
        match c {
            '\\' => r.push_str("\\\\"),
            '\x07' => r.push_str("\\a"),
            '\x08' => r.push_str("\\b"),
            '\x0c' => r.push_str("\\f"),
            '\n' => r.push_str("\\n"),
            '\r' => r.push_str("\\r"),
            '\t' => r.push_str("\\t"),
            '\x0b' => r.push_str("\\v"),
            '$' => r.push_str("\\$"),
            c if c.is_control() || !c.is_ascii() => r.push_str(&format!("\\{:03o}", c as u32)),
            c => r.push(c),
        }
    }
    r
}

fn append_file(f: &str, line: &str, line_ending: &str) -> io::Result<()> {
    let path = resolve_path_case_insensitive(f);
    write!(OpenOptions::new().create(true).append(true).open(path)?, "{}{}", line, line_ending)
}

fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { return UTF_8; }
    if std::str::from_utf8(bytes).is_ok() { return UTF_8; }
    let sj = calc_sjis(bytes); 
    let eu = calc_euc(bytes);
    if sj > eu { SHIFT_JIS } else if eu > sj { EUC_JP } else { SHIFT_JIS }
}

fn calc_sjis(b: &[u8]) -> i32 {
    let mut s = 0; 
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c <= 0x7F { i += 1; }
        else if (0x81..=0x9F).contains(&c) || (0xE0..=0xFC).contains(&c) {
            if i + 1 < b.len() && ((0x40..=0x7E).contains(&b[i+1]) || (0x80..=0xFC).contains(&b[i+1])) {
                s += 1; i += 2;
            } else { s -= 1; i += 1; }
        } else if (0xA1..=0xDF).contains(&c) { s += 1; i += 1; }
        else { s -= 1; i += 1; }
    }
    s
}

fn calc_euc(b: &[u8]) -> i32 {
    let mut s = 0; 
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c <= 0x7F { i += 1; }
        else if (0xA1..=0xFE).contains(&c) {
            if i + 1 < b.len() && (0xA1..=0xFE).contains(&b[i+1]) { s += 1; i += 2; }
            else { s -= 1; i += 1; }
        } else if c == 0x8E {
            if i + 1 < b.len() && (0xA1..=0xDF).contains(&b[i+1]) { s += 1; i += 2; }
            else { s -= 1; i += 1; }
        } else { s -= 1; i += 1; }
    }
    s
}

fn decode_to_utf8(bytes: &[u8]) -> String {
    let enc = detect_encoding(bytes);
    enc.decode(bytes).0.into_owned()
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
                    32 => "別のプロセスがファイルを使用中です".to_string(),
                    123 => "ファイル名、ディレクトリ名、またはボリュームラベルの構文が間違っています".to_string(),
                    _ => format!("{} (エラーコード: {})", e, code),
                };
            }
            e.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assign_command_ids, build_label_index, expand_globs, parse_args, parse_script,
        process_input_lines, process_lines, resolve_path_case_insensitive, Command, InputLine,
        Options, SedState,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sed-tests-{name}-{unique}"))
    }

    fn run_script(script: &str, input: &[&str], opts: Options) -> String {
        let mut commands = parse_script(script).unwrap();
        assign_command_ids(&mut commands);
        let labels = build_label_index(&commands);
        let mut out = Vec::new();
        process_lines(input, &commands, &labels, &opts, &mut out, "\n").unwrap();
        String::from_utf8(out).unwrap()
    }

    fn run_script_split(script: &str, chunks: &[Vec<&str>], opts: Options) -> String {
        let mut commands = parse_script(script).unwrap();
        assign_command_ids(&mut commands);
        let labels = build_label_index(&commands);
        let mut out = Vec::new();
        let mut state = SedState::new();

        for chunk in chunks {
            let input_lines: Vec<InputLine> = chunk
                .iter()
                .map(|line| InputLine { text: (*line).to_string(), line_ending: "\n" })
                .collect();
            process_input_lines(&input_lines, &commands, &labels, &opts, &mut out, &mut state).unwrap();
        }

        String::from_utf8(out).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn resolve_existing_path_matches_case_insensitively() {
        let root = test_dir("case-insensitive");
        fs::create_dir_all(&root).unwrap();

        let original = root.join("MiXeD.TXT");
        fs::write(&original, "hello").unwrap();

        let resolved = resolve_path_case_insensitive(&root.join("mixed.txt").to_string_lossy());

        assert!(resolved.exists());
        assert_eq!(fs::read_to_string(&resolved).unwrap(), "hello");

        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn expand_globs_uses_posix_like_separator_and_dot_rules() {
        let root = test_dir("glob-rules");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();

        let visible = root.join("Top.TXT");
        let hidden = root.join(".hidden.txt");
        let nested_file = nested.join("child.txt");

        fs::write(&visible, "top").unwrap();
        fs::write(&hidden, "hidden").unwrap();
        fs::write(&nested_file, "nested").unwrap();

        let pattern = root.join("*.txt").to_string_lossy().to_string();
        let expanded = expand_globs(vec![pattern]);

        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].ends_with("Top.TXT"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn substitution_defaults_to_basic_regular_expressions() {
        let output = run_script(r"s/\(ab\)/X/", &["ab"], Options::default());
        assert_eq!(output, "X\n");
    }

    #[test]
    fn substitution_uses_extended_regular_expressions_with_e_flag() {
        let opts = Options { extended_regex: true, ..Options::default() };
        let output = run_script(r"s/(ab)/X/", &["ab"], opts);
        assert_eq!(output, "X\n");
    }

    #[test]
    fn basic_regular_expressions_support_backreferences_in_patterns() {
        let output = run_script(r"s/\(ab\)\1/X/", &["abab"], Options::default());
        assert_eq!(output, "X\n");
    }

    #[test]
    fn extended_regular_expressions_support_backreferences_in_patterns() {
        let opts = Options { extended_regex: true, ..Options::default() };
        let output = run_script(r"s/(ab)\1/X/", &["abab"], opts);
        assert_eq!(output, "X\n");
    }

    #[test]
    fn basic_regular_expressions_do_not_treat_backslashed_plus_as_a_quantifier() {
        let output = run_script(r"s/a\+/X/", &["aa"], Options::default());
        assert_eq!(output, "aa\n");
    }

    #[test]
    fn basic_regular_expressions_do_not_treat_backslashed_pipe_as_alternation() {
        let output = run_script(r"s/a\|b/X/", &["a"], Options::default());
        assert_eq!(output, "a\n");
    }

    #[test]
    fn extended_regular_expressions_treat_plus_and_pipe_as_operators() {
        let opts = Options { extended_regex: true, ..Options::default() };
        let output = run_script(r"s/a+|b/X/g", &["aa b"], opts);
        assert_eq!(output, "X X\n");
    }

    #[test]
    fn address_ranges_stop_when_second_numeric_address_is_reached() {
        let output = run_script("2,3p", &["a", "b", "c", "d"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "b\nc\n");
    }

    #[test]
    fn regex_end_address_is_not_checked_on_range_start_line() {
        let output = run_script("1,/foo/p", &["foo", "bar", "foo", "baz"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "foo\nbar\nfoo\n");
    }

    #[test]
    fn empty_substitute_pattern_reuses_previous_regex() {
        let output = run_script(r"s/foo/X/; s//Y/", &["foofoo"], Options::default());
        assert_eq!(output, "XY\n");
    }

    #[test]
    fn empty_address_regex_reuses_previous_regex() {
        let output = run_script(r"/foo/p; //p", &["foo", "bar"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "foo\nfoo\n");
    }

    #[test]
    fn empty_regex_without_previous_pattern_is_an_error() {
        assert!(parse_script(r"s//X/").is_err());
        assert!(parse_script(r"//p").is_err());
    }

    #[test]
    fn replacement_ampersand_inserts_the_full_match() {
        let output = run_script(r"s/foo/[&]/", &["foo"], Options::default());
        assert_eq!(output, "[foo]\n");
    }

    #[test]
    fn escaped_ampersand_is_treated_literally_in_replacement() {
        let output = run_script(r"s/foo/\&/", &["foo"], Options::default());
        assert_eq!(output, "&\n");
    }

    #[test]
    fn escaped_backslash_is_treated_literally_in_replacement() {
        let output = run_script(r"s/foo/\\\\/", &["foo"], Options::default());
        assert_eq!(output, "\\\\\n");
    }

    #[test]
    fn dollar_sign_is_treated_literally_in_replacement() {
        let output = run_script(r"s/foo/$1/", &["foo"], Options::default());
        assert_eq!(output, "$1\n");
    }

    #[test]
    fn replacement_backreferences_still_work() {
        let output = run_script(r"s/\(foo\)/<\1>/", &["foo"], Options::default());
        assert_eq!(output, "<foo>\n");
    }

    #[test]
    fn line_address_zero_is_rejected() {
        assert!(parse_script("0p").is_err());
        assert!(parse_script("0,2p").is_err());
    }

    #[test]
    fn step_address_requires_a_positive_step() {
        assert!(parse_script("1~0p").is_err());
    }

    #[test]
    fn append_text_preserves_leading_and_trailing_spaces() {
        let output = run_script("a\\  keep both  ", &["x"], Options::default());
        assert_eq!(output, "x\n  keep both  \n");
    }

    #[test]
    fn append_text_treats_semicolon_as_literal_text() {
        let output = run_script("a\\x;y", &["z"], Options::default());
        assert_eq!(output, "z\nx;y\n");
    }

    #[test]
    fn append_text_supports_backslash_newline_form() {
        let output = run_script("a\\\nhello", &["z"], Options::default());
        assert_eq!(output, "z\nhello\n");
    }

    #[test]
    fn l_command_escapes_special_characters_and_marks_end_of_line() {
        let output = run_script("l", &["a\tb\\$"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "a\\tb\\\\\\$$\n");
    }

    #[test]
    fn l_command_uses_named_and_octal_escapes_for_non_printable_characters() {
        let output = run_script("l", &["\u{0007}\u{0001}"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "\\a\\001$\n");
    }

    #[test]
    fn q_command_accepts_an_optional_exit_code() {
        let commands = parse_script("q42").unwrap();
        match &commands[0].command {
            Command::QuitWithCode(code) => assert_eq!(*code, 42),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn q_uppercase_accepts_an_optional_silent_exit_code() {
        let commands = parse_script("Q7").unwrap();
        match &commands[0].command {
            Command::QuitSilentWithCode(code) => assert_eq!(*code, 7),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn q_command_rejects_trailing_garbage() {
        assert!(parse_script("q1x").is_err());
        assert!(parse_script("Q9oops").is_err());
    }

    #[test]
    fn n_p_d_cycle_supports_sliding_two_line_windows() {
        let output = run_script("N;P;D", &["a", "b", "c"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "a\nb\n");
    }

    #[test]
    fn p_prints_only_the_first_line_of_a_multiline_pattern_space() {
        let output = run_script("N;P", &["a", "b"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "a\n");
    }

    #[test]
    fn d_restarts_the_cycle_with_the_remaining_multiline_pattern_space() {
        let output = run_script("N;D", &["a", "b"], Options::default());
        assert_eq!(output, "b\n");
    }

    #[test]
    fn next_at_end_of_input_prints_current_pattern_space_once() {
        let output = run_script("n", &["a"], Options::default());
        assert_eq!(output, "a\n");
    }

    #[test]
    fn next_at_end_of_input_is_silent_under_quiet_mode() {
        let output = run_script("n", &["a"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "");
    }

    #[test]
    fn next_append_at_end_of_input_prints_current_pattern_space() {
        let output = run_script("N", &["a"], Options::default());
        assert_eq!(output, "a\n");
    }

    #[test]
    fn in_place_option_accepts_arbitrary_backup_suffix_as_next_argument() {
        let args = vec![
            "sed".to_string(),
            "-i".to_string(),
            "backup".to_string(),
            "s/x/y/".to_string(),
            "file.txt".to_string(),
        ];
        let (opts, files) = parse_args(&args).unwrap();
        assert!(opts.in_place);
        assert_eq!(opts.in_place_backup.as_deref(), Some("backup"));
        assert_eq!(opts.expressions, vec!["s/x/y/"]);
        assert_eq!(files, vec!["file.txt"]);
    }

    #[test]
    fn in_place_without_suffix_keeps_next_script_argument_available() {
        let args = vec![
            "sed".to_string(),
            "-i".to_string(),
            "-e".to_string(),
            "s/x/y/".to_string(),
            "file.txt".to_string(),
        ];
        let (opts, files) = parse_args(&args).unwrap();
        assert!(opts.in_place);
        assert_eq!(opts.in_place_backup, None);
        assert_eq!(opts.expressions, vec!["s/x/y/"]);
        assert_eq!(files, vec!["file.txt"]);
    }

    #[test]
    fn unknown_short_options_are_rejected() {
        let args = vec!["sed".to_string(), "-x".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn unknown_combined_short_options_are_rejected() {
        let args = vec!["sed".to_string(), "-nzx".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn basic_regular_expressions_support_posix_character_classes() {
        let output = run_script(r"s/[[:digit:]][[:digit:]]/X/g", &["ab12cd34"], Options::default());
        assert_eq!(output, "abXcdX\n");
    }

    #[test]
    fn basic_regular_expression_conversion_preserves_bracket_expression_literals() {
        let output = run_script(r"s/[\(\)\{\}\+\?\|]/X/g", &["(){}+?|"], Options::default());
        assert_eq!(output, "XXXXXXX\n");
    }

    #[test]
    fn basic_regular_expression_conversion_preserves_negated_bracket_expressions() {
        let output = run_script(r"s/[^[:digit:]]/X/g", &["a1b2"], Options::default());
        assert_eq!(output, "X1X2\n");
    }

    #[test]
    fn extended_regular_expressions_support_posix_character_classes() {
        let opts = Options { extended_regex: true, ..Options::default() };
        let output = run_script(r"s/[[:alpha:]]+/X/g", &["ab12cd"], opts);
        assert_eq!(output, "X12X\n");
    }

    #[test]
    fn address_regex_supports_posix_character_classes() {
        let output = run_script(r"/^[[:digit:]]\{3\}$/p", &["12", "123", "1234"], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "123\n");
    }

    #[test]
    fn substitute_command_rejects_unknown_flags() {
        assert!(parse_script(r"s/a/b/x").is_err());
    }

    #[test]
    fn write_command_treats_semicolon_as_part_of_the_file_name() {
        let commands = parse_script("w foo;bar").unwrap();
        match &commands[0].command {
            Command::WriteFile(path) => assert_eq!(path, "foo;bar"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn read_command_treats_closing_brace_as_part_of_the_file_name_until_newline() {
        let commands = parse_script("r foo}bar").unwrap();
        match &commands[0].command {
            Command::ReadFile(path) => assert_eq!(path, "foo}bar"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn labels_can_contain_non_alphanumeric_characters_until_command_terminator() {
        let commands = parse_script(":foo-bar\nb foo-bar").unwrap();
        match &commands[0].command {
            Command::Label(label) => assert_eq!(label, "foo-bar"),
            other => panic!("unexpected command: {other:?}"),
        }
        match &commands[1].command {
            Command::Branch(label) => assert_eq!(label, "foo-bar"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn branch_without_label_remains_valid() {
        let commands = parse_script("b").unwrap();
        match &commands[0].command {
            Command::Branch(label) => assert!(label.is_empty()),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn unterminated_substitute_command_is_rejected() {
        assert!(parse_script(r"s/foo/bar").is_err());
    }

    #[test]
    fn unterminated_address_regex_is_rejected() {
        assert!(parse_script("/foo p").is_err());
    }

    #[test]
    fn unterminated_translate_command_is_rejected() {
        assert!(parse_script("y/ab").is_err());
    }

    #[test]
    fn default_processing_keeps_line_numbers_across_input_chunks() {
        let output = run_script_split("3p", &[vec!["a", "b"], vec!["c", "d"]], Options { quiet: true, ..Options::default() });
        assert_eq!(output, "c\n");
    }

    #[test]
    fn separate_processing_resets_line_numbers_for_each_chunk() {
        let first = run_script("3p", &["a", "b"], Options { quiet: true, ..Options::default() });
        let second = run_script("3p", &["c", "d"], Options { quiet: true, ..Options::default() });
        assert_eq!(first, "");
        assert_eq!(second, "");
    }
}
