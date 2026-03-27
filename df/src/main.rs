use std::env;
use std::io::{self, Write};

use glob::glob;

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

// Windows ドライブタイプ定数
#[cfg(windows)]
const DRIVE_REMOVABLE: u32 = 2;
#[cfg(windows)]
const DRIVE_FIXED: u32 = 3;
#[cfg(windows)]
const DRIVE_REMOTE: u32 = 4;
#[cfg(windows)]
const DRIVE_CDROM: u32 = 5;
#[cfg(windows)]
const DRIVE_RAMDISK: u32 = 6;

#[derive(Default)]
struct Options {
    // POSIX オプション
    posix_format: bool, // -P: POSIX出力形式（512バイトブロック）
    portability: bool,  // -k: 1024バイトブロック（POSIX）

    // GNU拡張オプション
    all: bool,                    // -a: すべてのファイルシステムを表示
    human_readable: bool,         // -h: 人間が読みやすい形式 (1024単位)
    si: bool,                     // -H: 人間が読みやすい形式 (1000単位)
    block_size: Option<u64>,      // -B: ブロックサイズ指定
    total: bool,                  // --total: 合計行を表示
    type_filter: Option<String>,  // -t: 指定タイプのみ表示
    exclude_type: Option<String>, // -x: 指定タイプを除外
    local: bool,                  // -l: ローカルのみ
    show_type: bool,              // -T: ファイルシステムタイプを表示
    inodes: bool,                 // -i: inode情報を表示

    show_help: bool,
    show_version: bool,
}

struct FilesystemInfo {
    filesystem: String,    // ファイルシステム名/デバイス
    mount_point: String,   // マウントポイント
    fs_type: String,       // ファイルシステムタイプ
    total_blocks: u64,     // 総ブロック数
    used_blocks: u64,      // 使用ブロック数
    available_blocks: u64, // 利用可能ブロック数
    total_inodes: u64,     // 総inode数
    used_inodes: u64,      // 使用inode数
    available_inodes: u64, // 利用可能inode数
    block_size: u64,       // ブロックサイズ（バイト）
    #[allow(dead_code)]
    is_remote: bool, // リモートファイルシステムか
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (opts, paths) = parse_args(&args);

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("df 1.0.0 (Rust実装)");
        std::process::exit(0);
    }

    let paths = expand_path_globs(&paths);

    let filesystems = if paths.is_empty() {
        get_all_filesystems(&opts)
    } else {
        get_filesystems_for_paths(&paths, &opts)
    };

    print_filesystems(&filesystems, &opts);
}

fn parse_args(args: &[String]) -> (Options, Vec<String>) {
    let mut opts = Options::default();
    let mut paths = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        match arg.as_str() {
            "--" => {
                // 残りはすべてパスとして扱う
                paths.extend(args[i + 1..].iter().cloned());
                break;
            }
            // POSIX オプション
            "-P" => opts.posix_format = true,
            "-k" => opts.portability = true,
            // GNU拡張オプション
            "-a" | "--all" => opts.all = true,
            "-h" | "--human-readable" => opts.human_readable = true,
            "-H" | "--si" => {
                opts.human_readable = true;
                opts.si = true;
            }
            "-i" | "--inodes" => opts.inodes = true,
            "-l" | "--local" => opts.local = true,
            "-T" | "--print-type" => opts.show_type = true,
            "--total" => opts.total = true,
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            "-B" | "--block-size" => {
                if let Some(val) = args.get(i + 1) {
                    opts.block_size = parse_block_size(val);
                    i += 1;
                } else {
                    eprintln!("df: -B のあとにサイズを指定してください");
                    std::process::exit(1);
                }
            }
            "-t" | "--type" => {
                if let Some(val) = args.get(i + 1) {
                    opts.type_filter = Some(val.clone());
                    i += 1;
                } else {
                    eprintln!("df: -t のあとにタイプを指定してください");
                    std::process::exit(1);
                }
            }
            "-x" | "--exclude-type" => {
                if let Some(val) = args.get(i + 1) {
                    opts.exclude_type = Some(val.clone());
                    i += 1;
                } else {
                    eprintln!("df: -x のあとにタイプを指定してください");
                    std::process::exit(1);
                }
            }
            s if s.starts_with("-B") && s.len() > 2 => {
                opts.block_size = parse_block_size(&s[2..]);
            }
            s if s.starts_with("--block-size=") => {
                opts.block_size = parse_block_size(s.trim_start_matches("--block-size="));
            }
            s if s.starts_with("--type=") => {
                opts.type_filter = Some(s.trim_start_matches("--type=").to_string());
            }
            s if s.starts_with("--exclude-type=") => {
                opts.exclude_type = Some(s.trim_start_matches("--exclude-type=").to_string());
            }
            s if s.starts_with("--") => {
                eprintln!("df: 不明なオプション '{}'", s);
                std::process::exit(1);
            }
            s if s.starts_with('-') && s.len() > 1 => {
                // 複合短縮オプション
                for c in s.chars().skip(1) {
                    match c {
                        'P' => opts.posix_format = true,
                        'k' => opts.portability = true,
                        'a' => opts.all = true,
                        'h' => opts.human_readable = true,
                        'H' => {
                            opts.human_readable = true;
                            opts.si = true;
                        }
                        'i' => opts.inodes = true,
                        'l' => opts.local = true,
                        'T' => opts.show_type = true,
                        _ => {
                            eprintln!("df: 不明なオプション '-{}'", c);
                            std::process::exit(1);
                        }
                    }
                }
            }
            s => {
                paths.push(s.to_string());
            }
        }

        i += 1;
    }

    (opts, paths)
}

