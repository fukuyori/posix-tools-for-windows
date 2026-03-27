// ls - ディレクトリの内容を一覧表示
// POSIX準拠 + GNU拡張

use std::env;
use std::fs::{self, File, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

use chrono::{DateTime, Local};
use glob;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
};
#[cfg(windows)]
use windows::Win32::System::Console::{
    GetConsoleScreenBufferInfo, GetStdHandle, CONSOLE_SCREEN_BUFFER_INFO, STD_OUTPUT_HANDLE,
};

#[derive(Default, Clone, Copy, PartialEq)]
enum ColorMode {
    Never,
    #[default]
    Auto,
    Always,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum TimeType {
    #[default]
    Modification,  // mtime（デフォルト）
    Access,        // atime (-u)
    Change,        // ctime (-c) Windows では作成時刻
}

#[derive(Default, Clone, Copy, PartialEq)]
enum OutputFormat {
    #[default]
    Columns,       // -C デフォルト（TTY時）
    Long,          // -l
    OnePerLine,    // -1
    CommaDelim,    // -m
    Across,        // -x 横方向
}

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    all: bool,               // -a: . で始まるエントリも表示
    almost_all: bool,        // -A: . と .. 以外を表示
    directory: bool,         // -d: ディレクトリ自体を表示
    classify: bool,          // -F: タイプ識別子を付加
    follow_symlinks: bool,   // -L: シンボリックリンクをたどる（-lと共に）
    inode: bool,             // -i: inode番号を表示
    recursive: bool,         // -R: 再帰的に表示
    reverse: bool,           // -r: 逆順ソート
    size_sort: bool,         // -S: サイズ順ソート
    time_sort: bool,         // -t: 時間順ソート
    time_type: TimeType,     // -c/-u: 使用する時刻
    no_sort: bool,           // -f: ソートしない（-aを暗黙的に有効）
    hide_owner: bool,        // -g: オーナーを非表示（-lと共に）
    hide_group: bool,        // -o: グループを非表示（-lと共に）
    numeric_ids: bool,       // -n: UID/GIDを数値表示
    non_printable: bool,     // -q: 非表示文字を?に置換
    show_blocks: bool,       // -s: ブロック数を表示
    output_format: OutputFormat,
    
    // GNU拡張オプション
    human_readable: bool,    // -h: 人間が読みやすいサイズ
    si: bool,                // --si: 1000単位
    block_size: Option<u64>, // --block-size
    escape: bool,            // -b: エスケープ表示
    quote_name: bool,        // -Q: 名前をクォート
    group_dirs_first: bool,  // --group-directories-first
    color_mode: ColorMode,
    time_style: Option<String>,
    
    show_help: bool,
    show_version: bool,
}

struct FileInfo {
    name: String,
    path: PathBuf,
    metadata: Metadata,
    is_hidden: bool,
    is_system: bool,
    is_readonly: bool,
    link_target: Option<String>,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (opts, paths) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("ls: {}", e);
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("ls 1.0.0 (Rust Windows版 - POSIX準拠)");
        std::process::exit(0);
    }

    // glob展開
    let paths = expand_globs(paths);
    
    let paths = if paths.is_empty() {
        vec![".".to_string()]
    } else {
        paths
    };

    let mut exit_code = 0;
    let multiple = paths.len() > 1 || opts.recursive;

    // ファイルとディレクトリを分離
    let mut files: Vec<FileInfo> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();

    for path in &paths {
        let p = Path::new(path);
        match get_file_info(p, &opts) {
            Ok(info) => {
                if info.metadata.is_dir() && !opts.directory {
                    dirs.push(path.clone());
                } else {
                    files.push(info);
                }
            }
            Err(e) => {
                eprintln!("ls: '{}' にアクセスできません: {}", path, format_error(&e));
                exit_code = 1;
            }
        }
    }

    // 先にファイルを表示
    if !files.is_empty() {
        if !opts.no_sort {
            sort_entries(&mut files, &opts);
        }
        if opts.reverse {
            files.reverse();
        }
        if let Err(e) = display_files(&files, &opts) {
            eprintln!("ls: 書き込みエラー: {}", format_error(&e));
            exit_code = 1;
        }
        if !dirs.is_empty() {
            println!();
        }
    }

    // ディレクトリを表示
    for (i, dir) in dirs.iter().enumerate() {
        if i > 0 || !files.is_empty() {
            println!();
        }
        if multiple {
            println!("{}:", dir);
        }
        if let Err(e) = list_directory(dir, &opts) {
            eprintln!("ls: '{}' を読み込めません: {}", dir, format_error(&e));
            exit_code = 1;
        }
    }

    std::process::exit(exit_code);
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options::default();
    let mut paths = Vec::new();
    let mut i = 1;
    let mut end_of_opts = false;

    while i < args.len() {
        let arg = &args[i];

        if end_of_opts {
            paths.push(arg.clone());
            i += 1;
            continue;
        }

        if arg == "--" {
            end_of_opts = true;
            i += 1;
            continue;
        }

        if arg.starts_with("--") {
            parse_long_option(arg, &mut opts)?;
            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            parse_short_options(arg, &mut opts)?;
            i += 1;
            continue;
        }

        paths.push(arg.clone());
        i += 1;
    }

    // -f は -a を暗黙的に有効化
    if opts.no_sort {
        opts.all = true;
    }

    Ok((opts, paths))
}

