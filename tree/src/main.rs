use std::env;
use std::fs::{self, Metadata};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use glob::Pattern;

#[derive(Debug, Clone)]
struct Config {
    /// 対象ディレクトリ
    directories: Vec<String>,
    /// 全ファイル表示（隠しファイル含む）
    all_files: bool,
    /// ディレクトリのみ
    dirs_only: bool,
    /// 深さ制限
    max_depth: Option<usize>,
    /// フルパス表示
    full_path: bool,
    /// インデント文字列
    use_ascii: bool,
    /// 色付け
    colorize: bool,
    /// サイズ表示
    show_size: bool,
    /// 人間可読サイズ
    human_readable: bool,
    /// パーミッション表示
    show_permissions: bool,
    /// 更新日時表示
    show_date: bool,
    /// ファイル数のみ表示
    count_only: bool,
    /// パターンマッチ（含む）
    pattern: Option<Pattern>,
    /// パターンマッチ（除外）
    exclude_pattern: Option<Pattern>,
    /// ディレクトリ優先ソート
    dirs_first: bool,
    /// 逆順ソート
    reverse: bool,
    /// ソートなし
    no_sort: bool,
    /// サイズでソート
    sort_by_size: bool,
    /// 更新日時でソート
    sort_by_mtime: bool,
    /// シンボリックリンクをたどる
    follow_links: bool,
    /// 出力ファイル
    output_file: Option<String>,
    /// JSON出力
    json_output: bool,
    /// XML出力
    xml_output: bool,
    /// 末尾に分類子を付加
    classify: bool,
    /// クォート
    quote: bool,
    /// 行数制限
    file_limit: Option<usize>,
    /// 空ディレクトリを非表示
    prune_empty: bool,
    /// .gitignore対応
    gitignore: bool,
    /// 隠しファイル無視リスト
    ignore_patterns: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            directories: vec![".".to_string()],
            all_files: false,
            dirs_only: false,
            max_depth: None,
            full_path: false,
            use_ascii: false,
            colorize: true,
            show_size: false,
            human_readable: false,
            show_permissions: false,
            show_date: false,
            count_only: false,
            pattern: None,
            exclude_pattern: None,
            dirs_first: false,
            reverse: false,
            no_sort: false,
            sort_by_size: false,
            sort_by_mtime: false,
            follow_links: false,
            output_file: None,
            json_output: false,
            xml_output: false,
            classify: false,
            quote: false,
            file_limit: None,
            prune_empty: false,
            gitignore: false,
            ignore_patterns: vec![],
        }
    }
}

/// ツリー描画用の記号
struct TreeChars {
    branch: &'static str,
    last_branch: &'static str,
    vertical: &'static str,
    space: &'static str,
}

const TREE_CHARS_UNICODE: TreeChars = TreeChars {
    branch: "├── ",
    last_branch: "└── ",
    vertical: "│   ",
    space: "    ",
};

const TREE_CHARS_ASCII: TreeChars = TreeChars {
    branch: "|-- ",
    last_branch: "`-- ",
    vertical: "|   ",
    space: "    ",
};

/// 統計情報
#[derive(Default)]
struct Stats {
    directories: usize,
    files: usize,
}

#[derive(Clone, Debug)]
struct GitignoreRule {
    base_dir: PathBuf,
    basename_pattern: Option<Pattern>,
    path_pattern: Option<Vec<GitignorePathSegment>>,
    negated: bool,
    dir_only: bool,
    anchored: bool,
    has_slash: bool,
}

#[derive(Clone, Debug)]
enum GitignorePathSegment {
    DoubleStar,
    Pattern(Pattern),
}

/// ANSIカラーコード
struct Colors;

impl Colors {
    fn dir() -> &'static str { "\x1b[1;34m" }      // Bold Blue
    fn link() -> &'static str { "\x1b[1;36m" }     // Bold Cyan
    fn exec() -> &'static str { "\x1b[1;32m" }     // Bold Green
    fn archive() -> &'static str { "\x1b[1;31m" }  // Bold Red
    fn image() -> &'static str { "\x1b[1;35m" }    // Bold Magenta
    fn reset() -> &'static str { "\x1b[0m" }
}