fn is_glob_pattern(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

fn expand_path_globs(paths: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();

    for path in paths {
        if is_glob_pattern(path) {
            let pattern = if cfg!(windows) {
                path.replace('\\', "/")
            } else {
                path.clone()
            };

            let mut matched = false;
            for entry in glob(&pattern).unwrap_or_else(|err| {
                eprintln!("df: glob パターンの解析に失敗: {}", err);
                std::process::exit(1);
            }) {
                match entry {
                    Ok(pathbuf) => {
                        matched = true;
                        if let Some(s) = pathbuf.to_str() {
                            expanded.push(s.to_string());
                        }
                    }
                    Err(err) => {
                        eprintln!("df: glob 展開でエラー: {}", err);
                    }
                }
            }
            if !matched {
                expanded.push(path.clone());
            }
        } else {
            expanded.push(path.clone());
        }
    }

    expanded
}

fn parse_block_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let s_upper = s.to_uppercase();

    // サフィックスを解析
    let (num_str, multiplier) = if s_upper.ends_with("KB") {
        (&s[..s.len() - 2], 1000u64)
    } else if s_upper.ends_with("MB") {
        (&s[..s.len() - 2], 1000 * 1000)
    } else if s_upper.ends_with("GB") {
        (&s[..s.len() - 2], 1000 * 1000 * 1000)
    } else if s_upper.ends_with("TB") {
        (&s[..s.len() - 2], 1000 * 1000 * 1000 * 1000)
    } else if s_upper.ends_with('K') {
        (&s[..s.len() - 1], 1024u64)
    } else if s_upper.ends_with('M') {
        (&s[..s.len() - 1], 1024 * 1024)
    } else if s_upper.ends_with('G') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024)
    } else if s_upper.ends_with('T') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024 * 1024)
    } else {
        (s, 1u64)
    };

    if num_str.is_empty() {
        Some(multiplier)
    } else {
        num_str.parse::<u64>().ok().map(|n| n * multiplier)
    }
}

fn print_help() {
    println!(
        r#"使い方: df [オプション] [ファイル]...

ファイルシステムのディスク使用量を表示します。
引数がない場合、現在マウントされているすべてのファイルシステムを表示します。

POSIXオプション:
  -P                    POSIX出力形式（512バイトブロック単位）
  -k                    1024バイトブロック単位（デフォルト）

GNU拡張オプション:
  -a, --all             すべてのファイルシステムを表示（疑似FSを含む）
  -B, --block-size=SIZE ブロックサイズを指定
                        SIZE は数値と単位 (K, M, G, T, KB, MB, GB, TB)
  -h, --human-readable  人間が読みやすい形式（1024単位: K, M, G）
  -H, --si              人間が読みやすい形式（1000単位: KB, MB, GB）
  -i, --inodes          inode情報を表示
  -l, --local           ローカルファイルシステムのみ表示
  -T, --print-type      ファイルシステムタイプを表示
  -t, --type=TYPE       指定タイプのファイルシステムのみ表示
  -x, --exclude-type=TYPE
                        指定タイプのファイルシステムを除外
      --total           合計行を表示
      --help            このヘルプを表示
      --version         バージョンを表示

例:
  df                    すべてのファイルシステム
  df -h                 人間が読みやすい形式
  df -hT                タイプ付きで人間が読みやすい形式
  df -l                 ローカルファイルシステムのみ
  df -i                 inode使用状況
  df --total            合計行付き
  df /home              特定パスのファイルシステム
  df C:\\Windows\\system32\\*.dll  グロブ展開で複数パスを指定（Windows対応）"#
    );
}

