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

/// スクリプト断片の出所。エラー報告でどの -e / -f に由来するか特定するために使う。
#[derive(Debug, Clone)]
struct ScriptOrigin {
    /// 表示用ラベル。例: "-e #1", "ファイル 'script.sed'"
    label: String,
    /// ユーザが渡した原文（連結用の '\n' は含まない）
    source: String,
    /// 連結後スクリプト中での開始文字位置
    start: usize,
}

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    quiet: bool,              // -n: 自動出力を抑制
    expressions: Vec<String>, // -e: スクリプト式
    script_files: Vec<String>,// -f: スクリプトファイル
    
    // GNU拡張オプション
    in_place: bool,           // -i: インプレース編集
    in_place_backup: Option<String>, // -iSUFFIX: バックアップサフィックス
    extended_regex: bool,     // -E, -r: 拡張正規表現
    separate: bool,           // -s: ファイルを個別に処理
    unbuffered: bool,         // -u: バッファなし
    follow_symlinks: bool,    // --follow-symlinks: シンボリックリンクをたどる
    null_data: bool,          // -z: NUL区切り
    line_length: Option<usize>, // -l N: l コマンドの折り返し幅（None = デフォルト 70）
    sandbox: bool,            // --sandbox: e/r/w 系コマンドを禁止
    posix: bool,              // --posix: GNU拡張を抑制（受理のみ、動作は概ね共通）

    show_help: bool,
    show_version: bool,
}

#[derive(Debug, Clone)]
enum Address {
    Line(usize),
    LastLine,
    Regex {
        pattern: String,
        ignore_case: bool, // /pat/I
        multiline: bool,   // /pat/M
    },
    Step(usize, usize), // first~step
    Zero,               // 0 （0,/regexp/ 専用・GNU拡張）
    RelOffset(usize),   // addr1,+N （GNU拡張）
    Multiple(usize),    // addr1,~N （GNU拡張）
}

#[derive(Debug, Clone)]
enum Command {
    Substitute { pattern: String, replacement: String, flags: SubstituteFlags },
    Translate { src: Vec<char>, dst: Vec<char> },
    Delete,
    DeleteFirstLine,
    Print,
    PrintFirstLine,
    PrintUnambiguous(Option<usize>), // l [幅]
    PrintLineNum,
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
    /// フラット化後のブロック開始。値は「ブロックの次」の命令インデックス。
    /// アドレスが一致しなければそこへジャンプする。
    BlockBegin(usize),
    // GNU拡張
    ZapPattern,             // z: パターンスペースをクリア
    ReadLineFile(String),   // R: ファイルから1行ずつ読み込んで追加
    WriteFirstLine(String), // W: パターンスペースの最初の行をファイルに書き込み
    PrintFileName,          // F: 現在の入力ファイル名を出力
    Execute(String),        // e [コマンド]: コマンド実行（空ならパターンスペースを実行）
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

/// 活性化中のアドレス範囲の状態
#[derive(Debug, Clone)]
struct ActiveRange {
    /// +N / ~N / 行番号終端用に活性化時に確定した終了行（この行に達したら閉じる）
    end_line: Option<usize>,
}

struct SedState {
    pattern_space: String,
    hold_space: String,
    line_num: usize,
    substitution_made: bool,
    append_queue: Vec<String>,
    active_ranges: HashMap<usize, ActiveRange>,
    /// 0,/regexp/ の範囲を一度開始したコマンド（再開始しない）
    zero_done: std::collections::HashSet<usize>,
    /// w コマンド / s///w の出力先。起動後最初の書き込みで truncate し、以後追記（GNU 互換）
    write_files: HashMap<String, WriteTarget>,
    /// R コマンドの読み込み状態（ファイル内容と現在位置）
    read_line_files: HashMap<String, (Vec<String>, usize)>,
    /// F コマンド用の現在の入力ファイル名
    current_file: std::rc::Rc<String>,
}

/// w コマンドの出力先
enum WriteTarget {
    Stdout,
    Stderr,
    File(File),
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
            zero_done: std::collections::HashSet::new(),
            write_files: HashMap::new(),
            read_line_files: HashMap::new(),
            current_file: std::rc::Rc::new("-".to_string()),
        }
    }
}

struct InputLine {
    text: String,
    line_ending: &'static str,
    /// この行の入力元ファイル名（F コマンド用）
    file: std::rc::Rc<String>,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let (mut opts, files) = match parse_args(&args) {
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
        println!("sed (Rust版) 2.2.0");
        println!("POSIX.1-2017 寄り + GNU/Windows 拡張");
        std::process::exit(0);
    }

    // スクリプトを構築（エラー報告のために断片の出所を記録する）
    let mut script = String::new();
    let mut origins: Vec<ScriptOrigin> = Vec::new();
    for (i, expr) in opts.expressions.iter().enumerate() {
        if !script.is_empty() { script.push('\n'); }
        // Windowsでシングルクォートが残っている場合は除去
        let expr = expr.trim_matches('\'');
        let start = script.chars().count();
        script.push_str(expr);
        origins.push(ScriptOrigin {
            label: format!("-e #{}", i + 1),
            source: expr.to_string(),
            start,
        });
    }