fn print_help() {
    eprintln!(
        r#"使用法: tree [オプション]... [ディレクトリ]...
ディレクトリの内容をツリー形式で表示します。

リスト表示オプション:
  -a            隠しファイルも表示
  -d            ディレクトリのみ表示
  -l            シンボリックリンクをたどる
  -f            フルパスを表示
  -L LEVEL      深さをLEVELに制限
  -P PATTERN    PATTERNにマッチするファイルのみ表示
  -I PATTERN    PATTERNにマッチするファイルを除外
  --prune       空ディレクトリを表示しない
  --gitignore   .gitignore のパターンを適用
  --filelimit N N個以上のエントリがあるディレクトリを処理しない

ファイル情報オプション:
  -s            各ファイルのサイズを表示
  -h            サイズを人間可読形式で表示
  -p            パーミッションを表示
  -D            最終更新日時を表示
  -F            分類子を付加（/=ディレクトリ, *=実行可能）
  -Q            ファイル名をクォート

ソートオプション:
  -t            更新日時でソート
  -U            ソートしない
  -r            逆順でソート
  --dirsfirst   ディレクトリを先に表示
  -S            サイズでソート

出力オプション:
  -n            カラー出力を無効化
  -C            カラー出力を有効化（デフォルト）
  -A            ASCII文字でツリーを描画
  --charset X   文字セットを指定（ASCII/UTF-8）
  -o FILE       出力をFILEに書き込む
  -J            JSON形式で出力
  --noreport    末尾のレポートを表示しない

その他:
      --help    このヘルプを表示
      --version バージョン情報を表示

例:
  tree                      カレントディレクトリを表示
  tree -L 2                 深さ2まで表示
  tree -d                   ディレクトリのみ
  tree -a                   隠しファイルも含む
  tree -I "node_modules"    node_modulesを除外
  tree --gitignore          .gitignore を反映
  tree -P "*.rs"            .rsファイルのみ
  tree -hs                  サイズを人間可読形式で表示
  tree -J > tree.json       JSON出力

globパターン対応:
  tree src*                 src*にマッチするディレクトリを表示
"#
    );
}

fn print_version() {
    eprintln!("tree (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        let paths: Vec<String> = glob::glob(pattern)
            .map_err(|e| format!("globパターンエラー: {}", e))?
            .filter_map(Result::ok)
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        if paths.is_empty() {
            // ディレクトリが見つからない場合はそのまま返す
            Ok(vec![pattern.to_string()])
        } else {
            Ok(paths)
        }
    } else {
        Ok(vec![pattern.to_string()])
    }
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut config = Config::default();
    let mut directories: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-a" {
            config.all_files = true;
        } else if arg == "-d" {
            config.dirs_only = true;
        } else if arg == "-l" {
            config.follow_links = true;
        } else if arg == "-f" {
            config.full_path = true;
        } else if arg == "-L" {
            i += 1;
            if i >= args.len() {
                return Err("-L には深さが必要です".to_string());
            }
            config.max_depth = Some(args[i].parse()
                .map_err(|_| format!("無効な深さ: '{}'", args[i]))?);
        } else if arg.starts_with("-L") {
            config.max_depth = Some(arg[2..].parse()
                .map_err(|_| format!("無効な深さ: '{}'", &arg[2..]))?);
        } else if arg == "-P" {
            i += 1;
            if i >= args.len() {
                return Err("-P にはパターンが必要です".to_string());
            }
            config.pattern = Some(Pattern::new(&args[i])
                .map_err(|e| format!("無効なパターン: {}", e))?);
        } else if let Some(value) = arg.strip_prefix("-P") {
            if value.is_empty() {
                return Err("-P にはパターンが必要です".to_string());
            }
            config.pattern = Some(Pattern::new(value)
                .map_err(|e| format!("無効なパターン: {}", e))?);
        } else if arg == "-I" {
            i += 1;
            if i >= args.len() {
                return Err("-I にはパターンが必要です".to_string());
            }
            config.exclude_pattern = Some(Pattern::new(&args[i])
                .map_err(|e| format!("無効なパターン: {}", e))?);
            config.ignore_patterns.push(args[i].clone());
        } else if let Some(value) = arg.strip_prefix("-I") {
            if value.is_empty() {
                return Err("-I にはパターンが必要です".to_string());
            }
            config.exclude_pattern = Some(Pattern::new(value)
                .map_err(|e| format!("無効なパターン: {}", e))?);
            config.ignore_patterns.push(value.to_string());
        } else if arg == "-s" {
            config.show_size = true;
        } else if arg == "-h" {
            config.human_readable = true;
            config.show_size = true;
        } else if arg == "-p" {
            config.show_permissions = true;
        } else if arg == "-D" {
            config.show_date = true;
        } else if arg == "-F" {
            config.classify = true;
        } else if arg == "-Q" {
            config.quote = true;
        } else if arg == "-t" {
            config.sort_by_mtime = true;
        } else if arg == "-S" {
            config.sort_by_size = true;
        } else if arg == "-U" {
            config.no_sort = true;
        } else if arg == "-r" {
            config.reverse = true;
        } else if arg == "--dirsfirst" {
            config.dirs_first = true;
        } else if arg == "-n" {
            config.colorize = false;
        } else if arg == "-C" {
            config.colorize = true;
        } else if arg == "-A" || arg == "--charset=ASCII" {
            config.use_ascii = true;
        } else if arg == "--charset=UTF-8" || arg == "--charset=UTF8" {
            config.use_ascii = false;
        } else if arg == "--charset" {
            i += 1;
            if i >= args.len() {
                return Err("--charset には文字セット名が必要です".to_string());
            }
            match args[i].to_ascii_uppercase().as_str() {
                "ASCII" => config.use_ascii = true,
                "UTF-8" | "UTF8" => config.use_ascii = false,
                other => return Err(format!("未対応の文字セット: '{}'", other)),
            }
        } else if arg == "-o" {
            i += 1;
            if i >= args.len() {
                return Err("-o には出力ファイルが必要です".to_string());
            }
            config.output_file = Some(args[i].clone());
        } else if let Some(value) = arg.strip_prefix("-o") {
            if value.is_empty() {
                return Err("-o には出力ファイルが必要です".to_string());
            }
            config.output_file = Some(value.to_string());
        } else if arg == "-J" {
            config.json_output = true;
        } else if arg == "--prune" {
            config.prune_empty = true;
        } else if arg == "--gitignore" {
            config.gitignore = true;
        } else if arg == "--filelimit" {
            i += 1;
            if i >= args.len() {
                return Err("--filelimit には数値が必要です".to_string());
            }
            config.file_limit = Some(args[i].parse()
                .map_err(|_| format!("無効な数値: '{}'", args[i]))?);
        } else if let Some(value) = arg.strip_prefix("--filelimit=") {
            config.file_limit = Some(value.parse()
                .map_err(|_| format!("無効な数値: '{}'", value))?);
        } else if arg == "--noreport" {
            config.count_only = true;
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                let expanded = expand_glob(&args[j])?;
                directories.extend(expanded);
            }
            break;
        } else if arg.starts_with('-') {
            // 複合オプション
            for c in arg[1..].chars() {
                match c {
                    'a' => config.all_files = true,
                    'd' => config.dirs_only = true,
                    'l' => config.follow_links = true,
                    'f' => config.full_path = true,
                    's' => config.show_size = true,
                    'h' => { config.human_readable = true; config.show_size = true; }
                    'p' => config.show_permissions = true,
                    'D' => config.show_date = true,
                    'F' => config.classify = true,
                    'Q' => config.quote = true,
                    't' => config.sort_by_mtime = true,
                    'S' => config.sort_by_size = true,
                    'U' => config.no_sort = true,
                    'r' => config.reverse = true,
                    'n' => config.colorize = false,
                    'C' => config.colorize = true,
                    'A' => config.use_ascii = true,
                    'J' => config.json_output = true,
                    _ => return Err(format!("未対応のオプション: '-{}'", c)),
                }
            }
        } else {
            let expanded = expand_glob(arg)?;
            directories.extend(expanded);
        }

        i += 1;
    }

    if !directories.is_empty() {
        config.directories = directories;
    }

    // TTY検出
    if !atty_is_tty() {
        config.colorize = false;
    }

    Ok(config)
}