/// 出力用ブロックサイズを決定
fn get_output_block_size(opts: &Options) -> u64 {
    if opts.human_readable {
        1 // 人間が読みやすい形式ではバイト単位で計算
    } else if let Some(bs) = opts.block_size {
        bs
    } else if opts.posix_format {
        512 // POSIX -P
    } else {
        1024 // デフォルト（-k と同等）
    }
}

fn print_filesystems(filesystems: &[FilesystemInfo], opts: &Options) {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    let block_size = get_output_block_size(opts);

    // ヘッダー出力
    print_header(&mut stdout, opts, block_size).unwrap();

    let mut total_size: u64 = 0;
    let mut total_used: u64 = 0;
    let mut total_avail: u64 = 0;
    let mut total_inodes: u64 = 0;
    let mut total_iused: u64 = 0;
    let mut total_ifree: u64 = 0;

    for fs in filesystems {
        print_filesystem_line(&mut stdout, fs, opts, block_size).unwrap();

        // 合計用に累積
        let fs_total_bytes = fs.total_blocks * fs.block_size;
        let fs_used_bytes = fs.used_blocks * fs.block_size;
        let fs_avail_bytes = fs.available_blocks * fs.block_size;

        total_size += fs_total_bytes;
        total_used += fs_used_bytes;
        total_avail += fs_avail_bytes;
        total_inodes += fs.total_inodes;
        total_iused += fs.used_inodes;
        total_ifree += fs.available_inodes;
    }

    // 合計行
    if opts.total && !filesystems.is_empty() {
        print_total_line(
            &mut stdout,
            opts,
            block_size,
            total_size,
            total_used,
            total_avail,
            total_inodes,
            total_iused,
            total_ifree,
        )
        .unwrap();
    }
}

fn print_header<W: Write>(w: &mut W, opts: &Options, block_size: u64) -> io::Result<()> {
    if opts.inodes {
        if opts.show_type {
            writeln!(
                w,
                "{:<20} {:<8} {:>10} {:>10} {:>10} {:>5} {}",
                "Filesystem", "Type", "Inodes", "IUsed", "IFree", "IUse%", "Mounted on"
            )
        } else {
            writeln!(
                w,
                "{:<20} {:>10} {:>10} {:>10} {:>5} {}",
                "Filesystem", "Inodes", "IUsed", "IFree", "IUse%", "Mounted on"
            )
        }
    } else {
        let size_header = if opts.human_readable {
            "Size".to_string()
        } else if opts.posix_format {
            "512-blocks".to_string()
        } else {
            format!("{}-blocks", format_block_size_header(block_size))
        };

        if opts.show_type {
            writeln!(
                w,
                "{:<20} {:<8} {:>10} {:>10} {:>10} {:>5} {}",
                "Filesystem", "Type", size_header, "Used", "Avail", "Use%", "Mounted on"
            )
        } else {
            writeln!(
                w,
                "{:<20} {:>10} {:>10} {:>10} {:>5} {}",
                "Filesystem", size_header, "Used", "Avail", "Use%", "Mounted on"
            )
        }
    }
}

fn format_block_size_header(block_size: u64) -> String {
    match block_size {
        1024 => "1K".to_string(),
        1048576 => "1M".to_string(),
        1073741824 => "1G".to_string(),
        512 => "512".to_string(),
        _ => format!("{}B", block_size),
    }
}