fn parse_long_option(arg: &str, opts: &mut Options) -> Result<(), String> {
    let opt = &arg[2..];
    let (name, value) = if let Some(pos) = opt.find('=') {
        (&opt[..pos], Some(&opt[pos + 1..]))
    } else {
        (opt, None)
    };

    match name {
        "all" => opts.all = true,
        "almost-all" => opts.almost_all = true,
        "directory" => opts.directory = true,
        "classify" => opts.classify = true,
        "dereference" => opts.follow_symlinks = true,
        "inode" => opts.inode = true,
        "recursive" => opts.recursive = true,
        "reverse" => opts.reverse = true,
        "human-readable" => opts.human_readable = true,
        "si" => {
            opts.human_readable = true;
            opts.si = true;
        }
        "escape" => opts.escape = true,
        "quote-name" => opts.quote_name = true,
        "numeric-uid-gid" => opts.numeric_ids = true,
        "group-directories-first" => opts.group_dirs_first = true,
        "color" => {
            opts.color_mode = match value {
                Some("always") | Some("yes") | Some("force") => ColorMode::Always,
                Some("never") | Some("no") | Some("none") => ColorMode::Never,
                Some("auto") | Some("tty") | Some("if-tty") | None => ColorMode::Auto,
                Some(v) => return Err(format!("'--color' の引数が不正です: '{}'", v)),
            };
        }
        "block-size" => {
            let val = value.ok_or("'--block-size' には引数が必要です")?;
            opts.block_size = Some(parse_block_size(val)?);
        }
        "time-style" => {
            opts.time_style = Some(value.unwrap_or("locale").to_string());
        }
        "help" => opts.show_help = true,
        "version" => opts.show_version = true,
        _ => return Err(format!("不明なオプション: '--{}'", name)),
    }

    Ok(())
}

fn parse_short_options(arg: &str, opts: &mut Options) -> Result<(), String> {
    for c in arg[1..].chars() {
        match c {
            // POSIX標準
            'a' => opts.all = true,
            'A' => opts.almost_all = true,
            'C' => opts.output_format = OutputFormat::Columns,
            'c' => opts.time_type = TimeType::Change,
            'd' => opts.directory = true,
            'F' => opts.classify = true,
            'f' => opts.no_sort = true,
            'g' => {
                opts.output_format = OutputFormat::Long;
                opts.hide_owner = true;
            }
            'H' => opts.follow_symlinks = true,  // コマンドライン引数のみ
            'i' => opts.inode = true,
            'L' => opts.follow_symlinks = true,
            'l' => opts.output_format = OutputFormat::Long,
            'm' => opts.output_format = OutputFormat::CommaDelim,
            'n' => {
                opts.output_format = OutputFormat::Long;
                opts.numeric_ids = true;
            }
            'o' => {
                opts.output_format = OutputFormat::Long;
                opts.hide_group = true;
            }
            'q' => opts.non_printable = true,
            'R' => opts.recursive = true,
            'r' => opts.reverse = true,
            'S' => opts.size_sort = true,
            's' => opts.show_blocks = true,
            't' => opts.time_sort = true,
            'u' => opts.time_type = TimeType::Access,
            'U' => opts.no_sort = true,
            'x' => opts.output_format = OutputFormat::Across,
            '1' => opts.output_format = OutputFormat::OnePerLine,
            // GNU拡張
            'b' => opts.escape = true,
            'h' => opts.human_readable = true,
            'Q' => opts.quote_name = true,
            _ => return Err(format!("不正なオプション: '-{}'", c)),
        }
    }
    Ok(())
}