/// TTYかどうかを簡易判定
fn atty_is_tty() -> bool {
    io::stdout().is_terminal()
}

/// サイズを人間可読形式に
fn human_size(size: u64) -> String {
    const UNITS: &[&str] = &["", "K", "M", "G", "T", "P"];
    let mut size = size as f64;
    let mut unit_idx = 0;
    
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    
    if unit_idx == 0 {
        format!("{:4}", size as u64)
    } else {
        format!("{:4.1}{}", size, UNITS[unit_idx])
    }
}

/// パーミッションを文字列に
#[cfg(unix)]
fn format_permissions(mode: u32) -> String {
    let mut s = String::with_capacity(10);
    
    // ファイルタイプ
    s.push(if mode & 0o40000 != 0 { 'd' } else if mode & 0o120000 == 0o120000 { 'l' } else { '-' });
    
    // オーナー
    s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o100 != 0 { 'x' } else { '-' });
    
    // グループ
    s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o010 != 0 { 'x' } else { '-' });
    
    // その他
    s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o001 != 0 { 'x' } else { '-' });
    
    s
}

/// パーミッションを文字列に（Windows版）
#[cfg(not(unix))]
fn format_permissions(metadata: &Metadata) -> String {
    let mut s = String::with_capacity(10);
    
    // ファイルタイプ
    if metadata.is_dir() {
        s.push('d');
    } else {
        s.push('-');
    }
    
    // Windowsでは読み取り専用かどうかのみ
    let readonly = metadata.permissions().readonly();
    s.push('r');
    s.push(if readonly { '-' } else { 'w' });
    s.push('-');
    s.push('r');
    s.push(if readonly { '-' } else { 'w' });
    s.push('-');
    s.push('r');
    s.push(if readonly { '-' } else { 'w' });
    s.push('-');
    
    s
}

/// 日時をフォーマット
fn format_datetime(time: SystemTime) -> String {
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    
    // 簡易フォーマット
    let days = secs / 86400;
    let years = 1970 + days / 365;
    let remaining_days = days % 365;
    let months = remaining_days / 30 + 1;
    let day = remaining_days % 30 + 1;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    
    format!("{:04}-{:02}-{:02} {:02}:{:02}", years, months, day, hours, mins)
}

/// ファイルの色を取得
fn get_color(path: &Path, metadata: &Metadata, config: &Config) -> &'static str {
    if !config.colorize {
        return "";
    }
    
    if metadata.is_dir() {
        Colors::dir()
    } else if metadata.file_type().is_symlink() {
        Colors::link()
    } else {
        // 拡張子で判定
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext.to_lowercase().as_str() {
                "tar" | "gz" | "zip" | "rar" | "7z" | "bz2" | "xz" => Colors::archive(),
                "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "webp" => Colors::image(),
                "exe" | "bat" | "cmd" | "com" | "ps1" => Colors::exec(),
                _ => {
                    // Unix: 実行可能かチェック
                    #[cfg(unix)]
                    if metadata.permissions().mode() & 0o111 != 0 {
                        return Colors::exec();
                    }
                    ""
                }
            }
        } else {
            #[cfg(unix)]
            if metadata.permissions().mode() & 0o111 != 0 {
                return Colors::exec();
            }
            ""
        }
    }
}

