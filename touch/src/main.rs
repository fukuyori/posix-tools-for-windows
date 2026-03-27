// touch - ファイルのタイムスタンプを変更
// POSIX.1-2017準拠 + GNU拡張

use std::env;
use std::fs::File;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, NaiveDateTime, TimeZone};
use glob;

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    access_only: bool,       // -a: アクセス時間のみ変更
    modification_only: bool, // -m: 更新時間のみ変更
    no_create: bool,         // -c: ファイルを作成しない
    reference: Option<String>, // -r: 参照ファイル
    timestamp: Option<String>, // -t: タイムスタンプ指定
    
    // GNU拡張オプション
    date: Option<String>,    // -d, --date: 日時文字列
    no_dereference: bool,    // -h: シンボリックリンク自体を変更
    
    show_help: bool,
    show_version: bool,
}

#[derive(Clone, Copy)]
struct Timestamps {
    access: SystemTime,
    modification: SystemTime,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("touch: {}", e);
            eprintln!("詳細は 'touch --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("touch (Rust版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張");
        std::process::exit(0);
    }

    if files.is_empty() {
        eprintln!("touch: オペランドがありません");
        eprintln!("詳細は 'touch --help' を参照してください");
        std::process::exit(1);
    }

    // glob展開
    let files = expand_globs(files, opts.no_create);

    let timestamps = match determine_timestamps(&opts) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("touch: {}", e);
            std::process::exit(1);
        }
    };

    let mut exit_code = 0;

    for file in &files {
        if let Err(e) = touch_file(file, &opts, timestamps) {
            eprintln!("touch: '{}': {}", file, format_error(&e));
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
                "--no-create" => opts.no_create = true,
                "--no-dereference" => opts.no_dereference = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                s if s.starts_with("--date=") => {
                    opts.date = Some(s.trim_start_matches("--date=").to_string());
                }
                s if s.starts_with("--reference=") => {
                    opts.reference = Some(s.trim_start_matches("--reference=").to_string());
                }
                s if s.starts_with("--time=") => {
                    let val = s.trim_start_matches("--time=");
                    match val {
                        "access" | "atime" | "use" => opts.access_only = true,
                        "modify" | "mtime" => opts.modification_only = true,
                        _ => return Err(format!("無効な --time の引数: '{}'", val)),
                    }
                }
                "--date" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--date' には引数が必要です".to_string());
                    }
                    opts.date = Some(args[i].clone());
                }
                "--reference" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--reference' には引数が必要です".to_string());
                    }
                    opts.reference = Some(args[i].clone());
                }
                "--time" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("オプション '--time' には引数が必要です".to_string());
                    }
                    match args[i].as_str() {
                        "access" | "atime" | "use" => opts.access_only = true,
                        "modify" | "mtime" => opts.modification_only = true,
                        _ => return Err(format!("無効な --time の引数: '{}'", args[i])),
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

            while j < chars.len() {
                match chars[j] {
                    // POSIX標準
                    'a' => opts.access_only = true,
                    'm' => opts.modification_only = true,
                    'c' => opts.no_create = true,
                    'r' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.reference = Some(rest);
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-r' には引数が必要です".to_string());
                            }
                            opts.reference = Some(args[i].clone());
                            break;
                        }
                    }
                    't' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.timestamp = Some(rest);
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-t' には引数が必要です".to_string());
                            }
                            opts.timestamp = Some(args[i].clone());
                            break;
                        }
                    }
                    // GNU拡張
                    'd' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.date = Some(rest);
                            break;
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return Err("オプション '-d' には引数が必要です".to_string());
                            }
                            opts.date = Some(args[i].clone());
                            break;
                        }
                    }
                    'h' => opts.no_dereference = true,
                    'f' => {} // 互換性のため無視（常に成功）
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

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_files: Vec<String>, _no_create: bool) -> Vec<String> {
    let mut result = Vec::new();
    
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    
    for pattern in raw_files {
        // glob文字が含まれない場合はそのまま
        if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
            result.push(pattern);
            continue;
        }
        
        match glob::glob_with(&pattern, options) {
            Ok(paths) => {
                let mut matched = false;
                for entry in paths {
                    if let Ok(path) = entry {
                        result.push(path.to_string_lossy().to_string());
                        matched = true;
                    }
                }
                // Linux のシェル展開に寄せて、未一致時はリテラルのまま残す
                if !matched {
                    result.push(pattern);
                }
            }
            Err(_) => {
                // 不正な glob として解釈できない場合もリテラルとして扱う
                result.push(pattern);
            }
        }
    }

    result
}