fn parse_block_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let s_upper = s.to_uppercase();
    
    let (num_str, multiplier) = if s_upper.ends_with("KB") {
        (&s[..s.len() - 2], 1000u64)
    } else if s_upper.ends_with("MB") {
        (&s[..s.len() - 2], 1000 * 1000)
    } else if s_upper.ends_with("GB") {
        (&s[..s.len() - 2], 1000 * 1000 * 1000)
    } else if s_upper.ends_with('K') {
        (&s[..s.len() - 1], 1024u64)
    } else if s_upper.ends_with('M') {
        (&s[..s.len() - 1], 1024 * 1024)
    } else if s_upper.ends_with('G') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024)
    } else {
        (s, 1u64)
    };

    if num_str.is_empty() {
        Ok(multiplier)
    } else {
        num_str.parse::<u64>()
            .map(|n| n * multiplier)
            .map_err(|_| format!("不正なブロックサイズ: '{}'", s))
    }
}

/// POSIX寄りのglob展開。
/// Windowsでも Linux のシェル展開に近いルールで評価する。
fn expand_globs(raw_paths: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();

    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
        ..Default::default()
    };

    for pattern in raw_paths {
        if has_glob_magic(&pattern) {
            match glob::glob_with(&pattern, options) {
                Ok(paths) => {
                    let mut matched = false;
                    for entry in paths {
                        if let Ok(path) = entry {
                            result.push(path.to_string_lossy().to_string());
                            matched = true;
                        }
                    }
                    if !matched {
                        result.push(pattern);
                    }
                }
                Err(_) => {
                    result.push(pattern);
                }
            }
        } else {
            result.push(pattern);
        }
    }

    result
}