/// 分類子を取得
#[cfg(unix)]
fn get_classifier(_path: &Path, metadata: &Metadata) -> &'static str {
    if metadata.is_dir() {
        "/"
    } else if metadata.file_type().is_symlink() {
        "@"
    } else if metadata.permissions().mode() & 0o111 != 0 {
        "*"
    } else {
        ""
    }
}

/// 分類子を取得（Windows版）
#[cfg(not(unix))]
fn get_classifier(path: &Path, metadata: &Metadata) -> &'static str {
    if metadata.is_dir() {
        "/"
    } else if metadata.file_type().is_symlink() {
        "@"
    } else {
        // Windowsでは拡張子で実行可能か判定
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext.to_lowercase().as_str() {
                "exe" | "bat" | "cmd" | "com" | "ps1" => "*",
                _ => ""
            }
        } else {
            ""
        }
    }
}

/// エントリをソート
fn sort_entries(entries: &mut Vec<fs::DirEntry>, config: &Config) {
    if config.no_sort {
        return;
    }
    
    entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        
        // ディレクトリ優先
        if config.dirs_first {
            if a_is_dir && !b_is_dir {
                return std::cmp::Ordering::Less;
            }
            if !a_is_dir && b_is_dir {
                return std::cmp::Ordering::Greater;
            }
        }
        
        // サイズソート
        if config.sort_by_size {
            let a_size = a.metadata().map(|m| m.len()).unwrap_or(0);
            let b_size = b.metadata().map(|m| m.len()).unwrap_or(0);
            let cmp = b_size.cmp(&a_size); // 大きい順
            if cmp != std::cmp::Ordering::Equal {
                return if config.reverse { cmp.reverse() } else { cmp };
            }
        }
        
        // 更新日時ソート
        if config.sort_by_mtime {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            if let (Some(a_t), Some(b_t)) = (a_time, b_time) {
                let cmp = b_t.cmp(&a_t); // 新しい順
                if cmp != std::cmp::Ordering::Equal {
                    return if config.reverse { cmp.reverse() } else { cmp };
                }
            }
        }
        
        // 名前ソート
        let a_name = a.file_name().to_string_lossy().to_lowercase();
        let b_name = b.file_name().to_string_lossy().to_lowercase();
        let cmp = a_name.cmp(&b_name);
        if config.reverse { cmp.reverse() } else { cmp }
    });
}

/// ファイル名がパターンにマッチするか
fn matches_pattern(name: &str, config: &Config) -> bool {
    // 除外パターン
    if let Some(ref pat) = config.exclude_pattern {
        if pat.matches(name) {
            return false;
        }
    }
    
    for ignore in &config.ignore_patterns {
        if let Ok(pat) = Pattern::new(ignore) {
            if pat.matches(name) {
                return false;
            }
        }
    }
    
    // 含むパターン
    if let Some(ref pat) = config.pattern {
        return pat.matches(name);
    }
    
    true
}