    for script_file in &opts.script_files {
        let script_path = resolve_path_case_insensitive(script_file);
        match fs::read_to_string(&script_path) {
            Ok(content) => {
                if !script.is_empty() { script.push('\n'); }
                let start = script.chars().count();
                script.push_str(&content);
                origins.push(ScriptOrigin {
                    label: format!("ファイル '{}'", script_file),
                    source: content,
                    start,
                });
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

    // POSIX: スクリプトの最初の2文字が "#n" なら -n と同じ（GNU 4.9 も先頭2文字のみで判定）
    if script.starts_with("#n") {
        opts.quiet = true;
    }

    let mut commands = match parse_script_at(&script) {
        Ok(cmds) => cmds,
        Err((pos, msg)) => {
            eprintln!("sed: {}", format_script_error(pos, &msg, &origins));
            std::process::exit(1);
        }
    };
    assign_command_ids(&mut commands);
    let commands = flatten_commands(commands);
    let labels = build_label_index(&commands);

    // 正規表現・ラベル・サンドボックスの事前検証
    if let Err(msg) = validate_commands(&commands, &labels, &opts) {
        eprintln!("sed: {}", msg);
        std::process::exit(1);
    }

    // glob展開
    let files = expand_globs(files);

    // GNU 互換: -i は入力ファイルが必要
    if opts.in_place && files.is_empty() {
        eprintln!("sed: -i は標準入力に対しては使用できません（入力ファイルを指定してください）");
        std::process::exit(2);
    }

    std::process::exit(run(&files, &commands, &labels, &opts));
}

/// 入力の処理全体。終了コードを返す。
fn run(files: &[String], commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> i32 {
    if files.is_empty() {
        return match process_stdin(commands, labels, opts) {
            Ok(Some(code)) => code,
            Ok(None) => 0,
            Err(e) => {
                eprintln!("sed: {}", format_error(&e));
                1
            }
        };
    }

    if !opts.in_place && !opts.separate {
        // 連続ストリーム処理: 読めないファイルは警告してスキップし、処理は続行（GNU 互換）
        let mut lines = Vec::new();
        let mut had_error = false;
        for path in files {
            match read_file_lines(path, opts) {
                Ok(l) => lines.extend(l),
                Err(e) => {
                    eprintln!("sed: '{}' を読み込めません: {}", path, format_error(&e));
                    had_error = true;
                }
            }
        }
        let mut out = io::stdout().lock();
        let mut state = SedState::new();
        return match process_input_lines(&lines, commands, labels, opts, &mut out, &mut state) {
            Ok(Some(code)) => code,
            Ok(None) => if had_error { 1 } else { 0 },
            Err(e) => {
                eprintln!("sed: {}", format_error(&e));
                1
            }
        };
    }

    // -i / -s: ファイルごとに個別処理
    let mut exit_code = 0;
    for file in files {
        match process_file(file, commands, labels, opts) {
            // q/Q はその時点で全体を終了する（GNU 互換）
            Ok(Some(code)) => return code,
            Ok(None) => {}
            Err(e) => {
                eprintln!("sed: '{}': {}", file, format_error(&e));
                exit_code = 1;
            }
        }
    }
    exit_code
}

/// 実行前の静的検証: 正規表現がコンパイルできるか、分岐先ラベルが存在するか、
/// --sandbox で禁止コマンドが使われていないか。
fn validate_commands(commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> Result<(), String> {
    fn check_regex(pattern: &str, ignore_case: bool, multiline: bool, opts: &Options) -> Result<(), String> {
        build_regex(pattern, ignore_case, multiline, opts)
            .map(|_| ())
            .map_err(|e| format!("正規表現 '{}' が不正です: {}", pattern, e))
    }

    fn check_addr(addr: &Option<Address>, opts: &Options) -> Result<(), String> {
        if let Some(Address::Regex { pattern, ignore_case, multiline }) = addr {
            check_regex(pattern, *ignore_case, *multiline, opts)?;
        }
        Ok(())
    }

    for cmd in commands {
        check_addr(&cmd.addr1, opts)?;
        check_addr(&cmd.addr2, opts)?;

        match &cmd.command {
            Command::Substitute { pattern, flags, .. } => {
                check_regex(pattern, flags.ignore_case, flags.multiline, opts)?;
                if opts.sandbox && flags.write_file.is_some() {
                    return Err("サンドボックスモードでは s///w は使用できません".to_string());
                }
            }
            Command::Branch(l) | Command::Test(l) | Command::TestNot(l) => {
                if !l.is_empty() && !labels.contains_key(l) {
                    return Err(format!("ラベル '{}' が見つかりません", l));
                }
            }
            Command::ReadFile(_) | Command::WriteFile(_) | Command::ReadLineFile(_)
            | Command::WriteFirstLine(_) | Command::Execute(_)
                if opts.sandbox =>
            {
                return Err("サンドボックスモードでは e/r/w/R/W コマンドは使用できません".to_string());
            }
            _ => {}
        }
    }
    Ok(())
}

/// 1 ファイル分の入力行を読み込む（"-" は標準入力）
fn read_file_lines(path: &str, opts: &Options) -> io::Result<Vec<InputLine>> {
    if path == "-" {
        let mut buf = Vec::new();
        io::stdin().lock().read_to_end(&mut buf)?;
        return Ok(decode_input_lines(&buf, opts, "-"));
    }
    let p = resolve_path_case_insensitive(path);
    if p.is_dir() {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "ディレクトリです"));
    }
    let mut buf = Vec::new();
    File::open(&p)?.read_to_end(&mut buf)?;
    Ok(decode_input_lines(&buf, opts, path))
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
            // GNU 互換: バックアップサフィックスは -i.bak / --in-place=.bak の
            // 結合形式のみ。次の引数をサフィックスとして消費しない
            // （`sed -i 's/a/b/' file` を壊さないため）。
            "-i" | "--in-place" => opts.in_place = true,
            "-s" | "--separate" => opts.separate = true,
            "-u" | "--unbuffered" => opts.unbuffered = true,
            "-z" | "--null-data" => opts.null_data = true,
            "-l" | "--line-length" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-l' には引数が必要です".to_string());
                }
                let n: usize = args[i]
                    .parse()
                    .map_err(|_| format!("'-l' の引数が不正です: '{}'", args[i]))?;
                opts.line_length = Some(n);
            }
            "--posix" => opts.posix = true,
            "--sandbox" => opts.sandbox = true,
            "--debug" => {} // 互換のため受理して無視
            "--follow-symlinks" => opts.follow_symlinks = true,
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            // --expression=SCRIPT / --file=FILE
            s if s.starts_with("--expression=") => {
                opts.expressions.push(s["--expression=".len()..].to_string());
                has_script = true;
            }
            "--expression" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '--expression' には引数が必要です".to_string());
                }
                opts.expressions.push(args[i].clone());
                has_script = true;
            }
            s if s.starts_with("--file=") => {
                opts.script_files.push(s["--file=".len()..].to_string());
                has_script = true;
            }
            "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '--file' には引数が必要です".to_string());
                }
                opts.script_files.push(args[i].clone());
                has_script = true;
            }
            s if s.starts_with("--line-length=") => {
                let v = &s["--line-length=".len()..];
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("'--line-length' の引数が不正です: '{}'", v))?;
                opts.line_length = Some(n);
            }
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
                 ※ サフィックスは -i.bak のような結合形式のみ（GNU互換）
  -s, --separate ファイルを連続ストリームとしてではなく個別に処理
  -u, --unbuffered
                 入力ファイルから最小限のデータを読み込み、より頻繁に出力バッファをフラッシュ
  -z, --null-data
                 改行の代わりにNUL文字で行を区切る
  -l N, --line-length=N
                 l コマンドの折り返し幅（デフォルト 70、0 で折り返しなし）
      --posix    POSIX 準拠モード（受理のみ）
      --sandbox  e/r/w/R/W コマンドを禁止する
      --debug    互換のため受理（無視）
      --follow-symlinks
                 -i使用時にシンボリックリンクをたどる
      --help     このヘルプを表示して終了
      --version  バージョン情報を表示して終了

スクリプトの最初の2文字が #n の場合は -n と同じ動作になります（POSIX）。

-e または -f がない場合、最初の非オプション引数がsedスクリプトとして使用されます。
残りの引数は入力ファイル名として解釈されます。ファイル指定がない場合は標準入力を読み込みます。
Windows では `*`, `?`, `[]` を内部で展開します。先頭 `.` は明示した場合のみマッチし、既存ファイル名の解決は大小無視です。

アドレス:
  number         指定行番号
  $              最終行
  /regexp/       正規表現にマッチする行
  /regexp/I      大文字小文字を区別しない（GNU拡張、M でマルチライン）
  first~step     first行目から、step行ごと（GNU拡張）
  0,/regexp/     1行目から終端判定を行う範囲（GNU拡張）
  addr,+N        addr から N 行後まで（GNU拡張）
  addr,~N        addr から次の N の倍数行まで（GNU拡張）

