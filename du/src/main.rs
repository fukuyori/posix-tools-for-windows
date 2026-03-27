use std::collections::HashSet;
use std::env;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use glob;

/// オプション構造体
#[derive(Default)]
struct Options {
    // POSIX オプション
    all: bool,                    // -a: すべてのファイルを表示
    dereference_args: bool,       // -H: コマンドライン引数のシンボリックリンクを辿る
    block_size_k: bool,           // -k: 1024バイト単位
    dereference: bool,            // -L: すべてのシンボリックリンクを辿る
    summarize: bool,              // -s: サマリーのみ
    one_file_system: bool,        // -x: 同一ファイルシステムのみ

    // GNU 拡張オプション
    apparent_size: bool,          // --apparent-size: 実サイズ
    block_size: Option<u64>,      // -B, --block-size: ブロックサイズ指定
    bytes: bool,                  // -b, --bytes: バイト単位
    total: bool,                  // -c, --total: 合計を表示
    max_depth: Option<usize>,     // -d, --max-depth: 最大深さ
    human_readable: bool,         // -h, --human-readable: 人間が読みやすい形式
    si: bool,                     // --si: SI単位（1000単位）
    no_dereference: bool,         // -P, --no-dereference: シンボリックリンクを辿らない
    separate_dirs: bool,          // -S, --separate-dirs: サブディレクトリを含めない
    threshold: Option<i64>,       // -t, --threshold: しきい値
    time: bool,                   // --time: 最終更新時刻を表示
    exclude: Vec<String>,         // --exclude: 除外パターン
    null: bool,                   // -0, --null: null区切り
    count_links: bool,            // -l, --count-links: ハードリンクを複数回カウント

    // 制御フラグ
    show_help: bool,
    show_version: bool,
}

/// ディレクトリ統計
struct DirStats {
    size: u64,
}

/// 見たファイルを追跡するためのファイル識別子
#[cfg(windows)]
#[derive(Hash, Eq, PartialEq, Clone)]
struct FileId {
    volume_serial: u32,
    file_index: u64,
}

#[cfg(not(windows))]
#[derive(Hash, Eq, PartialEq, Clone)]
struct FileId {
    dev: u64,
    ino: u64,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let result = parse_args(&args);