fn has_glob_magic(pattern: &str) -> bool {
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '?' => return true,
            '[' => {
                if chars.peek().is_some() {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn print_help() {
    println!(r#"使い方: ls [オプション]... [ファイル]...

ファイルの情報を一覧表示します（デフォルトはカレントディレクトリ）。
エントリはアルファベット順にソートされます。

POSIX標準オプション:
  -a, --all             . で始まるエントリも表示
  -A, --almost-all      . と .. を除く全エントリを表示
  -c                    -lt と共に使用: ctime でソートして表示
                        -l と共に使用: mtime でソートして ctime を表示
  -C                    カラム形式で表示（デフォルト）
  -d, --directory       ディレクトリ自体の情報を表示
  -f                    ソートしない（-a を有効化）
  -F, --classify        エントリにタイプ識別子を付加 (*/=>@|)
  -g                    -l に似るがオーナーを表示しない
  -H                    コマンドライン引数のシンボリックリンクをたどる
  -i, --inode           各ファイルのinode番号を表示
  -L, --dereference     シンボリックリンクをたどる
  -l                    詳細形式で表示
  -m                    カンマ区切りで表示
  -n, --numeric-uid-gid -l に似るが UID/GID を数値で表示
  -o                    -l に似るがグループを表示しない
  -q                    非表示文字を ? で表示
  -R, --recursive       サブディレクトリを再帰的に表示
  -r, --reverse         ソート順を逆にする
  -S                    ファイルサイズ順にソート（大きい順）
  -s                    各ファイルのブロック数を表示
  -t                    更新時刻順にソート（新しい順）
  -u                    -lt と共に: atime でソートして表示
                        -l と共に: mtime でソートして atime を表示
  -x                    横方向にエントリを並べる
  -1                    1行に1ファイルを表示

GNU拡張オプション:
  -b, --escape          非表示文字を C 言語形式でエスケープ
  -h, --human-readable  -l と共に: サイズを K, M, G 単位で表示
      --si              -h と同様だが 1000 単位を使用
  -Q, --quote-name      エントリ名をダブルクォートで囲む
      --block-size=SIZE ブロックサイズを指定
      --color[=WHEN]    色付け表示 (auto/always/never)
      --group-directories-first
                        ディレクトリを先に表示
      --time-style=STYLE 時刻表示形式 (full-iso/long-iso/iso/locale)
      --help            このヘルプを表示
      --version         バージョン情報を表示

終了ステータス:
  0  正常終了
  1  軽微な問題（アクセス不可など）
  2  重大な問題（オプションエラーなど）

例:
  ls                    カレントディレクトリの一覧
  ls -la                隠しファイルを含む詳細表示
  ls -lh                人間が読みやすいサイズ表示
  ls -lt                更新時刻順で詳細表示
  ls -R                 再帰的に表示
  ls *.txt              txtファイルを一覧
  ls --color=always | less -R  色付きでページャに出力"#);
}

fn list_directory(path: &str, opts: &Options) -> io::Result<()> {
    let dir_path = Path::new(path);

    let mut entries = Vec::new();

    for entry_result in fs::read_dir(dir_path)? {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("ls: '{}' 内のエントリを読み取れません: {}", dir_path.display(), format_error(&e));
                continue;
            }
        };
        let file_path = entry.path();

        let info = match get_file_info(&file_path, opts) {
            Ok(info) => info,
            Err(e) => {
                eprintln!("ls: '{}': {}", file_path.display(), format_error(&e));
                continue;
            }
        };

        // フィルタリング
        if !opts.all && !opts.almost_all && info.is_hidden {
            continue;
        }

        entries.push(info);
    }

    if !opts.no_sort {
        sort_entries(&mut entries, opts);
    }

    if opts.reverse {
        entries.reverse();
    }

    display_files(&entries, opts)?;

    // 再帰表示
    if opts.recursive {
        for entry in &entries {
            if entry.metadata.is_dir() {
                let sub_path = entry.path.to_string_lossy().to_string();
                println!("\n{}:", sub_path);
                if let Err(e) = list_directory(&sub_path, opts) {
                    eprintln!("ls: '{}' を読み込めません: {}", sub_path, format_error(&e));
                }
            }
        }
    }

    Ok(())
}

fn get_file_info(path: &Path, opts: &Options) -> io::Result<FileInfo> {
    let metadata = if opts.follow_symlinks {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let (is_hidden, is_system, is_readonly) = get_file_attributes(path);

    let link_target = if metadata.file_type().is_symlink() {
        fs::read_link(path).ok().map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };

    Ok(FileInfo {
        name,
        path: path.to_path_buf(),
        metadata,
        is_hidden,
        is_system,
        is_readonly,
        link_target,
    })
}

#[cfg(windows)]
fn get_file_attributes(path: &Path) -> (bool, bool, bool) {
    let name = path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let is_dot_hidden = name.starts_with('.');

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide_path.as_ptr())) };

    if attrs == u32::MAX {
        return (is_dot_hidden, false, false);
    }

    let is_hidden = is_dot_hidden || (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
    let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
    let is_readonly = (attrs & FILE_ATTRIBUTE_READONLY.0) != 0;

    (is_hidden, is_system, is_readonly)
}

#[cfg(not(windows))]
fn get_file_attributes(path: &Path) -> (bool, bool, bool) {
    let name = path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    (name.starts_with('.'), false, false)
}

#[cfg(windows)]
fn get_file_id(path: &Path) -> u64 {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return 0,
    };

    unsafe {
        let handle = HANDLE(file.as_raw_handle() as *mut std::ffi::c_void);
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();

        if GetFileInformationByHandle(handle, &mut info).is_ok() {
            ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64)
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn get_file_id(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map(|m| m.ino()).unwrap_or(0)
}

fn sort_entries(entries: &mut Vec<FileInfo>, opts: &Options) {
    // --group-directories-first の場合は安定ソートを使用
    let sort_fn = |a: &FileInfo, b: &FileInfo| -> std::cmp::Ordering {
        // ディレクトリを先に
        if opts.group_dirs_first {
            match (a.metadata.is_dir(), b.metadata.is_dir()) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }
        }

        // ソート基準
        if opts.size_sort {
            b.metadata.len().cmp(&a.metadata.len())
        } else if opts.time_sort {
            let time_a = get_time(&a.metadata, opts.time_type);
            let time_b = get_time(&b.metadata, opts.time_type);
            time_b.cmp(&time_a)
        } else {
            a.name.cmp(&b.name)
        }
    };

    entries.sort_by(sort_fn);
}

#[cfg(test)]
mod tests {
    use super::{get_file_attributes, has_glob_magic};
    use std::path::Path;

    #[test]
    fn detects_glob_character_classes() {
        assert!(has_glob_magic("*.rs"));
        assert!(has_glob_magic("file?.txt"));
        assert!(has_glob_magic("src/[ab].rs"));
        assert!(!has_glob_magic("plain.txt"));
    }

    #[test]
    fn dotfile_is_treated_as_hidden() {
        let (is_hidden, _, _) = get_file_attributes(Path::new(".env"));
        assert!(is_hidden);
    }
}

fn get_time(meta: &Metadata, time_type: TimeType) -> SystemTime {
    match time_type {
        TimeType::Modification => meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        TimeType::Access => meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH),
        TimeType::Change => {
            // Windows では作成時刻を使用
            meta.created().unwrap_or_else(|_| {
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)
            })
        }
    }
}