fn print_filesystem_line<W: Write>(
    w: &mut W,
    fs: &FilesystemInfo,
    opts: &Options,
    block_size: u64,
) -> io::Result<()> {
    if opts.inodes {
        let iuse_percent = if fs.total_inodes > 0 {
            (fs.used_inodes as f64 / fs.total_inodes as f64 * 100.0) as u64
        } else {
            0
        };

        if opts.show_type {
            writeln!(
                w,
                "{:<20} {:<8} {:>10} {:>10} {:>10} {:>4}% {}",
                truncate_str(&fs.filesystem, 20),
                &fs.fs_type,
                fs.total_inodes,
                fs.used_inodes,
                fs.available_inodes,
                iuse_percent,
                &fs.mount_point
            )
        } else {
            writeln!(
                w,
                "{:<20} {:>10} {:>10} {:>10} {:>4}% {}",
                truncate_str(&fs.filesystem, 20),
                fs.total_inodes,
                fs.used_inodes,
                fs.available_inodes,
                iuse_percent,
                &fs.mount_point
            )
        }
    } else {
        // バイト単位に変換
        let total_bytes = fs.total_blocks * fs.block_size;
        let used_bytes = fs.used_blocks * fs.block_size;
        let avail_bytes = fs.available_blocks * fs.block_size;

        let use_percent = if total_bytes > 0 {
            (used_bytes as f64 / total_bytes as f64 * 100.0).ceil() as u64
        } else {
            0
        };

        let (size_str, used_str, avail_str) = if opts.human_readable {
            let base = if opts.si { 1000 } else { 1024 };
            (
                format_human(total_bytes, base),
                format_human(used_bytes, base),
                format_human(avail_bytes, base),
            )
        } else {
            (
                format!("{}", total_bytes / block_size),
                format!("{}", used_bytes / block_size),
                format!("{}", avail_bytes / block_size),
            )
        };

        if opts.show_type {
            writeln!(
                w,
                "{:<20} {:<8} {:>10} {:>10} {:>10} {:>4}% {}",
                truncate_str(&fs.filesystem, 20),
                &fs.fs_type,
                size_str,
                used_str,
                avail_str,
                use_percent,
                &fs.mount_point
            )
        } else {
            writeln!(
                w,
                "{:<20} {:>10} {:>10} {:>10} {:>4}% {}",
                truncate_str(&fs.filesystem, 20),
                size_str,
                used_str,
                avail_str,
                use_percent,
                &fs.mount_point
            )
        }
    }
}

fn print_total_line<W: Write>(
    w: &mut W,
    opts: &Options,
    block_size: u64,
    total_size: u64,
    total_used: u64,
    total_avail: u64,
    total_inodes: u64,
    total_iused: u64,
    total_ifree: u64,
) -> io::Result<()> {
    if opts.inodes {
        let iuse_percent = if total_inodes > 0 {
            (total_iused as f64 / total_inodes as f64 * 100.0) as u64
        } else {
            0
        };

        if opts.show_type {
            writeln!(
                w,
                "{:<20} {:<8} {:>10} {:>10} {:>10} {:>4}% {}",
                "total", "-", total_inodes, total_iused, total_ifree, iuse_percent, "-"
            )
        } else {
            writeln!(
                w,
                "{:<20} {:>10} {:>10} {:>10} {:>4}% {}",
                "total", total_inodes, total_iused, total_ifree, iuse_percent, "-"
            )
        }
    } else {
        let use_percent = if total_size > 0 {
            (total_used as f64 / total_size as f64 * 100.0).ceil() as u64
        } else {
            0
        };

        let (size_str, used_str, avail_str) = if opts.human_readable {
            let base = if opts.si { 1000 } else { 1024 };
            (
                format_human(total_size, base),
                format_human(total_used, base),
                format_human(total_avail, base),
            )
        } else {
            (
                format!("{}", total_size / block_size),
                format!("{}", total_used / block_size),
                format!("{}", total_avail / block_size),
            )
        };

        if opts.show_type {
            writeln!(
                w,
                "{:<20} {:<8} {:>10} {:>10} {:>10} {:>4}% {}",
                "total", "-", size_str, used_str, avail_str, use_percent, "-"
            )
        } else {
            writeln!(
                w,
                "{:<20} {:>10} {:>10} {:>10} {:>4}% {}",
                "total", size_str, used_str, avail_str, use_percent, "-"
            )
        }
    }
}