fn normalize_for_match(path: &Path) -> String {
    path.components()
        .filter_map(|component| {
            let part = component.as_os_str().to_string_lossy();
            if part.is_empty() || part == "." {
                None
            } else {
                Some(part.replace('\\', "/"))
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn unescape_gitignore_token(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                result.push(next);
            } else {
                result.push('\\');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

fn trim_trailing_unescaped_spaces(input: &str) -> String {
    let mut chars: Vec<char> = input.chars().collect();

    while matches!(chars.last(), Some(' ')) {
        let mut backslashes = 0usize;
        let mut idx = chars.len().saturating_sub(1);
        while idx > 0 && chars[idx - 1] == '\\' {
            backslashes += 1;
            idx -= 1;
        }

        if backslashes % 2 == 1 {
            break;
        }

        chars.pop();
    }

    chars.into_iter().collect()
}

fn compile_gitignore_path_pattern(pattern: &str) -> Option<Vec<GitignorePathSegment>> {
    let mut segments = Vec::new();

    for part in pattern.split('/') {
        if part == "**" {
            segments.push(GitignorePathSegment::DoubleStar);
        } else {
            segments.push(GitignorePathSegment::Pattern(Pattern::new(part).ok()?));
        }
    }

    Some(segments)
}

fn parse_gitignore_line(line: &str, base_dir: &Path) -> Option<GitignoreRule> {
    if line.is_empty() {
        return None;
    }

    let (negated, line) = if let Some(rest) = line.strip_prefix("\\!") {
        (false, format!("!{}", rest))
    } else if let Some(rest) = line.strip_prefix('!') {
        (true, rest.to_string())
    } else if let Some(rest) = line.strip_prefix("\\#") {
        (false, format!("#{}", rest))
    } else {
        if line.starts_with('#') {
            return None;
        }
        (false, line.to_string())
    };

    let raw_pattern = trim_trailing_unescaped_spaces(&line);
    if raw_pattern.is_empty() {
        return None;
    }

    let dir_only = raw_pattern.ends_with('/');
    let raw_pattern = raw_pattern.trim_end_matches('/');
    let anchored = raw_pattern.starts_with('/');
    let raw_pattern = raw_pattern.trim_start_matches("./");
    let raw_pattern = raw_pattern.trim_start_matches('/');
    if raw_pattern.is_empty() {
        return None;
    }

    let normalized = unescape_gitignore_token(raw_pattern).replace('\\', "/");
    let has_slash = normalized.contains('/');

    let basename_pattern = if has_slash {
        None
    } else {
        Pattern::new(&normalized).ok()
    };
    let path_pattern = if has_slash {
        Some(compile_gitignore_path_pattern(&normalized)?)
    } else {
        None
    };

    Some(GitignoreRule {
        base_dir: base_dir.to_path_buf(),
        basename_pattern,
        path_pattern,
        negated,
        dir_only,
        anchored,
        has_slash,
    })
}

fn load_gitignore_rules(dir: &Path) -> Vec<GitignoreRule> {
    let gitignore_path = dir.join(".gitignore");
    let contents = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };

    contents
        .lines()
        .filter_map(|line| parse_gitignore_line(line, dir))
        .collect()
}

fn matches_gitignore_segments(pattern: &[GitignorePathSegment], path: &[&str]) -> bool {
    fn inner(pattern: &[GitignorePathSegment], path: &[&str]) -> bool {
        if pattern.is_empty() {
            return path.is_empty();
        }

        match &pattern[0] {
            GitignorePathSegment::DoubleStar => {
                if inner(&pattern[1..], path) {
                    return true;
                }
                for idx in 0..path.len() {
                    if inner(&pattern[1..], &path[(idx + 1)..]) {
                        return true;
                    }
                }
                false
            }
            GitignorePathSegment::Pattern(segment_pattern) => {
                if let Some((head, tail)) = path.split_first() {
                    segment_pattern.matches(head) && inner(&pattern[1..], tail)
                } else {
                    false
                }
            }
        }
    }

    inner(pattern, path)
}

fn should_ignore_gitignore(path: &Path, is_dir: bool, rules: &[GitignoreRule]) -> bool {
    let mut ignored = false;

    for rule in rules {
        let rel_path = match path.strip_prefix(&rule.base_dir) {
            Ok(rel_path) => rel_path,
            Err(_) => continue,
        };

        let rel = normalize_for_match(rel_path);
        if rel.is_empty() {
            continue;
        }

        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let rel_segments: Vec<&str> = if rel.is_empty() {
            Vec::new()
        } else {
            rel.split('/').collect()
        };

        let basename_matches = rule
            .basename_pattern
            .as_ref()
            .map(|pattern| pattern.matches(&name))
            .unwrap_or(false);
        let path_matches = rule
            .path_pattern
            .as_ref()
            .map(|pattern| matches_gitignore_segments(pattern, &rel_segments))
            .unwrap_or(false);
        let matches = if rule.has_slash {
            path_matches
        } else if rule.anchored {
            rel_segments.len() == 1 && basename_matches
        } else {
            basename_matches
        };

        if matches && (!rule.dir_only || is_dir) {
            ignored = !rule.negated;
        }
    }

    ignored
}

fn is_hidden(name: &str, metadata: &Metadata) -> bool {
    if name == "." || name == ".." {
        return false;
    }

    if name.starts_with('.') {
        return true;
    }

    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        return metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0;
    }

    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

/// ツリーを表示
fn print_tree(
    path: &Path,
    prefix: &str,
    config: &Config,
    stats: &mut Stats,
    chars: &TreeChars,
    output: &mut dyn Write,
    depth: usize,
    parent_gitignore_rules: &[GitignoreRule],
) -> io::Result<bool> {
    // 深さ制限
    if let Some(max) = config.max_depth {
        if depth > max {
            return Ok(false);
        }
    }
    
    let mut gitignore_rules = parent_gitignore_rules.to_vec();
    if config.gitignore {
        gitignore_rules.extend(load_gitignore_rules(path));
    }

    // ディレクトリを読み込み
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            writeln!(output, "{} [エラー: {}]", path.display(), e)?;
            return Ok(false);
        }
    };
    
    let mut entries: Vec<fs::DirEntry> = Vec::new();
    for entry_result in rd {
        match entry_result {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                writeln!(output, "{}[エラー: '{}' 内のエントリを読み取れません: {}]", prefix, path.display(), e)?;
            }
        }
    }
    
    // ファイル数制限
    if let Some(limit) = config.file_limit {
        if entries.len() > limit {
            return Ok(false);
        }
    }
    
    // フィルタリング
    let mut filtered: Vec<fs::DirEntry> = entries
        .into_iter()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let metadata = match e.metadata() {
                Ok(metadata) => metadata,
                Err(_) => return false,
            };
            
            // 隠しファイル
            if !config.all_files && is_hidden(&name, &metadata) {
                return false;
            }

            if config.gitignore && should_ignore_gitignore(&e.path(), metadata.is_dir(), &gitignore_rules) {
                return false;
            }
            
            // ディレクトリのみモード
            if config.dirs_only {
                if !metadata.is_dir() {
                    return false;
                }
            }
            
            // パターンマッチ
            if !matches_pattern(&name, config) {
                // ディレクトリは常に表示（中身を見るため）
                if metadata.is_dir() {
                    return true;
                }
                return false;
            }
            
            true
        })
        .collect();
    
    // ソート
    sort_entries(&mut filtered, config);
    
    let count = filtered.len();
    let mut has_content = false;
    
    for (idx, entry) in filtered.iter().enumerate() {
        let is_last = idx == count - 1;
        let name = entry.file_name().to_string_lossy().to_string();
        let entry_path = entry.path();
        
        let metadata = if config.follow_links {
            fs::metadata(&entry_path)
        } else {
            entry.metadata()
        };
        
        let metadata = match metadata {
            Ok(m) => m,
            Err(_) => continue,
        };
        
        let is_dir = metadata.is_dir();
        
        // 表示文字列を構築
        let mut info = String::new();
        
        // パーミッション
        if config.show_permissions {
            #[cfg(unix)]
            {
                info.push_str(&format!("[{}]  ", format_permissions(metadata.permissions().mode())));
            }
            #[cfg(not(unix))]
            {
                info.push_str(&format!("[{}]  ", format_permissions(&metadata)));
            }
        }
        
        // サイズ
        if config.show_size {
            let size = metadata.len();
            if config.human_readable {
                info.push_str(&format!("[{:>6}]  ", human_size(size)));
            } else {
                info.push_str(&format!("[{:>10}]  ", size));
            }
        }
        
        // 日時
        if config.show_date {
            if let Ok(mtime) = metadata.modified() {
                info.push_str(&format!("[{}]  ", format_datetime(mtime)));
            }
        }
        
        // 分岐文字
        let branch = if is_last { chars.last_branch } else { chars.branch };
        
        // ファイル名
        let display_name = if config.full_path {
            entry_path.display().to_string()
        } else {
            name.clone()
        };
        
        let display_name = if config.quote {
            format!("\"{}\"", display_name)
        } else {
            display_name
        };
        
        // 色と分類子
        let color = get_color(&entry_path, &metadata, config);
        let classifier = if config.classify { get_classifier(&entry_path, &metadata) } else { "" };
        let reset = if config.colorize && !color.is_empty() { Colors::reset() } else { "" };
        
        // 出力
        writeln!(output, "{}{}{}{}{}{}{}", prefix, branch, info, color, display_name, classifier, reset)?;
        has_content = true;
        
        // 統計
        if is_dir {
            stats.directories += 1;
            
            // 再帰
            let new_prefix = format!("{}{}", prefix, if is_last { chars.space } else { chars.vertical });
            print_tree(&entry_path, &new_prefix, config, stats, chars, output, depth + 1, &gitignore_rules)?;
        } else {
            stats.files += 1;
        }
    }
    
    Ok(has_content)
}