fn display_files(files: &[FileInfo], opts: &Options) -> io::Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let use_color = should_use_color(opts);
    let is_tty = is_stdout_tty();

    match opts.output_format {
        OutputFormat::Long => display_long(files, opts, use_color, &mut stdout),
        OutputFormat::OnePerLine => display_one_per_line(files, opts, use_color, &mut stdout),
        OutputFormat::CommaDelim => display_comma(files, opts, use_color, &mut stdout),
        OutputFormat::Across => display_across(files, opts, use_color, &mut stdout),
        OutputFormat::Columns => {
            if is_tty {
                display_columns(files, opts, use_color, &mut stdout)
            } else {
                display_one_per_line(files, opts, use_color, &mut stdout)
            }
        }
    }
}

fn display_long<W: Write>(files: &[FileInfo], opts: &Options, use_color: bool, writer: &mut W) -> io::Result<()> {
    // 各カラムの幅を計算
    let mut max_size_width = 0;
    let mut max_nlink = 0u64;
    let mut total_blocks = 0u64;

    for file in files {
        let size_str = format_size(file.metadata.len(), opts);
        if size_str.len() > max_size_width {
            max_size_width = size_str.len();
        }
        // Windows では nlink は常に1として扱う
        max_nlink = max_nlink.max(1);
        total_blocks += (file.metadata.len() + 511) / 512;
    }

    // total 行を表示
    if opts.show_blocks || files.len() > 1 {
        writeln!(writer, "合計 {}", total_blocks)?;
    }

    let nlink_width = format!("{}", max_nlink).len();

    for file in files {
        let meta = &file.metadata;

        // inode
        if opts.inode {
            write!(writer, "{:>8} ", get_file_id(&file.path))?;
        }

        // ブロック数
        if opts.show_blocks {
            let blocks = (meta.len() + 511) / 512;
            write!(writer, "{:>4} ", blocks)?;
        }

        // ファイルタイプとパーミッション
        let file_type = if meta.is_dir() {
            'd'
        } else if meta.file_type().is_symlink() {
            'l'
        } else {
            '-'
        };

        let perms = format_permissions(file);
        write!(writer, "{}{} ", file_type, perms)?;

        // リンク数
        write!(writer, "{:>width$} ", 1, width = nlink_width)?;

        // オーナー
        if !opts.hide_owner {
            if opts.numeric_ids {
                write!(writer, "{:>5} ", 0)?;
            } else {
                write!(writer, "{:<8} ", get_owner_name())?;
            }
        }

        // グループ
        if !opts.hide_group {
            if opts.numeric_ids {
                write!(writer, "{:>5} ", 0)?;
            } else {
                write!(writer, "{:<8} ", get_group_name())?;
            }
        }

        // サイズ
        let size_str = format_size(meta.len(), opts);
        write!(writer, "{:>width$} ", size_str, width = max_size_width)?;

        // 時刻
        let time = get_time(meta, opts.time_type);
        let time_str = format_time(time, opts);
        write!(writer, "{} ", time_str)?;

        // ファイル名
        let name = format_name(file, opts, use_color);
        write!(writer, "{}", name)?;

        // シンボリックリンクのターゲット
        if let Some(ref target) = file.link_target {
            write!(writer, " -> {}", target)?;
        }

        writeln!(writer)?;
    }

    Ok(())
}

fn display_one_per_line<W: Write>(files: &[FileInfo], opts: &Options, use_color: bool, writer: &mut W) -> io::Result<()> {
    for file in files {
        if opts.inode {
            write!(writer, "{:>8} ", get_file_id(&file.path))?;
        }
        if opts.show_blocks {
            let blocks = (file.metadata.len() + 511) / 512;
            write!(writer, "{:>4} ", blocks)?;
        }
        let name = format_name(file, opts, use_color);
        writeln!(writer, "{}", name)?;
    }
    Ok(())
}