fn format_human(bytes: u64, base: u64) -> String {
    let units = if base == 1000 {
        ["B", "KB", "MB", "GB", "TB", "PB"]
    } else {
        ["B", "K", "M", "G", "T", "P"]
    };

    if bytes == 0 {
        return "0".to_string();
    }

    let mut value = bytes as f64;
    let mut unit_index = 0;

    while value >= base as f64 && unit_index < units.len() - 1 {
        value /= base as f64;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{}{}", bytes, units[unit_index])
    } else if value >= 100.0 {
        format!("{:.0}{}", value, units[unit_index])
    } else if value >= 10.0 {
        format!("{:.1}{}", value, units[unit_index])
    } else {
        format!("{:.1}{}", value, units[unit_index])
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars().take(max_len).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_is_glob_pattern() {
        assert!(is_glob_pattern("*.txt"));
        assert!(is_glob_pattern("test?.rs"));
        assert!(is_glob_pattern("file[0-9].log"));
        assert!(!is_glob_pattern("/tmp/file.txt"));
    }

    #[test]
    fn test_expand_path_globs_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path();

        fs::write(dir_path.join("a.txt"), "x").unwrap();
        fs::write(dir_path.join("b.txt"), "x").unwrap();

        let pattern = format!("{}/*.txt", dir_path.to_string_lossy());
        let result = expand_path_globs(&[pattern]);

        assert_eq!(result.len(), 2);
        let mut sorted: Vec<String> = result.into_iter().collect();
        sorted.sort();
        assert!(sorted[0].ends_with("a.txt"));
        assert!(sorted[1].ends_with("b.txt"));
    }

    #[test]
    fn test_expand_path_globs_no_match_keeps_original() {
        let path = "no_such_file_1234567890.txt".to_string();
        let result = expand_path_globs(&[path.clone()]);
        assert_eq!(result, vec![path]);
    }
}

// ============================================================================
// Windows 実装
// ============================================================================

#[cfg(windows)]
fn get_all_filesystems(opts: &Options) -> Vec<FilesystemInfo> {
    let mut filesystems = Vec::new();

    let drive_bits = unsafe { GetLogicalDrives() };

    for i in 0..26 {
        if drive_bits & (1 << i) != 0 {
            let letter = (b'A' + i) as char;
            let root = format!("{}:\\", letter);

            if let Some(info) = get_filesystem_info_windows(&root, opts) {
                filesystems.push(info);
            }
        }
    }

    filesystems
}

#[cfg(windows)]
fn get_filesystems_for_paths(paths: &[String], opts: &Options) -> Vec<FilesystemInfo> {
    let mut filesystems = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for path in paths {
        let p = std::path::Path::new(path);
        if !p.exists() {
            eprintln!(
                "df: '{}': そのようなファイルやディレクトリはありません",
                path
            );
            continue;
        }

        let root = get_root_path_windows(path);
        if seen.contains(&root) {
            continue;
        }
        seen.insert(root.clone());

        if let Some(info) = get_filesystem_info_windows(&root, opts) {
            filesystems.push(info);
        }
    }

    filesystems
}

#[cfg(windows)]
fn get_root_path_windows(path: &str) -> String {
    if path.len() >= 2 && path.chars().nth(1) == Some(':') {
        format!("{}:\\", path.chars().next().unwrap().to_ascii_uppercase())
    } else if path.starts_with("\\\\") {
        // UNCパス
        let parts: Vec<&str> = path.split('\\').collect();
        if parts.len() >= 4 {
            format!("\\\\{}\\{}\\", parts[2], parts[3])
        } else {
            path.to_string()
        }
    } else {
        // カレントドライブ
        std::env::current_dir()
            .map(|p| {
                let s = p.to_string_lossy();
                if s.len() >= 3 {
                    s.chars().take(3).collect::<String>()
                } else {
                    "C:\\".to_string()
                }
            })
            .unwrap_or_else(|_| "C:\\".to_string())
    }
}