fn print_help() {
    println!(r#"使い方: touch [オプション]... ファイル...

各ファイルのアクセス時刻と更新時刻を現在時刻に変更します。
ファイルが存在しない場合は、空のファイルを作成します。

-a と -m の両方が指定された場合、または両方とも指定されていない場合は、
アクセス時刻と更新時刻の両方が変更されます。

POSIX標準オプション:
  -a                    アクセス時刻のみ変更
  -c, --no-create       ファイルを作成しない
  -m                    更新時刻のみ変更
  -r, --reference=FILE  現在時刻の代わりに FILE の時刻を使用
  -t STAMP              現在時刻の代わりに [[CC]YY]MMDDhhmm[.ss] を使用

GNU拡張オプション:
  -d, --date=STRING     STRING を解析して現在時刻の代わりに使用
  -f                    無視（BSD互換性のため）
  -h, --no-dereference  シンボリックリンク自体の時刻を変更
      --time=WORD       変更する時刻を指定
                        access, atime, use: -a と同等
                        modify, mtime: -m と同等
      --help            このヘルプを表示して終了
      --version         バージョン情報を表示して終了

タイムスタンプ形式 (-t):
  MMDDhhmm              月日時分（今年）
  YYMMDDhhmm            年月日時分（2桁年: 69-99→19xx, 00-68→20xx）
  CCYYMMDDhhmm          年月日時分（4桁年）
  CCYYMMDDhhmm.ss       年月日時分秒

日時文字列形式 (-d):
  "2024-01-15 10:30:00"
  "2024-01-15 10:30"
  "2024-01-15"
  "2024/01/15 10:30:00"
  "now"
  "yesterday"
  "tomorrow"
  "+1 day", "-2 hours", "+30 minutes"

終了ステータス:
  0  正常終了
  1  エラー発生
  2  オプションエラー

例:
  touch file.txt                  現在時刻に更新
  touch -c file.txt               存在する場合のみ更新
  touch -a file.txt               アクセス時刻のみ更新
  touch -m file.txt               更新時刻のみ更新
  touch -d "2024-01-15" file      指定日時に設定
  touch -t 202401151030 file      タイムスタンプ形式で指定
  touch -r ref.txt file.txt       ref.txt と同じ時刻に設定
  touch *.txt                     すべてのtxtファイルを更新"#);
}

fn determine_timestamps(opts: &Options) -> Result<Timestamps, String> {
    // 優先順位: -r > -t > -d > 現在時刻
    
    if let Some(ref path) = opts.reference {
        return get_reference_timestamps(path)
            .map_err(|e| format!("参照ファイル '{}': {}", path, format_error(&e)));
    }

    if let Some(ref stamp) = opts.timestamp {
        let timestamp = parse_timestamp(stamp)
            .ok_or_else(|| format!("無効なタイムスタンプ形式: '{}'", stamp))?;
        return Ok(Timestamps {
            access: timestamp,
            modification: timestamp,
        });
    }

    if let Some(ref date_str) = opts.date {
        let timestamp = parse_date_string(date_str)
            .ok_or_else(|| format!("無効な日時形式: '{}'", date_str))?;
        return Ok(Timestamps {
            access: timestamp,
            modification: timestamp,
        });
    }

    let now = SystemTime::now();
    Ok(Timestamps {
        access: now,
        modification: now,
    })
}

fn parse_date_string(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    let now = Local::now();

    // 特殊キーワード
    match s.to_lowercase().as_str() {
        "now" => return Some(SystemTime::now()),
        "today" => {
            let dt = now.date_naive().and_hms_opt(0, 0, 0)?;
            let local_dt = Local.from_local_datetime(&dt).single()?;
            return Some(local_dt.into());
        }
        "yesterday" => {
            let dt = (now - chrono::Duration::days(1)).date_naive().and_hms_opt(0, 0, 0)?;
            let local_dt = Local.from_local_datetime(&dt).single()?;
            return Some(local_dt.into());
        }
        "tomorrow" => {
            let dt = (now + chrono::Duration::days(1)).date_naive().and_hms_opt(0, 0, 0)?;
            let local_dt = Local.from_local_datetime(&dt).single()?;
            return Some(local_dt.into());
        }
        _ => {}
    }

    // 相対時間 (+1 day, -2 hours など)
    if let Some(time) = parse_relative_time(s) {
        return Some(time);
    }

    // 標準的な日時形式
    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%Y/%m/%d",
        "%Y%m%d %H:%M:%S",
        "%Y%m%d %H:%M",
        "%Y%m%d",
        "%d %b %Y %H:%M:%S",
        "%d %b %Y",
        "%b %d, %Y %H:%M:%S",
        "%b %d, %Y",
    ];

    for fmt in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            let local_dt = Local.from_local_datetime(&dt).single()?;
            return Some(local_dt.into());
        }
        // 日付のみの形式
        if let Ok(date) = chrono::NaiveDate::parse_from_str(s, fmt) {
            let dt = date.and_hms_opt(0, 0, 0)?;
            let local_dt = Local.from_local_datetime(&dt).single()?;
            return Some(local_dt.into());
        }
    }

    None
}