コマンド:
  s/regexp/replacement/flags
                 置換（flags: g=全置換, p=出力, i=大小無視, m=マルチライン,
                       N=N番目（Ng で N番目以降すべて）, w FILE=ファイル出力）
                 replacement では \1-\9, &, \U \L \u \l \E（大小変換）が使える
  y/src/dst/     文字変換（1対1、'-' はリテラル）
  d              パターンスペースを削除
  D              パターンスペースの最初の行を削除しサイクルを再開
  p              パターンスペースを出力
  P              パターンスペースの最初の行を出力
  l [width]      パターンスペースを曖昧さなく出力（width 文字で折り返し）
  =              行番号を出力
  n              次の行を読み込み
  N              次の行をパターンスペースに追加
  q [exit-code]  終了
  Q [exit-code]  無出力で終了（GNU拡張）
  h H            パターンスペースをホールドスペースにコピー/追加
  g G            ホールドスペースをパターンスペースにコピー/追加
  x              パターンスペースとホールドスペースを交換
  a\text         textを後に追加（行末 \ で複数行継続可）
  i\text         textを前に挿入
  c\text         パターンスペースをtextで置換（範囲では最終行に一度だけ出力）
  r file         ファイルの内容を追加
  R file         ファイルから1行ずつ読み込んで追加（GNU拡張）
  w file         パターンスペースをファイルに書き込み
                 （/dev/stdout, /dev/stderr も指定可）
  W file         パターンスペースの最初の行をファイルに書き込み（GNU拡張）
  F              現在の入力ファイル名を出力（GNU拡張）
  e [command]    コマンドを実行（GNU拡張、引数なしはパターンスペースを実行）
  b [label]      ラベルに分岐
  t [label]      置換成功時にラベルに分岐
  T [label]      置換失敗時にラベルに分岐（GNU拡張）
  :label         ラベル定義
  {{ commands }}   コマンドのグループ化
  z              パターンスペースをクリア（GNU拡張）
  v [version]    バージョン要求（受理のみ、GNU拡張）

正規表現では \< \>（単語境界）も使えます。

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

#[cfg(test)]
fn parse_script(script: &str) -> Result<Vec<SedCommand>, String> {
    parse_script_at(script).map_err(|(_, m)| m)
}

/// `parse_script` の位置情報付きバージョン。エラー時に「連結後スクリプト中の文字位置」を返す。
fn parse_script_at(script: &str) -> Result<Vec<SedCommand>, (usize, String)> {
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
        let cmd_start = pos;
        match parse_sed_command(&chars, &mut pos) {
            Ok(cmd) => commands.push(cmd),
            Err(msg) => {
                // pos はエラー検知時点で 1〜数文字進んでいることが多い。
                // 直前のコマンド開始位置以降に丸めた上で 1 文字戻す（不明コマンド系で最も役に立つ位置になる）。
                let err_pos = pos.saturating_sub(1).max(cmd_start).min(chars.len());
                return Err((err_pos, msg));
            }
        }
    }
    normalize_empty_regexes(&mut commands).map_err(|m| (pos, m))?;
    Ok(commands)
}

/// 連結後スクリプト中の文字位置 `pos` から、対応する `ScriptOrigin` を引き当てる。
fn locate_origin(pos: usize, origins: &[ScriptOrigin]) -> Option<(&ScriptOrigin, usize)> {
    // origins は start で昇順かつ非重複である前提（main() で連結順に作っている）。
    // pos に対して start <= pos となる最後の origin を採用する。
    let mut chosen: Option<&ScriptOrigin> = None;
    for origin in origins {
        if origin.start <= pos {
            chosen = Some(origin);
        } else {
            break;
        }
    }
    chosen.map(|o| {
        let source_len = o.source.chars().count();
        let local = pos.saturating_sub(o.start).min(source_len);
        (o, local)
    })
}

/// パースエラーを「どの -e のどの位置か」を示す形に整形し、必要に応じて MSYS ヒントを付ける。
fn format_script_error(pos: usize, msg: &str, origins: &[ScriptOrigin]) -> String {
    let mut out = String::new();
    let located = locate_origin(pos, origins);
    if let Some((origin, local)) = located {
        out.push_str(&format!("{}, 位置 {}: {}", origin.label, local + 1, msg));
        // 該当行と矢印を表示（source が複数行のときは局所的に切り出す）
        let line_info = line_and_column(&origin.source, local);
        out.push('\n');
        out.push_str(&format!("  {}\n", line_info.line));
        out.push_str(&format!("  {}^", " ".repeat(line_info.column)));
        if let Some(hint) = msys_hint_if_applicable(msg, &origin.source) {
            out.push('\n');
            out.push_str(&hint);
        }
    } else {
        out.push_str(msg);
    }
    out
}

struct LineInfo<'a> {
    line: &'a str,
    /// 当該行内での 0 始まりカラム位置（文字単位）
    column: usize,
}

/// `source` 内の `char_offset` 位置を含む行と、その行内カラムを返す。
fn line_and_column(source: &str, char_offset: usize) -> LineInfo<'_> {
    let mut chars_seen = 0usize;
    for line in source.split('\n') {
        let line_chars = line.chars().count();
        // この行の文字範囲は [chars_seen, chars_seen + line_chars]。
        // +1 は改行ぶんだが、最終行は改行を持たないため offset = end_of_input でもこの行に当てる。
        if char_offset <= chars_seen + line_chars {
            return LineInfo {
                line,
                column: char_offset.saturating_sub(chars_seen),
            };
        }
        chars_seen += line_chars + 1; // 改行ぶん
    }
    LineInfo {
        line: source,
        column: char_offset.min(source.chars().count()),
    }
}