#[cfg(windows)]
fn get_filesystem_info_windows(root: &str, opts: &Options) -> Option<FilesystemInfo> {
    let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    let root_pcwstr = PCWSTR(root_wide.as_ptr());

    // ドライブタイプ取得
    let drive_type_val = unsafe { GetDriveTypeW(root_pcwstr) };

    let (drive_type_str, is_remote) = match drive_type_val {
        DRIVE_FIXED => ("Fixed", false),
        DRIVE_REMOVABLE => ("Removable", false),
        DRIVE_REMOTE => ("Remote", true),
        DRIVE_CDROM => ("CDROM", false),
        DRIVE_RAMDISK => ("RAMDisk", false),
        _ => return None,
    };

    // ローカルフィルタ
    if opts.local && is_remote {
        return None;
    }

    // タイプフィルタ
    if let Some(ref filter) = opts.type_filter {
        if !drive_type_str.eq_ignore_ascii_case(filter) {
            return None;
        }
    }

    // 除外フィルタ
    if let Some(ref exclude) = opts.exclude_type {
        if drive_type_str.eq_ignore_ascii_case(exclude) {
            return None;
        }
    }

    // ボリューム情報取得
    let mut volume_name: [u16; 256] = [0; 256];
    let mut fs_name: [u16; 256] = [0; 256];
    let mut serial_number: u32 = 0;
    let mut max_component_len: u32 = 0;
    let mut fs_flags: u32 = 0;

    let vol_result = unsafe {
        GetVolumeInformationW(
            root_pcwstr,
            Some(&mut volume_name),
            Some(&mut serial_number),
            Some(&mut max_component_len),
            Some(&mut fs_flags),
            Some(&mut fs_name),
        )
    };

    let fs_type = if vol_result.is_ok() {
        String::from_utf16_lossy(&fs_name[..fs_name.iter().position(|&c| c == 0).unwrap_or(0)])
    } else {
        drive_type_str.to_string()
    };

    let volume_label = if vol_result.is_ok() {
        String::from_utf16_lossy(
            &volume_name[..volume_name.iter().position(|&c| c == 0).unwrap_or(0)],
        )
    } else {
        String::new()
    };

    // 容量情報取得
    let mut free_bytes_available: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_free_bytes: u64 = 0;

    let space_result = unsafe {
        GetDiskFreeSpaceExW(
            root_pcwstr,
            Some(&mut free_bytes_available),
            Some(&mut total_bytes),
            Some(&mut total_free_bytes),
        )
    };

    if space_result.is_err() && !opts.all {
        return None;
    }

    let used_bytes = total_bytes.saturating_sub(total_free_bytes);

    let filesystem_name = if volume_label.is_empty() {
        root.trim_end_matches('\\').to_string()
    } else {
        format!("{} ({})", root.trim_end_matches('\\'), volume_label)
    };

    Some(FilesystemInfo {
        filesystem: filesystem_name,
        mount_point: root.trim_end_matches('\\').to_string(),
        fs_type,
        total_blocks: total_bytes,
        used_blocks: used_bytes,
        available_blocks: free_bytes_available,
        total_inodes: 0, // Windowsではinode情報なし
        used_inodes: 0,
        available_inodes: 0,
        block_size: 1, // バイト単位で保持
        is_remote,
    })
}

// ============================================================================
// Unix 実装
// ============================================================================

#[cfg(unix)]
fn get_all_filesystems(opts: &Options) -> Vec<FilesystemInfo> {
    let mut filesystems = Vec::new();

    // /proc/mounts または /etc/mtab を読む
    let mounts = read_mounts();

    for mount in mounts {
        if should_skip_mount(&mount, opts) {
            continue;
        }

        if let Some(info) = get_filesystem_info_unix(&mount, opts) {
            filesystems.push(info);
        }
    }

    filesystems
}

#[cfg(unix)]
fn get_filesystems_for_paths(paths: &[String], opts: &Options) -> Vec<FilesystemInfo> {
    let mut filesystems = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mounts = read_mounts();

    for path in paths {
        // パスからマウントポイントを特定
        let mount_point = find_mount_point(path, &mounts);

        if let Some(mp) = mount_point {
            if seen.contains(&mp.mount_point) {
                continue;
            }
            seen.insert(mp.mount_point.clone());

            if let Some(info) = get_filesystem_info_unix(&mp, opts) {
                filesystems.push(info);
            }
        } else {
            eprintln!(
                "df: '{}': そのようなファイルやディレクトリはありません",
                path
            );
        }
    }

    filesystems
}

#[cfg(unix)]
struct MountEntry {
    device: String,
    mount_point: String,
    fs_type: String,
}