fn parse_relative_time(s: &str) -> Option<SystemTime> {
    let s = s.trim().to_lowercase();
    let now = Local::now();
    
    // パターン: +1 day, -2 hours, +30 minutes, など
    // 正規表現を使わずにパース
    
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    
    // 数値部分を抽出
    let mut num_end = 0;
    let chars: Vec<char> = s.chars().collect();
    
    // 符号をスキップ
    if !chars.is_empty() && (chars[0] == '+' || chars[0] == '-') {
        num_end = 1;
    }
    
    // 数字をスキップ
    while num_end < chars.len() && chars[num_end].is_ascii_digit() {
        num_end += 1;
    }
    
    if num_end == 0 || (num_end == 1 && (chars[0] == '+' || chars[0] == '-')) {
        return None;
    }
    
    let num_str: String = chars[..num_end].iter().collect();
    let num: i64 = num_str.parse().ok()?;
    
    // 単位部分を抽出
    let unit_str: String = chars[num_end..].iter().collect();
    let unit_str = unit_str.trim().to_lowercase();
    
    // 末尾の 's' を除去（複数形対応）
    let unit = unit_str.trim_end_matches('s');
    
    let duration = match unit {
        "second" | "sec" => chrono::Duration::seconds(num),
        "minute" | "min" => chrono::Duration::minutes(num),
        "hour" | "hr" | "h" => chrono::Duration::hours(num),
        "day" | "d" => chrono::Duration::days(num),
        "week" | "wk" | "w" => chrono::Duration::weeks(num),
        "month" | "mon" => chrono::Duration::days(num * 30), // 近似
        "year" | "yr" | "y" => chrono::Duration::days(num * 365), // 近似
        _ => return None,
    };
    
    let new_time = now + duration;
    Some(new_time.into())
}