/// JSON出力
fn print_tree_json(
    path: &Path,
    config: &Config,
    depth: usize,
    parent_gitignore_rules: &[GitignoreRule],
) -> io::Result<serde_json::Value> {
    use std::collections::BTreeMap;
    
    // 簡易JSON構築
    let mut obj = BTreeMap::new();
    obj.insert("name".to_string(), serde_json::Value::String(
        path.file_name().unwrap_or_default().to_string_lossy().to_string()
    ));
    obj.insert("type".to_string(), serde_json::Value::String(
        if path.is_dir() { "directory".to_string() } else { "file".to_string() }
    ));
    
    if path.is_dir() {
        let mut gitignore_rules = parent_gitignore_rules.to_vec();
        if config.gitignore {
            gitignore_rules.extend(load_gitignore_rules(path));
        }

        if let Some(max) = config.max_depth {
            if depth >= max {
                return Ok(serde_json::Value::Object(obj.into_iter().collect()));
            }
        }
        
        let mut children = Vec::new();
        if let Ok(entries) = fs::read_dir(path) {
            for entry_result in entries {
                let entry = match entry_result {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("tree: '{}' 内のエントリを読み取れません: {}", path.display(), e);
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().to_string();
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if !config.all_files && is_hidden(&name, &metadata) {
                    continue;
                }
                if config.gitignore && should_ignore_gitignore(&entry.path(), metadata.is_dir(), &gitignore_rules) {
                    continue;
                }
                children.push(print_tree_json(&entry.path(), config, depth + 1, &gitignore_rules)?);
            }
        }
        obj.insert("contents".to_string(), serde_json::Value::Array(children));
    }
    
    Ok(serde_json::Value::Object(obj.into_iter().collect()))
}

/// 簡易JSONシリアライザ
mod serde_json {
    use std::collections::BTreeMap;
    
    #[derive(Clone)]
    pub enum Value {
        String(String),
        Array(Vec<Value>),
        Object(BTreeMap<String, Value>),
    }
    
    impl Value {
        pub fn to_string_pretty(&self) -> String {
            format_value(self, 0)
        }
    }
    
    fn format_value(value: &Value, indent: usize) -> String {
        let ind = "  ".repeat(indent);
        match value {
            Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Array(arr) => {
                if arr.is_empty() {
                    "[]".to_string()
                } else {
                    let items: Vec<String> = arr.iter()
                        .map(|v| format!("{}{}", "  ".repeat(indent + 1), format_value(v, indent + 1)))
                        .collect();
                    format!("[\n{}\n{}]", items.join(",\n"), ind)
                }
            }
            Value::Object(obj) => {
                if obj.is_empty() {
                    "{}".to_string()
                } else {
                    let items: Vec<String> = obj.iter()
                        .map(|(k, v)| format!("{}\"{}\": {}", "  ".repeat(indent + 1), k, format_value(v, indent + 1)))
                        .collect();
                    format!("{{\n{}\n{}}}", items.join(",\n"), ind)
                }
            }
        }
    }
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tree: {}", e);
            eprintln!("詳しくは 'tree --help' を参照してください");
            process::exit(1);
        }
    };

    if config.xml_output {
        eprintln!("tree: XML出力は未実装です");
        process::exit(1);
    }

    let chars = if config.use_ascii { &TREE_CHARS_ASCII } else { &TREE_CHARS_UNICODE };
    
    // 出力先
    let mut output: Box<dyn Write> = if let Some(ref file) = config.output_file {
        match std::fs::File::create(file) {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!("tree: '{}': {}", file, e);
                process::exit(1);
            }
        }
    } else {
        Box::new(io::stdout())
    };
    
    let mut total_stats = Stats::default();
    
    // JSON出力
    if config.json_output {
        let mut results = Vec::new();
        for dir in &config.directories {
            let path = PathBuf::from(dir);
            if !path.exists() {
                eprintln!("tree: '{}': そのようなファイルやディレクトリはありません", dir);
                continue;
            }
            if let Ok(json) = print_tree_json(&path, &config, 0, &[]) {
                results.push(json);
            }
        }
        if results.len() == 1 {
            writeln!(output, "{}", results[0].to_string_pretty()).ok();
        } else {
            writeln!(output, "{}", serde_json::Value::Array(results).to_string_pretty()).ok();
        }
        return;
    }
    
    // 通常出力
    for (idx, dir) in config.directories.iter().enumerate() {
        let path = PathBuf::from(dir);
        
        if !path.exists() {
            eprintln!("tree: '{}': そのようなファイルやディレクトリはありません", dir);
            continue;
        }
        
        if idx > 0 {
            writeln!(output).ok();
        }
        
        // ルートディレクトリ名
        let color = if config.colorize { Colors::dir() } else { "" };
        let reset = if config.colorize { Colors::reset() } else { "" };
        writeln!(output, "{}{}{}", color, dir, reset).ok();
        
        let mut stats = Stats::default();
        print_tree(&path, "", &config, &mut stats, chars, &mut output, 1, &[]).ok();
        
        total_stats.directories += stats.directories;
        total_stats.files += stats.files;
    }
    
    // レポート
    if !config.count_only {
        writeln!(output).ok();
        if config.dirs_only {
            writeln!(output, "{} ディレクトリ", total_stats.directories).ok();
        } else {
            writeln!(output, "{} ディレクトリ, {} ファイル", 
                     total_stats.directories, total_stats.files).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_temp_dir(name: &str) -> PathBuf {
        let unique = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("tree-tests-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn render_tree(root: &Path, gitignore: bool) -> String {
        let config = Config {
            directories: vec![root.display().to_string()],
            colorize: false,
            gitignore,
            ..Config::default()
        };
        let mut output = Vec::new();
        let mut stats = Stats::default();
        print_tree(
            root,
            "",
            &config,
            &mut stats,
            &TREE_CHARS_ASCII,
            &mut output,
            1,
            &[],
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn gitignore_ignores_directory_and_log_files() {
        let dir = make_temp_dir("gitignore-basic");
        fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("error.log"), "log").unwrap();
        fs::write(dir.join("keep.txt"), "ok").unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("target"), true, &rules));
        assert!(should_ignore_gitignore(&dir.join("error.log"), false, &rules));
        assert!(!should_ignore_gitignore(&dir.join("keep.txt"), false, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_honors_negation() {
        let dir = make_temp_dir("gitignore-negation");
        fs::write(dir.join(".gitignore"), "*.log\n!important.log\n").unwrap();
        fs::write(dir.join("debug.log"), "log").unwrap();
        fs::write(dir.join("important.log"), "keep").unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("debug.log"), false, &rules));
        assert!(!should_ignore_gitignore(&dir.join("important.log"), false, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn child_gitignore_rules_apply_under_subdirectory() {
        let dir = make_temp_dir("gitignore-nested");
        let child = dir.join("src");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join(".gitignore"), "*.tmp\n").unwrap();
        fs::write(child.join("cache.tmp"), "tmp").unwrap();
        fs::write(child.join("main.rs"), "fn main() {}\n").unwrap();

        let mut rules = Vec::new();
        rules.extend(load_gitignore_rules(&dir));
        rules.extend(load_gitignore_rules(&child));

        assert!(should_ignore_gitignore(&child.join("cache.tmp"), false, &rules));
        assert!(!should_ignore_gitignore(&child.join("main.rs"), false, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_anchored_pattern_only_matches_from_rule_root() {
        let dir = make_temp_dir("gitignore-anchored");
        fs::write(dir.join(".gitignore"), "/build/\n").unwrap();
        fs::create_dir_all(dir.join("build")).unwrap();
        fs::create_dir_all(dir.join("src").join("build")).unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("build"), true, &rules));
        assert!(!should_ignore_gitignore(&dir.join("src").join("build"), true, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_supports_escaped_comment_and_negation_prefixes() {
        let dir = make_temp_dir("gitignore-escaped");
        fs::write(dir.join(".gitignore"), "\\#notes.txt\n\\!important.txt\n").unwrap();
        fs::write(dir.join("#notes.txt"), "note").unwrap();
        fs::write(dir.join("!important.txt"), "note").unwrap();
        fs::write(dir.join("normal.txt"), "note").unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("#notes.txt"), false, &rules));
        assert!(should_ignore_gitignore(&dir.join("!important.txt"), false, &rules));
        assert!(!should_ignore_gitignore(&dir.join("normal.txt"), false, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_negation_can_restore_nested_file() {
        let dir = make_temp_dir("gitignore-negation-nested");
        fs::write(dir.join(".gitignore"), "*.log\n!important.log\n").unwrap();
        let child = dir.join("src");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join("debug.log"), "log").unwrap();
        fs::write(child.join("important.log"), "keep").unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&child.join("debug.log"), false, &rules));
        assert!(!should_ignore_gitignore(&child.join("important.log"), false, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_trims_unescaped_trailing_spaces() {
        let dir = make_temp_dir("gitignore-trailing-space");
        fs::write(dir.join(".gitignore"), "foo \nbar\\ \n").unwrap();
        fs::write(dir.join("foo"), "x").unwrap();
        fs::write(dir.join("bar "), "y").unwrap();
        fs::write(dir.join("bar"), "z").unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("foo"), false, &rules));
        assert!(should_ignore_gitignore(&dir.join("bar "), false, &rules));
        assert!(!should_ignore_gitignore(&dir.join("bar"), false, &rules));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_does_not_restore_child_when_parent_dir_remains_ignored() {
        let dir = make_temp_dir("gitignore-parent-stays-ignored");
        let build = dir.join("build");
        fs::create_dir_all(&build).unwrap();
        fs::write(dir.join(".gitignore"), "build/\n!build/keep.txt\n").unwrap();
        fs::write(build.join("keep.txt"), "keep").unwrap();

        let rendered = render_tree(&dir, true);

        assert!(!rendered.contains("build"));
        assert!(!rendered.contains("keep.txt"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_restores_child_after_parent_dir_is_unignored() {
        let dir = make_temp_dir("gitignore-parent-unignored");
        let build = dir.join("build");
        fs::create_dir_all(&build).unwrap();
        fs::write(
            dir.join(".gitignore"),
            "build/\n!build/\n!build/keep.txt\n",
        )
        .unwrap();
        fs::write(build.join("keep.txt"), "keep").unwrap();

        let rendered = render_tree(&dir, true);

        assert!(rendered.contains("build"));
        assert!(rendered.contains("keep.txt"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_slash_pattern_is_relative_to_gitignore_dir() {
        let dir = make_temp_dir("gitignore-relative-slash");
        fs::create_dir_all(dir.join("foo")).unwrap();
        fs::create_dir_all(dir.join("nested").join("foo")).unwrap();
        fs::write(dir.join(".gitignore"), "foo/bar.txt\n").unwrap();
        fs::write(dir.join("foo").join("bar.txt"), "x").unwrap();
        fs::write(dir.join("nested").join("foo").join("bar.txt"), "y").unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("foo").join("bar.txt"), false, &rules));
        assert!(!should_ignore_gitignore(
            &dir.join("nested").join("foo").join("bar.txt"),
            false,
            &rules
        ));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gitignore_double_star_can_match_across_directories() {
        let dir = make_temp_dir("gitignore-double-star");
        fs::create_dir_all(dir.join("a").join("b").join("c")).unwrap();
        fs::write(dir.join(".gitignore"), "a/**/target.txt\n**/cache\nlogs/**\n").unwrap();
        fs::write(dir.join("a").join("target.txt"), "x").unwrap();
        fs::write(dir.join("a").join("b").join("c").join("target.txt"), "y").unwrap();
        fs::create_dir_all(dir.join("x").join("cache")).unwrap();
        fs::create_dir_all(dir.join("logs").join("deep").join("more")).unwrap();

        let rules = load_gitignore_rules(&dir);

        assert!(should_ignore_gitignore(&dir.join("a").join("target.txt"), false, &rules));
        assert!(should_ignore_gitignore(
            &dir.join("a").join("b").join("c").join("target.txt"),
            false,
            &rules
        ));
        assert!(should_ignore_gitignore(&dir.join("x").join("cache"), true, &rules));
        assert!(should_ignore_gitignore(&dir.join("logs").join("deep"), true, &rules));
        assert!(should_ignore_gitignore(
            &dir.join("logs").join("deep").join("more"),
            true,
            &rules
        ));

        fs::remove_dir_all(&dir).unwrap();
    }
}