#[cfg(unix)]
fn read_mounts() -> Vec<MountEntry> {
    let mut entries = Vec::new();

    // /proc/mounts を優先、なければ /etc/mtab
    let content = std::fs::read_to_string("/proc/mounts")
        .or_else(|_| std::fs::read_to_string("/etc/mtab"))
        .unwrap_or_default();

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            entries.push(MountEntry {
                device: parts[0].to_string(),
                mount_point: parts[1].to_string(),
                fs_type: parts[2].to_string(),
            });
        }
    }

    entries
}

#[cfg(unix)]
fn should_skip_mount(mount: &MountEntry, opts: &Options) -> bool {
    // 疑似ファイルシステムをスキップ（-a がない場合）
    if !opts.all {
        let pseudo_types = [
            "sysfs",
            "proc",
            "devtmpfs",
            "devpts",
            "tmpfs",
            "securityfs",
            "cgroup",
            "cgroup2",
            "pstore",
            "debugfs",
            "hugetlbfs",
            "mqueue",
            "fusectl",
            "configfs",
            "binfmt_misc",
            "autofs",
            "efivarfs",
            "bpf",
            "tracefs",
        ];

        if pseudo_types.contains(&mount.fs_type.as_str()) {
            return true;
        }

        // /dev, /sys, /proc で始まるものをスキップ
        if mount.mount_point.starts_with("/dev/")
            || mount.mount_point.starts_with("/sys/")
            || mount.mount_point.starts_with("/proc/")
        {
            return true;
        }
    }

    // ローカルフィルタ
    if opts.local {
        let remote_types = ["nfs", "nfs4", "cifs", "smbfs", "ncpfs", "afs", "fuse.sshfs"];
        if remote_types.contains(&mount.fs_type.as_str()) {
            return true;
        }
    }

    // タイプフィルタ
    if let Some(ref filter) = opts.type_filter {
        if !mount.fs_type.eq_ignore_ascii_case(filter) {
            return true;
        }
    }

    // 除外フィルタ
    if let Some(ref exclude) = opts.exclude_type {
        if mount.fs_type.eq_ignore_ascii_case(exclude) {
            return true;
        }
    }

    false
}

#[cfg(unix)]
fn find_mount_point<'a>(path: &str, mounts: &'a [MountEntry]) -> Option<&'a MountEntry> {
    let path = Path::new(path);
    let canonical = std::fs::canonicalize(path).ok()?;
    let path_str = canonical.to_string_lossy();

    // 最長一致するマウントポイントを探す
    let mut best_match: Option<&MountEntry> = None;
    let mut best_len = 0;

    for mount in mounts {
        if path_str.starts_with(&mount.mount_point) || path_str == mount.mount_point {
            if mount.mount_point.len() > best_len {
                best_len = mount.mount_point.len();
                best_match = Some(mount);
            }
        }
    }

    best_match
}

#[cfg(unix)]
fn get_filesystem_info_unix(mount: &MountEntry, opts: &Options) -> Option<FilesystemInfo> {
    let path_cstr = CString::new(mount.mount_point.as_bytes()).ok()?;

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };

    let result = unsafe { libc::statvfs(path_cstr.as_ptr(), &mut stat) };

    if result != 0 {
        return None;
    }

    let block_size = stat.f_frsize as u64;
    let total_blocks = stat.f_blocks as u64;
    let free_blocks = stat.f_bfree as u64;
    let available_blocks = stat.f_bavail as u64;
    let used_blocks = total_blocks.saturating_sub(free_blocks);

    let total_inodes = stat.f_files as u64;
    let free_inodes = stat.f_ffree as u64;
    let available_inodes = stat.f_favail as u64;
    let used_inodes = total_inodes.saturating_sub(free_inodes);

    let remote_types = ["nfs", "nfs4", "cifs", "smbfs", "ncpfs", "afs", "fuse.sshfs"];
    let is_remote = remote_types.contains(&mount.fs_type.as_str());

    Some(FilesystemInfo {
        filesystem: mount.device.clone(),
        mount_point: mount.mount_point.clone(),
        fs_type: mount.fs_type.clone(),
        total_blocks,
        used_blocks,
        available_blocks,
        total_inodes,
        used_inodes,
        available_inodes,
        block_size,
        is_remote,
    })
}