fn parse_timestamp(s: &str) -> Option<SystemTime> {
    let (datetime_part, seconds) = if s.contains('.') {
        let parts: Vec<&str> = s.splitn(2, '.').collect();
        (parts[0], parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0))
    } else {
        (s, 0)
    };

    // 数字のみであることを確認
    if !datetime_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let now = Local::now();
    let len = datetime_part.len();

    let (year, month, day, hour, minute) = match len {
        // MMDDhhmm - 今年
        8 => {
            let month: u32 = datetime_part[0..2].parse().ok()?;
            let day: u32 = datetime_part[2..4].parse().ok()?;
            let hour: u32 = datetime_part[4..6].parse().ok()?;
            let minute: u32 = datetime_part[6..8].parse().ok()?;
            (now.year(), month, day, hour, minute)
        }
        // YYMMDDhhmm - 2桁年
        10 => {
            let year: i32 = datetime_part[0..2].parse().ok()?;
            // POSIX: 69-99 → 1969-1999, 00-68 → 2000-2068
            let year = if year >= 69 { 1900 + year } else { 2000 + year };
            let month: u32 = datetime_part[2..4].parse().ok()?;
            let day: u32 = datetime_part[4..6].parse().ok()?;
            let hour: u32 = datetime_part[6..8].parse().ok()?;
            let minute: u32 = datetime_part[8..10].parse().ok()?;
            (year, month, day, hour, minute)
        }
        // CCYYMMDDhhmm - 4桁年
        12 => {
            let year: i32 = datetime_part[0..4].parse().ok()?;
            let month: u32 = datetime_part[4..6].parse().ok()?;
            let day: u32 = datetime_part[6..8].parse().ok()?;
            let hour: u32 = datetime_part[8..10].parse().ok()?;
            let minute: u32 = datetime_part[10..12].parse().ok()?;
            (year, month, day, hour, minute)
        }
        _ => return None,
    };

    // 範囲チェック
    if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59 || seconds > 59 {
        return None;
    }

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, seconds)?;
    let dt = NaiveDateTime::new(date, time);
    let local_dt = Local.from_local_datetime(&dt).single()?;

    Some(local_dt.into())
}

fn touch_file(path: &str, opts: &Options, timestamps: Timestamps) -> io::Result<()> {
    let path = Path::new(path);

    // ファイルが存在しない場合
    if !path.exists() {
        if opts.no_create {
            return Ok(());
        }

        // ディレクトリパスの場合はエラー
        let path_str = path.to_string_lossy();
        if path_str.ends_with('/') || path_str.ends_with('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ディレクトリは作成できません",
            ));
        }

        // 親ディレクトリが存在しない場合はエラー
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("親ディレクトリ '{}' が存在しません", parent.display()),
                ));
            }
        }

        File::create(path)?;
    }

    // ディレクトリの場合もタイムスタンプを変更
    set_file_times(path, opts, timestamps)?;

    Ok(())
}

fn set_file_times(path: &Path, opts: &Options, timestamps: Timestamps) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, SetFileTime, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
        OPEN_EXISTING,
    };

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

    unsafe {
        // ディレクトリも扱えるように FILE_FLAG_BACKUP_SEMANTICS を指定
        // -h オプションの場合は FILE_FLAG_OPEN_REPARSE_POINT でシンボリックリンク自体を開く
        let mut flags = FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS;
        if opts.no_dereference {
            flags |= FILE_FLAG_OPEN_REPARSE_POINT;
        }

        let handle = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_WRITE_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            HANDLE::default(),
        )?;

        let access_filetime = systemtime_to_filetime(timestamps.access);
        let modification_filetime = systemtime_to_filetime(timestamps.modification);

        // -a のみ: アクセス時刻のみ変更
        // -m のみ: 更新時刻のみ変更
        // 両方または両方なし: 両方変更
        let (access_time, modification_time): (Option<*const FILETIME>, Option<*const FILETIME>) =
            if opts.access_only && !opts.modification_only {
                (Some(&access_filetime as *const FILETIME), None)
            } else if opts.modification_only && !opts.access_only {
                (None, Some(&modification_filetime as *const FILETIME))
            } else {
                (
                    Some(&access_filetime as *const FILETIME),
                    Some(&modification_filetime as *const FILETIME),
                )
            };

        let result = SetFileTime(handle, None, access_time, modification_time);
        let _ = CloseHandle(handle);

        result?;
    }

    Ok(())
}