    let (opts, paths) = match result {
        Ok((opts, paths)) => (opts, paths),
        Err(e) => {
            eprintln!("du: {}", e);
            std::process::exit(1);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("du 1.0.0 (Rust Windows版 - POSIX準拠)");
        std::process::exit(0);
    }

    // -a と -s は排他的（POSIX準拠）
    if opts.all && opts.summarize {
        eprintln!("du: -a と -s を同時に指定することはできません");
        std::process::exit(1);
    }

    let paths = if paths.is_empty() {
        vec![".".to_string()]
    } else {
        paths
    };

    let mut grand_total: u64 = 0;
    let mut seen_files: HashSet<FileId> = HashSet::new();
    let mut exit_code = 0;

    for path_str in &paths {
        let path = Path::new(path_str);

        // パスの存在確認
        let metadata = if opts.dereference_args || opts.dereference {
            fs::metadata(path)
        } else {
            fs::symlink_metadata(path)
        };

        match metadata {
            Ok(_meta) => {
                // -x オプション用のデバイスIDを取得
                let root_device = get_device_id_from_path(path);

                match calculate_size(path, &opts, 0, &mut seen_files, root_device, true) {
                    Ok(stats) => {
                        grand_total += stats.size;
                    }
                    Err(e) => {
                        eprintln!("du: '{}': {}", path.display(), format_error(&e));
                        exit_code = 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("du: '{}' にアクセスできません: {}", path.display(), format_error(&e));
                exit_code = 1;
            }
        }
    }

    if opts.total {
        print_size(grand_total, "合計", &opts);
    }

    std::process::exit(exit_code);
}

/// 引数を解析
fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options::default();
    let mut paths = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            paths.extend(args[i + 1..].iter().cloned());
            break;
        }

        // ロングオプション
        if arg.starts_with("--") {
            match arg.as_str() {
                "--all" => opts.all = true,
                "--bytes" => {
                    opts.bytes = true;
                    opts.apparent_size = true;
                    opts.block_size = Some(1);
                }
                "--total" => opts.total = true,
                "--human-readable" => opts.human_readable = true,
                "--summarize" => opts.summarize = true,
                "--separate-dirs" => opts.separate_dirs = true,
                "--null" => opts.null = true,
                "--si" => {
                    opts.human_readable = true;
                    opts.si = true;
                }
                "--apparent-size" => opts.apparent_size = true,
                "--dereference" => opts.dereference = true,
                "--dereference-args" => opts.dereference_args = true,
                "--no-dereference" => opts.no_dereference = true,
                "--one-file-system" => opts.one_file_system = true,
                "--count-links" => opts.count_links = true,
                "--time" => opts.time = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                s if s.starts_with("--max-depth=") => {
                    let val = s.trim_start_matches("--max-depth=");
                    opts.max_depth = Some(val.parse().map_err(|_| {
                        format!("無効な最大深さ '{}'", val)
                    })?);
                }
                s if s.starts_with("--block-size=") => {
                    let val = s.trim_start_matches("--block-size=");
                    opts.block_size = Some(parse_block_size(val).ok_or_else(|| {
                        format!("無効なブロックサイズ '{}'", val)
                    })?);
                }
                s if s.starts_with("--threshold=") => {
                    let val = s.trim_start_matches("--threshold=");
                    opts.threshold = Some(parse_threshold(val).ok_or_else(|| {
                        format!("無効なしきい値 '{}'", val)
                    })?);
                }
                s if s.starts_with("--exclude=") => {
                    opts.exclude.push(s.trim_start_matches("--exclude=").to_string());
                }
                "--max-depth" => {
                    let val = args.get(i + 1).ok_or("--max-depth にはオプション引数が必要です")?;
                    opts.max_depth = Some(val.parse().map_err(|_| {
                        format!("無効な最大深さ '{}'", val)
                    })?);
                    i += 1;
                }
                "--block-size" => {
                    let val = args.get(i + 1).ok_or("--block-size にはオプション引数が必要です")?;
                    opts.block_size = Some(parse_block_size(val).ok_or_else(|| {
                        format!("無効なブロックサイズ '{}'", val)
                    })?);
                    i += 1;
                }
                "--threshold" => {
                    let val = args.get(i + 1).ok_or("--threshold にはオプション引数が必要です")?;
                    opts.threshold = Some(parse_threshold(val).ok_or_else(|| {
                        format!("無効なしきい値 '{}'", val)
                    })?);
                    i += 1;
                }
                "--exclude" => {
                    let val = args.get(i + 1).ok_or("--exclude にはオプション引数が必要です")?;
                    opts.exclude.push(val.clone());
                    i += 1;
                }
                _ => {
                    return Err(format!("認識できないオプション '{}'", arg));
                }
            }
            i += 1;
            continue;
        }

        // ショートオプション（結合対応）
        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;

            while j < chars.len() {
                match chars[j] {
                    // POSIX オプション
                    'a' => opts.all = true,
                    'H' => {
                        opts.dereference_args = true;
                        opts.no_dereference = false;
                    }
                    'k' => opts.block_size_k = true,
                    'L' => {
                        opts.dereference = true;
                        opts.no_dereference = false;
                    }
                    's' => opts.summarize = true,
                    'x' => opts.one_file_system = true,

                    // GNU 拡張オプション
                    'b' => {
                        opts.bytes = true;
                        opts.apparent_size = true;
                        opts.block_size = Some(1);
                    }
                    'c' => opts.total = true,
                    'h' => opts.human_readable = true,
                    'l' => opts.count_links = true,
                    'P' => {
                        opts.no_dereference = true;
                        opts.dereference = false;
                        opts.dereference_args = false;
                    }
                    'S' => opts.separate_dirs = true,
                    '0' => opts.null = true,
                    'd' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.max_depth = Some(rest.parse().map_err(|_| {
                                format!("無効な最大深さ '{}'", rest)
                            })?);
                            j = chars.len(); // ループ終了
                            continue;
                        } else if let Some(val) = args.get(i + 1) {
                            opts.max_depth = Some(val.parse().map_err(|_| {
                                format!("無効な最大深さ '{}'", val)
                            })?);
                            i += 1;
                            j = chars.len();
                            continue;
                        } else {
                            return Err("-d にはオプション引数が必要です".to_string());
                        }
                    }
                    'B' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.block_size = Some(parse_block_size(&rest).ok_or_else(|| {
                                format!("無効なブロックサイズ '{}'", rest)
                            })?);
                            j = chars.len();
                            continue;
                        } else if let Some(val) = args.get(i + 1) {
                            opts.block_size = Some(parse_block_size(val).ok_or_else(|| {
                                format!("無効なブロックサイズ '{}'", val)
                            })?);
                            i += 1;
                            j = chars.len();
                            continue;
                        } else {
                            return Err("-B にはオプション引数が必要です".to_string());
                        }
                    }
                    't' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.threshold = Some(parse_threshold(&rest).ok_or_else(|| {
                                format!("無効なしきい値 '{}'", rest)
                            })?);
                            j = chars.len();
                            continue;
                        } else if let Some(val) = args.get(i + 1) {
                            opts.threshold = Some(parse_threshold(val).ok_or_else(|| {
                                format!("無効なしきい値 '{}'", val)
                            })?);
                            i += 1;
                            j = chars.len();
                            continue;
                        } else {
                            return Err("-t にはオプション引数が必要です".to_string());
                        }
                    }
                    _ => {
                        return Err(format!("無効なオプション -- '{}'", chars[j]));
                    }
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        // 引数（パス）
        paths.push(arg.clone());
        i += 1;
    }

    // glob展開
    let paths = expand_globs(paths);

    Ok((opts, paths))
}

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_paths: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    
    // Windowsでは大文字小文字を区別しない
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    
    for pattern in raw_paths {
        // Windows でも POSIX シェルに近い感覚で扱えるように、
        // \ 区切りや [] も glob メタ文字として解釈する。
        if contains_glob_metachar(&pattern) {
            let normalized = normalize_glob_pattern(&pattern);
            match glob::glob_with(&normalized, options) {
                Ok(paths) => {
                    let mut matched = false;
                    for entry in paths {
                        if let Ok(path) = entry {
                            let path: PathBuf = path;
                            result.push(path.to_string_lossy().to_string());
                            matched = true;
                        }
                    }
                    if !matched {
                        // マッチなしの場合は元のパターンをそのまま（エラー表示用）
                        result.push(pattern);
                    }
                }
                Err(_) => {
                    // glob解析エラーの場合も元のパターンをそのまま
                    result.push(pattern);
                }
            }
        } else {
            result.push(pattern);
        }
    }
    
    result
}