fn display_comma<W: Write>(files: &[FileInfo], opts: &Options, use_color: bool, writer: &mut W) -> io::Result<()> {
    let names: Vec<String> = files.iter()
        .map(|f| format_name(f, opts, use_color))
        .collect();
    writeln!(writer, "{}", names.join(", "))?;
    Ok(())
}

fn display_across<W: Write>(files: &[FileInfo], opts: &Options, use_color: bool, writer: &mut W) -> io::Result<()> {
    let term_width = get_terminal_width().unwrap_or(80);
    let max_width = files.iter()
        .map(|f| display_width(&f.name) + if opts.classify && f.metadata.is_dir() { 1 } else { 0 })
        .max()
        .unwrap_or(1);
    
    let col_width = max_width + 2;
    let num_cols = (term_width / col_width).max(1);

    for (i, file) in files.iter().enumerate() {
        if opts.inode {
            write!(writer, "{:>8} ", get_file_id(&file.path))?;
        }
        let name = format_name(file, opts, use_color);
        let dw = display_width(&file.name) + if opts.classify && file.metadata.is_dir() { 1 } else { 0 };
        let padding = col_width.saturating_sub(dw);

        write!(writer, "{}", name)?;
        
        if (i + 1) % num_cols == 0 || i == files.len() - 1 {
            writeln!(writer)?;
        } else {
            write!(writer, "{}", " ".repeat(padding))?;
        }
    }
    Ok(())
}

fn display_columns<W: Write>(files: &[FileInfo], opts: &Options, use_color: bool, writer: &mut W) -> io::Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let term_width = get_terminal_width().unwrap_or(80);

    let display_widths: Vec<usize> = files.iter().map(|f| {
        let name_width = display_width(&f.name);
        let suffix_width = if opts.classify && f.metadata.is_dir() { 1 } else { 0 };
        let inode_width = if opts.inode { 9 } else { 0 };
        let blocks_width = if opts.show_blocks { 5 } else { 0 };
        name_width + suffix_width + inode_width + blocks_width
    }).collect();

    let max_width = display_widths.iter().max().copied().unwrap_or(1);
    let col_width = max_width + 2;
    let num_cols = (term_width / col_width).max(1);
    let num_rows = (files.len() + num_cols - 1) / num_cols;

    for row in 0..num_rows {
        for col in 0..num_cols {
            let idx = col * num_rows + row;
            if idx >= files.len() {
                break;
            }

            let file = &files[idx];

            if opts.inode {
                write!(writer, "{:>8} ", get_file_id(&file.path))?;
            }
            if opts.show_blocks {
                let blocks = (file.metadata.len() + 511) / 512;
                write!(writer, "{:>4} ", blocks)?;
            }

            let name = format_name(file, opts, use_color);
            let display_len = display_widths[idx];
            let padding = if col < num_cols - 1 && idx + num_rows < files.len() {
                col_width.saturating_sub(display_len)
            } else {
                0
            };

            write!(writer, "{}{}", name, " ".repeat(padding))?;
        }
        writeln!(writer)?;
    }

    Ok(())
}

fn format_name(file: &FileInfo, opts: &Options, use_color: bool) -> String {
    let mut name = if opts.escape {
        escape_name(&file.name)
    } else if opts.non_printable {
        replace_non_printable(&file.name)
    } else {
        file.name.clone()
    };

    if opts.quote_name {
        name = format!("\"{}\"", name);
    }

    if opts.classify {
        if file.metadata.is_dir() {
            name.push('/');
        } else if file.metadata.file_type().is_symlink() {
            name.push('@');
        } else if is_executable_file(&file.name) {
            name.push('*');
        }
    }

    if use_color {
        name = colorize_name(&name, file);
    }

    name
}