fn systemtime_to_filetime(time: SystemTime) -> windows::Win32::Foundation::FILETIME {
    // Windows FILETIME: 1601-01-01 からの100ナノ秒単位
    // Unix epoch: 1970-01-01
    const UNIX_TO_WINDOWS_EPOCH: u64 = 11644473600;
    const WINDOWS_TICKS_PER_SEC: u64 = 10_000_000;

    let intervals = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs() + UNIX_TO_WINDOWS_EPOCH;
            let nanos = duration.subsec_nanos() as u64;
            secs * WINDOWS_TICKS_PER_SEC + nanos / 100
        }
        Err(err) => {
            let duration = err.duration();
            let delta = duration.as_secs() * WINDOWS_TICKS_PER_SEC
                + duration.subsec_nanos() as u64 / 100;
            UNIX_TO_WINDOWS_EPOCH * WINDOWS_TICKS_PER_SEC - delta
        }
    };

    windows::Win32::Foundation::FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    }
}

fn get_reference_timestamps(path: &str) -> io::Result<Timestamps> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileTime, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide_path: Vec<u16> = Path::new(path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS,
            HANDLE::default(),
        )?;

        let mut access_time = FILETIME::default();
        let mut modification_time = FILETIME::default();
        let result = GetFileTime(handle, None, Some(&mut access_time), Some(&mut modification_time));
        let _ = CloseHandle(handle);
        result?;

        Ok(Timestamps {
            access: filetime_to_systemtime(access_time)?,
            modification: filetime_to_systemtime(modification_time)?,
        })
    }
}

fn filetime_to_systemtime(filetime: windows::Win32::Foundation::FILETIME) -> io::Result<SystemTime> {
    const UNIX_TO_WINDOWS_EPOCH: u64 = 11644473600;
    const WINDOWS_TICKS_PER_SEC: u64 = 10_000_000;

    let intervals = ((filetime.dwHighDateTime as u64) << 32) | filetime.dwLowDateTime as u64;
    let unix_epoch_intervals = UNIX_TO_WINDOWS_EPOCH * WINDOWS_TICKS_PER_SEC;

    if intervals >= unix_epoch_intervals {
        let delta = intervals - unix_epoch_intervals;
        Ok(UNIX_EPOCH
            + Duration::from_secs(delta / WINDOWS_TICKS_PER_SEC)
            + Duration::from_nanos((delta % WINDOWS_TICKS_PER_SEC) * 100))
    } else {
        let delta = unix_epoch_intervals - intervals;
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(delta / WINDOWS_TICKS_PER_SEC))
            .and_then(|t| t.checked_sub(Duration::from_nanos((delta % WINDOWS_TICKS_PER_SEC) * 100)))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "時刻の変換に失敗しました"))
    }
}

fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::AlreadyExists => "すでに存在します".to_string(),
        io::ErrorKind::InvalidInput => e.to_string(),
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
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_temp_dir() -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "touch-tests-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn expand_globs_matches_case_insensitively_on_windows() {
        let dir = make_temp_dir();
        let file = dir.join("MixedCase.TXT");
        File::create(&file).unwrap();

        let pattern = dir.join("*.txt").to_string_lossy().to_string();
        let expanded = expand_globs(vec![pattern], false);

        assert_eq!(expanded, vec![file.to_string_lossy().to_string()]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn expand_globs_keeps_unmatched_pattern_literal() {
        let dir = make_temp_dir();
        let pattern = dir.join("*.missing").to_string_lossy().to_string();

        let expanded = expand_globs(vec![pattern.clone()], true);

        assert_eq!(expanded, vec![pattern]);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn filetime_roundtrip_supports_pre_unix_epoch() {
        let timestamp = UNIX_EPOCH - Duration::from_secs(1);

        let roundtrip = filetime_to_systemtime(systemtime_to_filetime(timestamp)).unwrap();

        assert_eq!(roundtrip, timestamp);
    }
}