/// Windows + Git Bash の MSYS 自動パス変換が原因と思われるエラーに対し、ユーザ向けヒントを返す。
///
/// 条件:
///   * Windows 上で実行されている
///   * 「不明なコマンド」エラーである
///   * 該当する -e の原文が `<drive>:\...` または `<drive>:/...` で始まる
fn msys_hint_if_applicable(msg: &str, source: &str) -> Option<String> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    if !msg.starts_with("不明なコマンド") {
        return None;
    }
    let bytes = source.as_bytes();
    let looks_like_windows_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if !looks_like_windows_path {
        return None;
    }
    Some(
        "ヒント: Git Bash / MSYS の自動パス変換により '/...' が Windows パスへ書き換えられた可能性があります。\n  \
         回避策: 環境変数 MSYS_NO_PATHCONV=1 を設定するか、'//pat/' のように先頭スラッシュを二重にしてください。"
            .to_string()
    )
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
        if let Some(Address::Regex { pattern, .. }) = addr {
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
    let addr2 = if addr1.is_some() && *pos < chars.len() && chars[*pos] == ',' {
        *pos += 1;
        skip_ws(chars, pos);
        // GNU拡張: addr1,+N（N行後まで）と addr1,~N（次のNの倍数行まで）
        if *pos < chars.len() && (chars[*pos] == '+' || chars[*pos] == '~') {
            let rel = chars[*pos] == '+';
            *pos += 1;
            let mut s = String::new();
            while *pos < chars.len() && chars[*pos].is_ascii_digit() {
                s.push(chars[*pos]); *pos += 1;
            }
            let n: usize = s.parse().map_err(|_| "無効な数値です".to_string())?;
            if !rel && n == 0 {
                return Err("~ の値は 1 以上でなければなりません".to_string());
            }
            Some(if rel { Address::RelOffset(n) } else { Address::Multiple(n) })
        } else {
            Some(parse_address(chars, pos)?.unwrap_or(Address::LastLine))
        }
    } else { None };

    // 0,/regexp/ の検証: アドレス 0 は正規表現終端との組み合わせのみ有効
    if matches!(addr1, Some(Address::Zero)) {
        match &addr2 {
            Some(Address::Regex { .. }) => {}
            _ => return Err("行番号 0 は 0,/regexp/ の形式でのみ使用できます".to_string()),
        }
    }

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
            if n == 0 {
                return Err("行番号は 1 以上でなければなりません".to_string());
            }
            return Ok(Some(Address::Step(n, step)));
        }
        if n == 0 {
            // 0,/regexp/ 用（parse_sed_command 側で検証する）
            return Ok(Some(Address::Zero));
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
        let pattern = read_delim(chars, pos, delim)?;
        // GNU拡張: アドレス正規表現の I（大小無視）/ M（マルチライン）フラグ
        let mut ignore_case = false;
        let mut multiline = false;
        while *pos < chars.len() {
            match chars[*pos] {
                'I' => { ignore_case = true; *pos += 1; }
                'M' => { multiline = true; *pos += 1; }
                _ => break,
            }
        }
        return Ok(Some(Address::Regex { pattern, ignore_case, multiline }));
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
        'l' => {
            skip_ws(chars, pos);
            let width = read_optional_number(chars, pos)?.map(|n| n.max(0) as usize);
            ensure_command_terminator(chars, pos, "l")?;
            Ok(Command::PrintUnambiguous(width))
        }
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
        'R' => {
            skip_ws(chars, pos);
            let f = read_fname(chars, pos);
            if f.is_empty() {
                Err("ファイル名が必要です".to_string())
            } else {
                Ok(Command::ReadLineFile(f))
            }
        }
        'W' => {
            skip_ws(chars, pos);
            let f = read_fname(chars, pos);
            if f.is_empty() {
                Err("ファイル名が必要です".to_string())
            } else {
                Ok(Command::WriteFirstLine(f))
            }
        }
        'F' => Ok(Command::PrintFileName),
        'e' => {
            skip_ws(chars, pos);
            Ok(Command::Execute(read_fname(chars, pos)))
        }
        'v' => {
            // GNU拡張: バージョン要求。引数は受理して無視する
            skip_ws(chars, pos);
            let _ = read_label(chars, pos);
            Ok(Command::Block(Vec::new()))
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
        (false, None) => Ok(Command::QuitWithCode(0)),
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

/// y コマンドの文字列をエスケープ解決して文字列にする。
/// GNU sed 互換: '-' はリテラル（tr のような範囲展開は行わない）。
fn expand_range(s: &str) -> Vec<char> {
    let mut r = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && i + 1 < cs.len() {
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
    while *pos < chars.len() {
        let c = chars[*pos];
        if c == '\n' { break; }
        if c == '\\' {
            *pos += 1;
            if *pos >= chars.len() { break; } // 末尾の裸のバックスラッシュは無視
            let n = chars[*pos];
            *pos += 1;
            match n {
                // 行末のバックスラッシュ + 改行はテキストの継続（POSIX）
                '\n' => t.push('\n'),
                'n' => t.push('\n'),
                't' => t.push('\t'),
                'r' => t.push('\r'),
                other => t.push(other), // \\ → \ を含む
            }
        } else {
            t.push(c);
            *pos += 1;
        }
    }
    t
}

/// パース木のブロックをフラットな命令列に変換する。
/// これによりラベル分岐（ブロック内外への b/t/T）と D の全体再開が正しく動く。
fn flatten_commands(commands: Vec<SedCommand>) -> Vec<SedCommand> {
    fn rec(commands: Vec<SedCommand>, out: &mut Vec<SedCommand>) {
        for mut cmd in commands {
            if matches!(cmd.command, Command::Block(_)) {
                let Command::Block(inner) =
                    std::mem::replace(&mut cmd.command, Command::BlockBegin(0))
                else { unreachable!() };
                // アドレスなしの空ブロック（v コマンド等）は完全に無視する
                if inner.is_empty() && cmd.addr1.is_none() && cmd.addr2.is_none() && !cmd.negate {
                    continue;
                }
                let begin = out.len();
                out.push(cmd);
                rec(inner, out);
                let end = out.len();
                if let Command::BlockBegin(e) = &mut out[begin].command {
                    *e = end;
                }
            } else {
                out.push(cmd);
            }
        }
    }

    let mut out = Vec::new();
    rec(commands, &mut out);
    out
}

fn build_label_index(commands: &[SedCommand]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (i, cmd) in commands.iter().enumerate() {
        if let Command::Label(n) = &cmd.command {
            m.insert(n.clone(), i);
        }
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

fn process_stdin(commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> io::Result<Option<i32>> {
    let mut buf = Vec::new();
    io::stdin().lock().read_to_end(&mut buf)?;
    let lines = decode_input_lines(&buf, opts, "-");
    let mut out = io::stdout().lock();
    let mut state = SedState::new();
    process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)
}

fn process_file(path: &str, commands: &[SedCommand], labels: &HashMap<String, usize>, opts: &Options) -> io::Result<Option<i32>> {
    // "-" は標準入力
    if path == "-" {
        return process_stdin(commands, labels, opts);
    }

    let lines = read_file_lines(path, opts)?;
    let p = resolve_path_case_insensitive(path);

    if opts.in_place {
        if let Some(ref suf) = opts.in_place_backup {
            let backup_path = format!("{}{}", p.to_string_lossy(), suf);
            fs::copy(&p, &backup_path)?;
        }
        let mut out = Vec::new();
        let mut state = SedState::new();
        // q/Q でもここまでの出力はファイルに書き戻す（GNU 互換）
        let quit = process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)?;
        File::create(&p)?.write_all(&out)?;
        Ok(quit)
    } else {
        let mut out = io::stdout().lock();
        let mut state = SedState::new();
        process_input_lines(&lines, commands, labels, opts, &mut out, &mut state)
    }
}

fn decode_input_lines(bytes: &[u8], opts: &Options, file: &str) -> Vec<InputLine> {
    let file = std::rc::Rc::new(file.to_string());
    let content = decode_to_utf8(bytes);
    if opts.null_data {
        content
            .split('\0')
            .map(|line| InputLine { text: line.to_string(), line_ending: "\0", file: file.clone() })
            .collect()
    } else {
        let line_ending = detect_line_ending(&content);
        content
            .lines()
            .map(|line| InputLine { text: line.to_string(), line_ending, file: file.clone() })
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
                           opts: &Options, w: &mut W, _line_ending: &str) -> io::Result<Option<i32>> {
    let file = std::rc::Rc::new("-".to_string());
    let input_lines: Vec<InputLine> = lines
        .iter()
        .map(|line| InputLine { text: (*line).to_string(), line_ending: "\n", file: file.clone() })
        .collect();
    let mut state = SedState::new();
    process_input_lines(&input_lines, commands, labels, opts, w, &mut state)
}

/// 1 サイクル分の入力処理。q/Q による終了コードを Some で返す。
fn process_input_lines<W: Write>(lines: &[InputLine], commands: &[SedCommand], labels: &HashMap<String, usize>,
                                 opts: &Options, w: &mut W, state: &mut SedState) -> io::Result<Option<i32>> {
    let total = lines.len();
    let mut iter = lines.iter().enumerate().peekable();

    while let Some((idx, line)) = iter.next() {
        state.line_num += 1;
        state.pattern_space = line.text.clone();
        state.substitution_made = false;
        state.append_queue.clear();
        state.current_file = line.file.clone();
        let is_last = idx + 1 == total;
        let mut line_ending = line.line_ending;

        loop {
            let result = exec_cmds(commands, state, labels, opts, w, &mut iter, is_last, &mut line_ending)?;

            match result {
                ExecResult::Restart => {
                    // D: 新しい行を読まずにサイクルを再開（追加テキストは先に吐き出す）
                    flush_append_queue(state, w, line_ending)?;
                    continue;
                }
                ExecResult::Continue => {
                    if !opts.quiet { write!(w, "{}{}", state.pattern_space, line_ending)?; }
                    flush_append_queue(state, w, line_ending)?;
                }
                ExecResult::Delete => {
                    flush_append_queue(state, w, line_ending)?;
                }
                ExecResult::Quit(code) => {
                    if !opts.quiet { write!(w, "{}{}", state.pattern_space, line_ending)?; }
                    flush_append_queue(state, w, line_ending)?;
                    return Ok(Some(code));
                }
                ExecResult::QuitSilent(code) => return Ok(Some(code)),
                ExecResult::EndOfInput => return Ok(None),
            }
            break;
        }
    }
    Ok(None)
}

fn flush_append_queue<W: Write>(state: &mut SedState, w: &mut W, line_ending: &str) -> io::Result<()> {
    for t in std::mem::take(&mut state.append_queue) {
        write!(w, "{}{}", t, line_ending)?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum ExecResult {
    Continue,
    Delete,
    /// D: 新しい行を読まずにサイクルを再実行
    Restart,
    /// q [code]: パターンスペースを出力して終了
    Quit(i32),
    /// Q [code]: 出力せずに終了
    QuitSilent(i32),
    /// n が入力の終端に達した（正常終了扱い）
    EndOfInput,
}

fn exec_cmds<'a, W, I>(commands: &[SedCommand], state: &mut SedState, labels: &HashMap<String, usize>,
                       opts: &Options, w: &mut W, iter: &mut std::iter::Peekable<I>, mut is_last: bool,
                       line_ending: &mut &'static str) -> io::Result<ExecResult>
where W: Write, I: Iterator<Item = (usize, &'a InputLine)> {
    let mut ci = 0;
    while ci < commands.len() {
        let cmd = &commands[ci];

        // ブロック開始: アドレス不一致ならブロックの外へジャンプ
        if let Command::BlockBegin(end) = &cmd.command {
            let (matched, _) = addr_match(cmd, state, is_last, opts);
            ci = if matched { ci + 1 } else { *end };
            continue;
        }

        let (matched, range_end) = addr_match(cmd, state, is_last, opts);
        if !matched { ci += 1; continue; }

        match &cmd.command {
            Command::Substitute { pattern, replacement, flags } => {
                let (new, ok) = apply_subst(&state.pattern_space, pattern, replacement, flags, opts);
                state.pattern_space = new;
                if ok {
                    state.substitution_made = true;
                    if flags.print { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
                    if let Some(ref f) = flags.write_file {
                        let text = state.pattern_space.clone();
                        write_to_file(state, f, &text, line_ending)?;
                    }
                }
            }
            Command::Translate { src, dst } => {
                state.pattern_space = translate(&state.pattern_space, src, dst);
            }
            Command::Delete => return Ok(ExecResult::Delete),
            Command::DeleteFirstLine => {
                if let Some(p) = state.pattern_space.find('\n') {
                    state.pattern_space = state.pattern_space[p + 1..].to_string();
                    return Ok(ExecResult::Restart);
                } else { return Ok(ExecResult::Delete); }
            }
            Command::Print => { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
            Command::PrintFirstLine => {
                write!(w, "{}{}", state.pattern_space.lines().next().unwrap_or(""), *line_ending)?;
            }
            Command::PrintUnambiguous(width) => {
                let wrap = width.or(opts.line_length).unwrap_or(70);
                write!(w, "{}{}", format_unambiguous(&state.pattern_space, wrap), *line_ending)?;
            }
            Command::PrintLineNum => { write!(w, "{}{}", state.line_num, *line_ending)?; }
            Command::QuitWithCode(c) => return Ok(ExecResult::Quit(*c)),
            Command::QuitSilentWithCode(c) => return Ok(ExecResult::QuitSilent(*c)),
            Command::Next => {
                if !opts.quiet { write!(w, "{}{}", state.pattern_space, *line_ending)?; }
                flush_append_queue(state, w, line_ending)?;
                if let Some((_, l)) = iter.next() {
                    state.line_num += 1;
                    state.pattern_space = l.text.clone();
                    state.current_file = l.file.clone();
                    is_last = iter.peek().is_none();
                    *line_ending = l.line_ending;
                }
                else { return Ok(ExecResult::EndOfInput); }
            }
            Command::NextAppend => {
                if let Some((_, l)) = iter.next() {
                    state.line_num += 1;
                    state.pattern_space.push('\n');
                    state.pattern_space.push_str(&l.text);
                    state.current_file = l.file.clone();
                    is_last = iter.peek().is_none();
                    *line_ending = l.line_ending;
                } else { return Ok(ExecResult::Quit(0)); }
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
            Command::Change(t) => {
                // 範囲アドレスでは範囲の最終行で一度だけテキストを出力する（POSIX）
                if range_end {
                    write!(w, "{}{}", t, *line_ending)?;
                }
                return Ok(ExecResult::Delete);
            }
            Command::ReadFile(f) => {
                if let Ok(c) = fs::read_to_string(resolve_path_case_insensitive(f)) {
                    state.append_queue.push(c.trim_end_matches(['\r', '\n']).to_string());
                }
            }
            Command::WriteFile(f) => {
                let text = state.pattern_space.clone();
                write_to_file(state, f, &text, line_ending)?;
            }
            Command::ReadLineFile(f) => {
                // R: 呼び出しごとにファイルから1行読み込んで追加（尽きたら何もしない）
                if !state.read_line_files.contains_key(f) {
                    let lines: Vec<String> = fs::read_to_string(resolve_path_case_insensitive(f))
                        .map(|c| c.lines().map(str::to_string).collect())
                        .unwrap_or_default();
                    state.read_line_files.insert(f.clone(), (lines, 0));
                }
                if let Some((lines, idx)) = state.read_line_files.get_mut(f) {
                    if *idx < lines.len() {
                        state.append_queue.push(lines[*idx].clone());
                        *idx += 1;
                    }
                }
            }
            Command::WriteFirstLine(f) => {
                let text = state.pattern_space.lines().next().unwrap_or("").to_string();
                write_to_file(state, f, &text, line_ending)?;
            }
            Command::PrintFileName => {
                write!(w, "{}{}", state.current_file, *line_ending)?;
            }
            Command::Execute(cmdline) => {
                if cmdline.is_empty() {
                    // パターンスペースをコマンドとして実行し、出力で置き換える
                    let output = shell_exec(&state.pattern_space);
                    state.pattern_space = output.trim_end_matches(['\r', '\n']).to_string();
                } else {
                    // コマンドを実行して出力を先に流す
                    let output = shell_exec(cmdline);
                    w.write_all(output.as_bytes())?;
                    if !output.is_empty() && !output.ends_with('\n') {
                        write!(w, "{}", *line_ending)?;
                    }
                }
            }
            Command::Block(_) | Command::BlockBegin(_) => unreachable!("フラット化済み"),
        }
        ci += 1;
    }
    Ok(ExecResult::Continue)
}

/// e コマンド用のシェル実行
fn shell_exec(cmdline: &str) -> String {
    use std::process::Command as ProcCommand;

    #[cfg(windows)]
    let output = ProcCommand::new("cmd").arg("/C").arg(cmdline).output();
    #[cfg(not(windows))]
    let output = ProcCommand::new("sh").arg("-c").arg(cmdline).output();

    match output {
        Ok(o) => decode_to_utf8(&o.stdout),
        Err(_) => String::new(),
    }
}

/// アドレスの一致判定。
/// 戻り値は (一致したか, 範囲の最終行か)。範囲の最終行フラグは `c` コマンドが
/// テキストを一度だけ出力するために使う（単一アドレス・無アドレスでは常に true）。
fn addr_match(cmd: &SedCommand, state: &mut SedState, is_last: bool, opts: &Options) -> (bool, bool) {
    let ln = state.line_num;
    let (m, range_end) = match (&cmd.addr1, &cmd.addr2) {
        (None, None) => (true, true),
        (None, Some(a)) | (Some(a), None) => (
            single_match(a, ln, is_last, &state.pattern_space, opts),
            true,
        ),
        (Some(start), Some(end)) => {
            if let Some(active) = state.active_ranges.get(&cmd.id).cloned() {
                let closes = range_end_reached(end, &active, ln, is_last, &state.pattern_space, opts);
                if closes {
                    state.active_ranges.remove(&cmd.id);
                }
                (true, closes || is_last)
            } else {
                let start_matches = match start {
                    // 0,/re/: 1行目の手前から範囲が始まる（一度だけ）
                    Address::Zero => !state.zero_done.contains(&cmd.id),
                    a => single_match(a, ln, is_last, &state.pattern_space, opts),
                };
                if start_matches {
                    if matches!(start, Address::Zero) {
                        state.zero_done.insert(cmd.id);
                        // 終端の正規表現を「この行から」判定する（通常範囲との違い）
                        let closes = single_match(end, ln, is_last, &state.pattern_space, opts);
                        if !closes {
                            state.active_ranges.insert(cmd.id, ActiveRange { end_line: None });
                        }
                        (true, closes || is_last)
                    } else {
                        let end_line = range_end_line_hint(end, ln);
                        let closes = match (end, end_line) {
                            (_, Some(el)) => ln >= el,
                            (Address::LastLine, _) => is_last,
                            _ => false, // 正規表現終端は次の行から判定
                        };
                        if !closes {
                            state.active_ranges.insert(cmd.id, ActiveRange { end_line });
                        }
                        (true, closes || is_last)
                    }
                } else {
                    (false, false)
                }
            }
        }
    };
    if cmd.negate { (!m, true) } else { (m, range_end) }
}

/// 範囲終端の具体的な行番号（活性化時に確定できるもの）
fn range_end_line_hint(end: &Address, start_line: usize) -> Option<usize> {
    match end {
        Address::Line(n) => Some(*n),
        Address::RelOffset(n) => Some(start_line + n),
        Address::Multiple(n) => Some(((start_line + n - 1) / n) * n),
        _ => None,
    }
}

/// 活性化中の範囲がこの行で終わるか
fn range_end_reached(end: &Address, active: &ActiveRange, ln: usize, is_last: bool, line: &str, opts: &Options) -> bool {
    if let Some(el) = active.end_line {
        return ln >= el;
    }
    match end {
        Address::LastLine => is_last,
        Address::Regex { .. } | Address::Step(_, _) => single_match(end, ln, is_last, line, opts),
        _ => true,
    }
}

fn single_match(a: &Address, ln: usize, is_last: bool, line: &str, opts: &Options) -> bool {
    match a {
        Address::Line(n) => ln == *n,
        Address::LastLine => is_last,
        Address::Regex { pattern, ignore_case, multiline } => {
            build_regex(pattern, *ignore_case, *multiline, opts)
                .map(|r| r.is_match(line).unwrap_or(false))
                .unwrap_or(false)
        }
        Address::Step(f, s) => ln >= *f && *s > 0 && (ln - f) % s == 0,
        Address::Zero => false,
        Address::RelOffset(_) | Address::Multiple(_) => false,
    }
}

fn build_regex(pat: &str, ignore_case: bool, multiline: bool, opts: &Options) -> Result<Regex, FancyRegexError> {
    let base_pattern = if opts.extended_regex {
        ere_fixup(pat)
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

/// ERE モードの前処理: GNU 拡張の \< \> を \b に変換する（ブラケット式の中は除く）
fn ere_fixup(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut in_bracket = false;

    while let Some(c) = chars.next() {
        if in_bracket {
            out.push(c);
            if c == ']' { in_bracket = false; }
            continue;
        }
        match c {
            '[' => { in_bracket = true; out.push(c); }
            '\\' => match chars.next() {
                Some('<') | Some('>') => out.push_str(r"\b"),
                Some(n) => { out.push('\\'); out.push(n); }
                None => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
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
                    // GNU拡張: \< \> は単語境界
                    '<' | '>' => out.push_str(r"\b"),
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

/// 置換文字列のトークン
#[derive(Debug, Clone, PartialEq)]
enum ReplTok {
    Lit(char),
    Group(usize), // \1〜\9、& は Group(0)
    UpperOne,     // \u: 次の1文字を大文字に
    LowerOne,     // \l: 次の1文字を小文字に
    UpperAll,     // \U: \E まで大文字に
    LowerAll,     // \L: \E まで小文字に
    CaseEnd,      // \E
}

fn parse_replacement(s: &str) -> Vec<ReplTok> {
    let mut toks = Vec::new();
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        if c == '\\' {
            match cs.next() {
                Some(n @ '0'..='9') => toks.push(ReplTok::Group(n.to_digit(10).unwrap() as usize)),
                Some('&') => toks.push(ReplTok::Lit('&')),
                Some('\\') => toks.push(ReplTok::Lit('\\')),
                Some('n') => toks.push(ReplTok::Lit('\n')),
                Some('t') => toks.push(ReplTok::Lit('\t')),
                Some('r') => toks.push(ReplTok::Lit('\r')),
                Some('u') => toks.push(ReplTok::UpperOne),
                Some('l') => toks.push(ReplTok::LowerOne),
                Some('U') => toks.push(ReplTok::UpperAll),
                Some('L') => toks.push(ReplTok::LowerAll),
                Some('E') => toks.push(ReplTok::CaseEnd),
                Some(other) => toks.push(ReplTok::Lit(other)),
                None => toks.push(ReplTok::Lit('\\')),
            }
        } else if c == '&' {
            toks.push(ReplTok::Group(0));
        } else {
            toks.push(ReplTok::Lit(c));
        }
    }
    toks
}

/// 大小変換の状態を保ちながらトークン列を展開する
fn expand_replacement(toks: &[ReplTok], caps: &fancy_regex::Captures) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum CaseMode { None, Upper, Lower }

    let mut out = String::new();
    let mut mode = CaseMode::None;
    let mut one_shot: Option<bool> = None; // Some(true)=大文字化, Some(false)=小文字化

    let push_str = |out: &mut String, s: &str, mode: &CaseMode, one_shot: &mut Option<bool>| {
        for ch in s.chars() {
            let converted: String = if let Some(upper) = one_shot.take() {
                if upper { ch.to_uppercase().collect() } else { ch.to_lowercase().collect() }
            } else {
                match mode {
                    CaseMode::Upper => ch.to_uppercase().collect(),
                    CaseMode::Lower => ch.to_lowercase().collect(),
                    CaseMode::None => ch.to_string(),
                }
            };
            out.push_str(&converted);
        }
    };

    for tok in toks {
        match tok {
            ReplTok::Lit(c) => {
                let s = c.to_string();
                push_str(&mut out, &s, &mode, &mut one_shot);
            }
            ReplTok::Group(n) => {
                let text = caps.get(*n).map(|m| m.as_str()).unwrap_or("");
                push_str(&mut out, text, &mode, &mut one_shot);
            }
            ReplTok::UpperOne => one_shot = Some(true),
            ReplTok::LowerOne => one_shot = Some(false),
            ReplTok::UpperAll => { mode = CaseMode::Upper; one_shot = None; }
            ReplTok::LowerAll => { mode = CaseMode::Lower; one_shot = None; }
            ReplTok::CaseEnd => { mode = CaseMode::None; one_shot = None; }
        }
    }
    out
}

fn apply_subst(line: &str, pat: &str, repl: &str, flags: &SubstituteFlags, opts: &Options) -> (String, bool) {
    let re = match build_regex(pat, flags.ignore_case, flags.multiline, opts) {
        Ok(r) => r,
        Err(_) => return (line.to_string(), false),
    };

    let toks = parse_replacement(repl);

    // N 番目から置換を開始する。g との併用（GNU: N 番目以降すべて置換）にも対応
    let start_n = flags.nth.unwrap_or(1);
    let mut result = String::new();
    let mut last = 0usize;
    let mut count = 0usize;
    let mut replaced = false;

    for caps in re.captures_iter(line).flatten() {
        let m = caps.get(0).unwrap();
        count += 1;
        let should = if flags.global { count >= start_n } else { count == start_n };
        if should {
            result.push_str(&line[last..m.start()]);
            result.push_str(&expand_replacement(&toks, &caps));
            last = m.end();
            replaced = true;
            if !flags.global { break; }
        }
    }

    if !replaced {
        return (line.to_string(), false);
    }
    result.push_str(&line[last..]);
    (result, true)
}

fn translate(s: &str, src: &[char], dst: &[char]) -> String {
    s.chars()
        .map(|c| src.iter().position(|&x| x == c).and_then(|p| dst.get(p).copied()).unwrap_or(c))
        .collect()
}

/// l コマンドの出力を組み立てる。`wrap` が 2 以上なら
/// 各行が wrap 文字以内になるよう `\` + 改行で折り返す（GNU 互換、デフォルト 70）。
/// wrap = 0 は折り返しなし。エスケープシーケンスの途中では折り返さない。
fn format_unambiguous(s: &str, wrap: usize) -> String {
    // 文字ごとにエスケープ済みトークンを作る
    let tokens: Vec<String> = s.chars().map(|c| esc_unambig(&c.to_string())).collect();

    if wrap < 2 {
        let mut out: String = tokens.concat();
        out.push('$');
        return out;
    }

    let mut out = String::new();
    let mut col = 0usize;
    for tok in &tokens {
        // 折り返し行は末尾の '\' を含めて wrap 文字以内
        if col + tok.chars().count() > wrap - 1 {
            out.push('\\');
            out.push('\n');
            col = 0;
        }
        out.push_str(tok);
        col += tok.chars().count();
    }
    out.push('$');
    out
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

/// w コマンド / s///w の書き込み。
/// GNU 互換: 各出力ファイルは起動後最初の書き込みで truncate し、以後は追記する。
/// /dev/stdout・/dev/stderr は特別扱いする。
fn write_to_file(state: &mut SedState, f: &str, line: &str, line_ending: &str) -> io::Result<()> {
    if !state.write_files.contains_key(f) {
        let target = match f {
            "/dev/stdout" => WriteTarget::Stdout,
            "/dev/stderr" => WriteTarget::Stderr,
            _ => {
                let path = resolve_path_case_insensitive(f);
                WriteTarget::File(OpenOptions::new().create(true).write(true).truncate(true).open(path)?)
            }
        };
        state.write_files.insert(f.to_string(), target);
    }
    match state.write_files.get_mut(f).unwrap() {
        WriteTarget::Stdout => write!(io::stdout(), "{}{}", line, line_ending),
        WriteTarget::Stderr => write!(io::stderr(), "{}{}", line, line_ending),
        WriteTarget::File(file) => write!(file, "{}{}", line, line_ending),
    }
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

    fn compile(script: &str) -> (Vec<super::SedCommand>, std::collections::HashMap<String, usize>) {
        let mut commands = parse_script(script).unwrap();
        assign_command_ids(&mut commands);
        let commands = super::flatten_commands(commands);
        let labels = build_label_index(&commands);
        (commands, labels)
    }

    fn run_script(script: &str, input: &[&str], opts: Options) -> String {
        let (commands, labels) = compile(script);
        let mut out = Vec::new();
        process_lines(input, &commands, &labels, &opts, &mut out, "\n").unwrap();
        String::from_utf8(out).unwrap()
    }

    fn run_script_split(script: &str, chunks: &[Vec<&str>], opts: Options) -> String {
        let (commands, labels) = compile(script);
        let mut out = Vec::new();
        let mut state = SedState::new();

        let file = std::rc::Rc::new("-".to_string());
        for chunk in chunks {
            let input_lines: Vec<InputLine> = chunk
                .iter()
                .map(|line| InputLine { text: (*line).to_string(), line_ending: "\n", file: file.clone() })
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
    fn in_place_backup_suffix_must_be_attached_gnu_style() {
        // GNU 互換: -i の直後の独立した引数はサフィックスではなくスクリプト
        // （`sed -i 's/a/b/' file` が正しく動くことが最重要）
        let args = vec![
            "sed".to_string(),
            "-i".to_string(),
            "s/x/y/".to_string(),
            "file.txt".to_string(),
        ];
        let (opts, files) = parse_args(&args).unwrap();
        assert!(opts.in_place);
        assert_eq!(opts.in_place_backup, None);
        assert_eq!(opts.expressions, vec!["s/x/y/"]);
        assert_eq!(files, vec!["file.txt"]);

        // 結合形式 -i.bak はサフィックス
        let args = vec![
            "sed".to_string(),
            "-i.bak".to_string(),
            "s/x/y/".to_string(),
            "file.txt".to_string(),
        ];
        let (opts, files) = parse_args(&args).unwrap();
        assert!(opts.in_place);
        assert_eq!(opts.in_place_backup.as_deref(), Some(".bak"));
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
    fn zero_regex_range_can_end_on_first_line() {
        // 0,/re/ は 1 行目から終端判定する（1,/re/ との違い）
        let quiet = Options { quiet: true, ..Options::default() };
        let output = run_script("0,/foo/p", &["foo", "bar", "foo"], quiet);
        assert_eq!(output, "foo\n");

        let quiet = Options { quiet: true, ..Options::default() };
        let output = run_script("1,/foo/p", &["foo", "bar", "foo"], quiet);
        assert_eq!(output, "foo\nbar\nfoo\n");
    }

    #[test]
    fn relative_offset_range_matches_n_following_lines() {
        let output = run_script("2,+2d", &["a", "b", "c", "d", "e"], Options::default());
        assert_eq!(output, "a\ne\n");
    }

    #[test]
    fn multiple_range_ends_at_next_multiple() {
        // 3,~4 → 3行目から次の4の倍数行（4行目）まで
        let output = run_script("3,~4d", &["a", "b", "c", "d", "e"], Options::default());
        assert_eq!(output, "a\nb\ne\n");
    }

    #[test]
    fn address_regex_i_flag_ignores_case() {
        let quiet = Options { quiet: true, ..Options::default() };
        let output = run_script("/FOO/Ip", &["foo", "bar"], quiet);
        assert_eq!(output, "foo\n");
    }

    #[test]
    fn change_command_prints_text_once_for_a_range() {
        let output = run_script("2,4c\\X", &["a", "b", "c", "d", "e"], Options::default());
        assert_eq!(output, "a\nX\ne\n");
    }

    #[test]
    fn change_command_prints_text_per_line_for_single_address() {
        let output = run_script("/x/c\\Y", &["x", "a", "x"], Options::default());
        assert_eq!(output, "Y\na\nY\n");
    }

    #[test]
    fn substitution_case_conversion_escapes() {
        let output = run_script(r"s/\(.*\)/\U\1/", &["abc"], Options::default());
        assert_eq!(output, "ABC\n");
        let output = run_script(r"s/\(.*\)/\u\1/", &["abc"], Options::default());
        assert_eq!(output, "Abc\n");
        let output = run_script(r"s/\(a\)\(bc\)/\U\1\E\2/", &["abc"], Options::default());
        assert_eq!(output, "Abc\n");
        let output = run_script(r"s/\(.*\)/\L\1/", &["ABC"], Options::default());
        assert_eq!(output, "abc\n");
    }

    #[test]
    fn substitution_nth_with_global_replaces_from_nth_onward() {
        let output = run_script("s/o/0/2g", &["oooo"], Options::default());
        assert_eq!(output, "o000\n");
    }

    #[test]
    fn substitution_counts_as_made_even_when_output_is_identical() {
        // s/a/a/ でも置換は「行われた」— t が分岐する
        let quiet = Options { quiet: true, ..Options::default() };
        let output = run_script("s/a/a/; t yes; b; :yes; p", &["a"], quiet);
        assert_eq!(output, "a\n");
    }

    #[test]
    fn word_boundary_escapes_are_supported() {
        let output = run_script(r"s/\<cat\>/dog/", &["cat catalog"], Options::default());
        assert_eq!(output, "dog catalog\n");
    }

    #[test]
    fn branch_out_of_a_block_works() {
        // ブロック内からブロック外のラベルへ分岐できる（フラット化の検証）
        let quiet = Options { quiet: true, ..Options::default() };
        let output = run_script("/a/{b skip}; p; :skip", &["a", "b"], quiet);
        assert_eq!(output, "b\n");
    }

    #[test]
    fn delete_first_line_restarts_whole_cycle_from_inside_a_block() {
        // ブロック内の D もサイクル全体を再開する
        let output = run_script("N; /b/{D}", &["a", "b"], Options::default());
        assert_eq!(output, "b\n");
    }

    #[test]
    fn undefined_branch_label_is_rejected_before_execution() {
        let mut commands = parse_script("b nosuch").unwrap();
        assign_command_ids(&mut commands);
        let commands = super::flatten_commands(commands);
        let labels = build_label_index(&commands);
        assert!(super::validate_commands(&commands, &labels, &Options::default()).is_err());
    }

    #[test]
    fn invalid_regex_is_rejected_before_execution() {
        let mut commands = parse_script(r"s/a\(/X/").unwrap();
        assign_command_ids(&mut commands);
        let commands = super::flatten_commands(commands);
        let labels = build_label_index(&commands);
        assert!(super::validate_commands(&commands, &labels, &Options::default()).is_err());
    }

    #[test]
    fn sandbox_rejects_file_and_exec_commands() {
        let opts = Options { sandbox: true, ..Options::default() };
        let mut commands = parse_script("w out.txt").unwrap();
        assign_command_ids(&mut commands);
        let commands = super::flatten_commands(commands);
        let labels = build_label_index(&commands);
        assert!(super::validate_commands(&commands, &labels, &opts).is_err());
    }

    #[test]
    fn y_command_treats_hyphen_literally() {
        // GNU 互換: y は範囲展開しない
        let output = run_script("y/a-c/x-z/", &["a-c"], Options::default());
        assert_eq!(output, "x-z\n");
    }

    #[test]
    fn append_text_supports_multiline_continuation() {
        let output = run_script("a\\line1\\\nline2", &["x"], Options::default());
        assert_eq!(output, "x\nline1\nline2\n");
    }

    #[test]
    fn l_command_wraps_long_lines_with_backslash() {
        let quiet = Options { quiet: true, ..Options::default() };
        let output = run_script("l 10", &["aaaaaaaaaaaaaaaaaaaa"], quiet);
        // 各行は末尾の '\' を含めて 10 文字以内
        for line in output.lines() {
            assert!(line.chars().count() <= 10, "line too long: {line}");
        }
        assert!(output.ends_with("$\n"));
        // 中身を復元すると元の文字数
        let joined: String = output.lines().collect::<String>()
            .replace('\\', "").replace('$', "");
        assert_eq!(joined.chars().count(), 20);
    }

    #[test]
    fn quit_code_is_propagated_not_exited() {
        let (commands, labels) = compile("q5");
        let mut out = Vec::new();
        let code = process_lines(&["a", "b"], &commands, &labels, &Options::default(), &mut out, "\n").unwrap();
        assert_eq!(code, Some(5));
        assert_eq!(String::from_utf8(out).unwrap(), "a\n");
    }

    #[test]
    fn quit_silent_code_suppresses_output() {
        let (commands, labels) = compile("Q7");
        let mut out = Vec::new();
        let code = process_lines(&["a"], &commands, &labels, &Options::default(), &mut out, "\n").unwrap();
        assert_eq!(code, Some(7));
        assert!(out.is_empty());
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