fn escape_name(name: &str) -> String {
    let mut result = String::new();
    for c in name.chars() {
        match c {
            '\n' => result.push_str("\\n"),
            '\t' => result.push_str("\\t"),
            '\r' => result.push_str("\\r"),
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            c if c.is_control() => {
                result.push_str(&format!("\\{:03o}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

fn replace_non_printable(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

fn colorize_name(name: &str, file: &FileInfo) -> String {
    if file.metadata.is_dir() {
        format!("\x1b[34m{}\x1b[0m", name)
    } else if file.metadata.file_type().is_symlink() {
        format!("\x1b[36m{}\x1b[0m", name)
    } else if file.is_system {
        format!("\x1b[31m{}\x1b[0m", name)
    } else if file.is_hidden {
        format!("\x1b[90m{}\x1b[0m", name)
    } else if is_executable_file(&file.name) {
        format!("\x1b[32m{}\x1b[0m", name)
    } else {
        name.to_string()
    }
}

fn format_permissions(file: &FileInfo) -> String {
    // Windowsの簡易パーミッション表示
    let r = 'r';
    let w = if file.is_readonly { '-' } else { 'w' };
    let x = if file.metadata.is_dir() || is_executable_file(&file.name) { 'x' } else { '-' };
    format!("{}{}{}{}{}{}{}{}{}", r, w, x, r, '-', x, r, '-', x)
}

fn format_size(size: u64, opts: &Options) -> String {
    if let Some(block_size) = opts.block_size {
        return format!("{}", (size + block_size - 1) / block_size);
    }

    if opts.human_readable {
        let base = if opts.si { 1000u64 } else { 1024u64 };
        let units = if opts.si {
            ["B", "KB", "MB", "GB", "TB", "PB"]
        } else {
            ["", "K", "M", "G", "T", "P"]
        };

        let mut value = size as f64;
        let mut unit_idx = 0;

        while value >= base as f64 && unit_idx < units.len() - 1 {
            value /= base as f64;
            unit_idx += 1;
        }

        if unit_idx == 0 {
            format!("{}", size)
        } else if value >= 100.0 {
            format!("{:.0}{}", value, units[unit_idx])
        } else if value >= 10.0 {
            format!("{:.1}{}", value, units[unit_idx])
        } else {
            format!("{:.2}{}", value, units[unit_idx])
        }
    } else {
        format!("{}", size)
    }
}

fn format_time(time: SystemTime, opts: &Options) -> String {
    let datetime: DateTime<Local> = time.into();
    
    let style = opts.time_style.as_deref().unwrap_or("locale");
    match style {
        "full-iso" => datetime.format("%Y-%m-%d %H:%M:%S.%9f %z").to_string(),
        "long-iso" => datetime.format("%Y-%m-%d %H:%M").to_string(),
        "iso" => datetime.format("%Y-%m-%d").to_string(),
        _ => {
            // locale / デフォルト: 6ヶ月以内なら月日時分、それ以外は月日年
            let now = Local::now();
            let six_months_ago = now - chrono::Duration::days(183);
            
            if datetime > six_months_ago {
                datetime.format("%b %e %H:%M").to_string()
            } else {
                datetime.format("%b %e  %Y").to_string()
            }
        }
    }
}

fn get_owner_name() -> String {
    // Windowsでは簡易的に現在のユーザー名を返す
    env::var("USERNAME").unwrap_or_else(|_| "user".to_string())
}

fn get_group_name() -> String {
    // Windowsではグループ概念が異なるため簡易表示
    "users".to_string()
}

fn is_executable_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com" | "ps1" | "msi")
}

#[cfg(windows)]
fn is_stdout_tty() -> bool {
    unsafe {
        let handle = match GetStdHandle(STD_OUTPUT_HANDLE) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        GetConsoleScreenBufferInfo(handle, &mut info).is_ok()
    }
}

#[cfg(not(windows))]
fn is_stdout_tty() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
}

fn should_use_color(opts: &Options) -> bool {
    match opts.color_mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => is_stdout_tty(),
    }
}

#[cfg(windows)]
fn get_terminal_width() -> Option<usize> {
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();

        if GetConsoleScreenBufferInfo(handle, &mut info).is_ok() {
            let width = (info.srWindow.Right - info.srWindow.Left + 1) as usize;
            Some(width)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn get_terminal_width() -> Option<usize> {
    // Unix では termios を使用
    Some(80)
}

fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum()
}

fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        io::ErrorKind::NotADirectory => "ディレクトリではありません".to_string(),
        _ => {
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "そのようなファイルやディレクトリはありません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    123 => "ファイル名、ディレクトリ名、またはボリュームラベルの構文が間違っています".to_string(),
                    _ => format!("エラー (コード: {})", code),
                };
            }
            e.to_string()
        }
    }
}