/// ブロックサイズを解析
fn parse_block_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // GNU coreutils形式: K, M, G, KB, MB, GB, KiB, MiB, GiB
    let s_upper = s.to_uppercase();

    // 接尾辞を解析
    let (num_str, multiplier) = if s_upper.ends_with("KIB") || s_upper.ends_with("K") {
        let end = if s_upper.ends_with("KIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024u64)
    } else if s_upper.ends_with("MIB") || s_upper.ends_with("M") {
        let end = if s_upper.ends_with("MIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024u64 * 1024)
    } else if s_upper.ends_with("GIB") || s_upper.ends_with("G") {
        let end = if s_upper.ends_with("GIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024u64 * 1024 * 1024)
    } else if s_upper.ends_with("TIB") || s_upper.ends_with("T") {
        let end = if s_upper.ends_with("TIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024u64 * 1024 * 1024 * 1024)
    } else if s_upper.ends_with("KB") {
        (&s[..s.len() - 2], 1000u64)
    } else if s_upper.ends_with("MB") {
        (&s[..s.len() - 2], 1000u64 * 1000)
    } else if s_upper.ends_with("GB") {
        (&s[..s.len() - 2], 1000u64 * 1000 * 1000)
    } else if s_upper.ends_with("TB") {
        (&s[..s.len() - 2], 1000u64 * 1000 * 1000 * 1000)
    } else {
        (s, 1u64)
    };

    if num_str.is_empty() {
        Some(multiplier)
    } else {
        num_str.parse::<u64>().ok().map(|n| n * multiplier)
    }
}

/// しきい値を解析
fn parse_threshold(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let negative = s.starts_with('-');
    let s = s.trim_start_matches('-').trim_start_matches('+');
    let s_upper = s.to_uppercase();

    let (num_str, multiplier) = if s_upper.ends_with("KIB") || s_upper.ends_with("K") {
        let end = if s_upper.ends_with("KIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024i64)
    } else if s_upper.ends_with("MIB") || s_upper.ends_with("M") {
        let end = if s_upper.ends_with("MIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024i64 * 1024)
    } else if s_upper.ends_with("GIB") || s_upper.ends_with("G") {
        let end = if s_upper.ends_with("GIB") { s.len() - 3 } else { s.len() - 1 };
        (&s[..end], 1024i64 * 1024 * 1024)
    } else if s_upper.ends_with("KB") {
        (&s[..s.len() - 2], 1000i64)
    } else if s_upper.ends_with("MB") {
        (&s[..s.len() - 2], 1000i64 * 1000)
    } else if s_upper.ends_with("GB") {
        (&s[..s.len() - 2], 1000i64 * 1000 * 1000)
    } else {
        (s, 1i64)
    };

    if num_str.is_empty() {
        Some(if negative { -multiplier } else { multiplier })
    } else {
        num_str.parse::<i64>().ok().map(|n| {
            let val = n * multiplier;
            if negative { -val } else { val }
        })
    }
}

/// ヘルプを表示
fn print_help() {
    println!(
        r#"使い方: du [オプション]... [ファイル]...

各ファイルのディスク使用量を集計します。

POSIX オプション:
  -a             ディレクトリだけでなく、すべてのファイルを表示
  -H             コマンドライン引数のシンボリックリンクを辿る
  -k             1024バイトブロック単位で表示（デフォルト）
  -L             すべてのシンボリックリンクを辿る
  -s             各引数の合計のみを表示
  -x             異なるファイルシステムのディレクトリをスキップ

GNU 拡張オプション:
  -0, --null            改行の代わりにNUL文字で終端
  -b, --bytes           バイト単位で実サイズを表示 (--apparent-size --block-size=1)
  -B, --block-size=SIZE SIZEバイトのブロック単位で表示
  -c, --total           総計を表示
  -d, --max-depth=N     ディレクトリ深さNまでの合計を表示（-s と同様に 0 なら概要のみ）
  -h, --human-readable  人間が読みやすい形式でサイズを表示
  -l, --count-links     ハードリンクがある場合は複数回カウント
  -P, --no-dereference  シンボリックリンクを辿らない（デフォルト）
  -S, --separate-dirs   ディレクトリ自体のサイズのみ（サブディレクトリを含めない）
      --si              -h と同様だが 1024 ではなく 1000 の累乗を使用
      --apparent-size   ディスク使用量ではなく実サイズを表示
  -t, --threshold=SIZE  SIZE より大きい（正の場合）または小さい（負の場合）
                        エントリのみを表示
      --exclude=PATTERN PATTERN に一致するファイルを除外
      --time            最終更新時刻を表示
      --help            このヘルプを表示して終了
      --version         バージョン情報を表示して終了

SIZE の形式:
  数値のみ   バイト
  K または KiB  キビバイト (1024バイト)
  M または MiB  メビバイト (1024²バイト)
  G または GiB  ギビバイト (1024³バイト)
  KB            キロバイト (1000バイト)
  MB            メガバイト (1000²バイト)
  GB            ギガバイト (1000³バイト)

表示するサイズは POSIX 準拠でデフォルト 512 バイトブロック。
環境変数 POSIXLY_CORRECT が設定されている場合は 512 バイト単位。
それ以外は 1024 バイト単位（-k 相当）で表示。

例:
  du                  カレントディレクトリ
  du -h               人間が読みやすい形式
  du -sh *            各項目のサマリー
  du -hs folder       結合オプション
  du -h --max-depth=1 深さ1まで
  du -hd1             深さ1まで（結合形式）
  du -ah              全ファイル表示
  du -c dir1 dir2     合計付き
  du -x /             ルートファイルシステムのみ"#
    );
}

/// ファイルサイズを計算
fn calculate_size(
    path: &Path,
    opts: &Options,
    depth: usize,
    seen_files: &mut HashSet<FileId>,
    root_device: Option<u64>,
    is_cmdline_arg: bool,
) -> io::Result<DirStats> {
    // メタデータを取得（シンボリックリンクの扱いを決定）
    let follow_link = if is_cmdline_arg {
        opts.dereference_args || opts.dereference
    } else {
        opts.dereference
    };

    let metadata = if follow_link {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }?;

    // 除外パターンチェック
    if let Some(name) = path.file_name() {
        let name_str = name.to_string_lossy();
        for pattern in &opts.exclude {
            if matches_pattern(&name_str, pattern) {
                return Ok(DirStats { size: 0 });
            }
        }
    }

    // -x オプション: 異なるファイルシステムをスキップ
    if opts.one_file_system {
        if let Some(root_dev) = root_device {
            if let Some(current_dev) = get_device_id_from_path(path) {
                if root_dev != current_dev {
                    return Ok(DirStats { size: 0 });
                }
            }
        }
    }

    // ハードリンクの重複カウント防止
    if !opts.count_links && !metadata.is_dir() {
        if let Some(file_id) = get_file_id_from_path(path) {
            if !seen_files.insert(file_id) {
                // 既に見たファイル
                return Ok(DirStats { size: 0 });
            }
        }
    }

    // ファイルの場合
    if metadata.is_file() || metadata.file_type().is_symlink() {
        let size = get_file_size(path, &metadata, opts);

        // ファイルを表示するかどうか
        let should_print = if is_cmdline_arg {
            // コマンドライン引数のファイルは常に表示（POSIX準拠）
            !opts.summarize || depth == 0
        } else {
            // -a オプションがある場合のみ
            opts.all && !opts.summarize
        };

        if should_print && should_show(size, opts) && within_depth(depth, opts) {
            print_size(size, &path.display().to_string(), opts);
        }

        return Ok(DirStats { size });
    }

    // ディレクトリでない場合（デバイスファイルなど）
    if !metadata.is_dir() {
        return Ok(DirStats { size: 0 });
    }

    // ディレクトリの処理
    let mut total_size: u64 = 0;

    // ディレクトリ自体のサイズ（通常は0かクラスタサイズ）
    // Windowsでは通常0
    let dir_self_size = get_file_size(path, &metadata, opts);
    total_size += dir_self_size;

    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("du: '{}' を読み込めません: {}", path.display(), format_error(&e));
            return Ok(DirStats { size: total_size });
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("du: ディレクトリエントリの読み取りエラー: {}", format_error(&e));
                continue;
            }
        };

        let entry_path = entry.path();

        // 除外パターンチェック
        if let Some(name) = entry_path.file_name() {
            let name_str = name.to_string_lossy();
            let mut excluded = false;
            for pattern in &opts.exclude {
                if matches_pattern(&name_str, pattern) {
                    excluded = true;
                    break;
                }
            }
            if excluded {
                continue;
            }
        }

        // シンボリックリンクの扱いを決定
        let entry_meta = if opts.dereference {
            fs::metadata(&entry_path)
        } else {
            fs::symlink_metadata(&entry_path)
        };

        match entry_meta {
            Ok(meta) => {
                if meta.is_dir() {
                    match calculate_size(&entry_path, opts, depth + 1, seen_files, root_device, false) {
                        Ok(stats) => {
                            if !opts.separate_dirs {
                                total_size += stats.size;
                            }
                        }
                        Err(e) => {
                            eprintln!("du: '{}': {}", entry_path.display(), format_error(&e));
                        }
                    }
                } else {
                    // ファイルまたはシンボリックリンク
                    // ハードリンクの重複チェック
                    if !opts.count_links {
                        if let Some(file_id) = get_file_id_from_path(&entry_path) {
                            if !seen_files.insert(file_id) {
                                continue; // 既に見たファイル
                            }
                        }
                    }

                    let file_size = get_file_size(&entry_path, &meta, opts);
                    total_size += file_size;

                    // -a オプションでファイルを表示
                    if opts.all && !opts.summarize && should_show(file_size, opts) && within_depth(depth + 1, opts) {
                        print_size(file_size, &entry_path.display().to_string(), opts);
                    }
                }
            }
            Err(e) => {
                eprintln!("du: '{}' にアクセスできません: {}", entry_path.display(), format_error(&e));
            }
        }
    }

    // ディレクトリ自体を表示
    let should_print_dir = if opts.summarize {
        depth == 0
    } else {
        within_depth(depth, opts)
    };

    if should_print_dir && should_show(total_size, opts) {
        print_size(total_size, &path.display().to_string(), opts);
    }

    Ok(DirStats { size: total_size })
}

/// 深さ制限内かどうか
fn within_depth(depth: usize, opts: &Options) -> bool {
    match opts.max_depth {
        Some(max) => depth <= max,
        None => true,
    }
}

/// ファイルサイズを取得
fn get_file_size(path: &Path, metadata: &Metadata, opts: &Options) -> u64 {
    if opts.apparent_size {
        metadata.len()
    } else {
        get_disk_size(path, metadata)
    }
}

/// ディスク上のサイズを取得
fn get_disk_size(path: &Path, metadata: &Metadata) -> u64 {
    // 圧縮ファイルサイズを取得（Windowsのみ）
    #[cfg(windows)]
    if let Some(size) = get_compressed_size(path) {
        return size;
    }

    let file_size = metadata.len();
    if file_size == 0 {
        return 0;
    }

    // クラスタサイズに切り上げ
    let cluster_size = get_cluster_size(path).unwrap_or(4096);
    ((file_size + cluster_size - 1) / cluster_size) * cluster_size
}

/// Windowsで圧縮ファイルサイズを取得
#[cfg(windows)]
fn get_compressed_size(path: &Path) -> Option<u64> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetCompressedFileSizeW;

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(once(0)).collect();

    unsafe {
        let mut high: u32 = 0;
        let low = GetCompressedFileSizeW(PCWSTR(wide_path.as_ptr()), Some(&mut high));

        if low == 0xFFFFFFFF {
            let error = windows::Win32::Foundation::GetLastError();
            if error.0 != 0 {
                return None;
            }
        }

        Some(((high as u64) << 32) | (low as u64))
    }
}

#[cfg(not(windows))]
fn get_compressed_size(_path: &Path) -> Option<u64> {
    None
}

/// クラスタサイズを取得
#[cfg(windows)]
fn get_cluster_size(path: &Path) -> Option<u64> {
    use std::iter::once;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceW;

    let root = get_root_path(path)?;
    let wide_root: Vec<u16> = root.encode_utf16().chain(once(0)).collect();

    unsafe {
        let mut sectors_per_cluster: u32 = 0;
        let mut bytes_per_sector: u32 = 0;
        let mut number_of_free_clusters: u32 = 0;
        let mut total_number_of_clusters: u32 = 0;

        let result = GetDiskFreeSpaceW(
            PCWSTR(wide_root.as_ptr()),
            Some(&mut sectors_per_cluster),
            Some(&mut bytes_per_sector),
            Some(&mut number_of_free_clusters),
            Some(&mut total_number_of_clusters),
        );

        if result.is_ok() {
            Some((sectors_per_cluster as u64) * (bytes_per_sector as u64))
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn get_cluster_size(_path: &Path) -> Option<u64> {
    None
}

/// ルートパスを取得
#[cfg(windows)]
fn get_root_path(path: &Path) -> Option<String> {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };

    let path_str = abs_path.to_string_lossy();

    if path_str.len() >= 2 && path_str.chars().nth(1) == Some(':') {
        Some(format!("{}:\\", path_str.chars().next()?))
    } else if path_str.starts_with("\\\\") {
        let parts: Vec<&str> = path_str.split('\\').collect();
        if parts.len() >= 4 {
            Some(format!("\\\\{}\\{}\\", parts[2], parts[3]))
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(not(windows))]
fn get_root_path(_path: &Path) -> Option<String> {
    Some("/".to_string())
}

/// ファイルIDを取得（ハードリンク検出用）
#[cfg(windows)]
fn get_file_id_from_path(path: &Path) -> Option<FileId> {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = File::open(path).ok()?;
    let handle = file.as_raw_handle();

    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let result = GetFileInformationByHandle(
            windows::Win32::Foundation::HANDLE(handle),
            &mut info,
        );

        if result.is_ok() {
            let file_index = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
            Some(FileId {
                volume_serial: info.dwVolumeSerialNumber,
                file_index,
            })
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn get_file_id_from_path(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        Some(FileId {
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    } else {
        None
    }
}

/// デバイスIDを取得
#[cfg(windows)]
fn get_device_id_from_path(path: &Path) -> Option<u64> {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = File::open(path).ok()?;
    let handle = file.as_raw_handle();

    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let result = GetFileInformationByHandle(
            windows::Win32::Foundation::HANDLE(handle),
            &mut info,
        );

        if result.is_ok() {
            Some(info.dwVolumeSerialNumber as u64)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn get_device_id_from_path(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        Some(metadata.dev())
    } else {
        None
    }
}

/// しきい値をチェック
fn should_show(size: u64, opts: &Options) -> bool {
    match opts.threshold {
        Some(t) if t > 0 => size >= t as u64,
        Some(t) if t < 0 => size <= (-t) as u64,
        _ => true,
    }
}

/// パターンマッチング（シェルグロブ簡易版）
fn matches_pattern(name: &str, pattern: &str) -> bool {
    if contains_glob_metachar(pattern) {
        glob_match(pattern, name)
    } else {
        names_equal(name, pattern)
    }
}

/// グロブメタ文字を含むかを返す
fn contains_glob_metachar(pattern: &str) -> bool {
    #[cfg(windows)]
    {
        pattern.contains(['*', '?', '['])
    }

    #[cfg(not(windows))]
    {
        let mut escaped = false;

        for ch in pattern.chars() {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '*' | '?' | '[' => return true,
                _ => {}
            }
        }

        false
    }
}

/// glob ライブラリ向けにパターンを正規化
fn normalize_glob_pattern(pattern: &str) -> String {
    #[cfg(windows)]
    {
        pattern.replace('\\', "/")
    }

    #[cfg(not(windows))]
    {
        pattern.to_string()
    }
}

/// Windows では POSIX シェル展開に寄せて大文字小文字を無視する
fn names_equal(lhs: &str, rhs: &str) -> bool {
    #[cfg(windows)]
    {
        lhs.eq_ignore_ascii_case(rhs)
    }

    #[cfg(not(windows))]
    {
        lhs == rhs
    }
}

/// glob crate を使ったグロブマッチング
fn glob_match(pattern: &str, text: &str) -> bool {
    let options = glob::MatchOptions {
        case_sensitive: !cfg!(windows),
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    let normalized_pattern = normalize_glob_pattern(pattern);
    let normalized_text = normalize_glob_pattern(text);

    match glob::Pattern::new(&normalized_pattern) {
        Ok(compiled) => compiled.matches_with(&normalized_text, options),
        Err(_) => names_equal(text, pattern),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bracket_glob_syntax() {
        assert!(contains_glob_metachar("file[0-9].txt"));
        assert!(!contains_glob_metachar("plain.txt"));
    }

    #[test]
    fn normalizes_windows_separators_for_globbing() {
        #[cfg(windows)]
        assert_eq!(normalize_glob_pattern(r"src\*.rs"), "src/*.rs");

        #[cfg(not(windows))]
        assert_eq!(normalize_glob_pattern(r"src\*.rs"), r"src\*.rs");
    }

    #[test]
    fn glob_match_supports_character_classes() {
        assert!(glob_match("file[0-9].txt", "file7.txt"));
        assert!(!glob_match("file[0-9].txt", "filex.txt"));
    }

    #[test]
    fn exact_match_follows_platform_case_rules() {
        #[cfg(windows)]
        assert!(matches_pattern("Cargo.toml", "cargo.toml"));

        #[cfg(not(windows))]
        assert!(!matches_pattern("Cargo.toml", "cargo.toml"));
    }
}

/// サイズを表示
fn print_size(size: u64, path: &str, opts: &Options) {
    let size_str = format_size(size, opts);
    let terminator = if opts.null { '\0' } else { '\n' };

    print!("{}\t{}{}", size_str, path, terminator);
}

/// サイズをフォーマット
fn format_size(bytes: u64, opts: &Options) -> String {
    // -b (--bytes) オプション
    if opts.bytes {
        return format!("{}", bytes);
    }

    // -h (--human-readable) オプション
    if opts.human_readable {
        let base = if opts.si { 1000u64 } else { 1024u64 };
        let units = if opts.si {
            ["B", "KB", "MB", "GB", "TB", "PB"]
        } else {
            ["", "K", "M", "G", "T", "P"]
        };

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
            format!("{:.2}{}", value, units[unit_index])
        }
    } else if let Some(block_size) = opts.block_size {
        // 明示的なブロックサイズ（切り捨て除算 - Linux du互換）
        format!("{}", bytes / block_size)
    } else if opts.block_size_k {
        // -k オプション（切り捨て除算 - Linux du互換）
        format!("{}", bytes / 1024)
    } else {
        // デフォルト: POSIXLY_CORRECT環境変数をチェック
        let block_size = if std::env::var("POSIXLY_CORRECT").is_ok() {
            512u64 // POSIX準拠: 512バイトブロック
        } else {
            1024u64 // GNU拡張デフォルト
        };
        // 切り捨て除算 - Linux du互換
        format!("{}", bytes / block_size)
    }
}

/// エラーメッセージをフォーマット
fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::InvalidInput => "無効な引数です".to_string(),
        _ => {
            // Windowsのエラーコードを確認
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "そのようなファイルやディレクトリはありません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    32 => "別のプロセスがファイルを使用中です".to_string(),
                    123 => "ファイル名、ディレクトリ名、またはボリューム ラベルの構文が間違っています".to_string(),
                    _ => format!("エラー (コード: {})", code),
                };
            }
            e.to_string()
        }
    }
}
