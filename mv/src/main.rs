// mv - ファイルを移動（名前変更）
// POSIX準拠 + GNU拡張

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use glob;

#[derive(Default)]
struct Options {
    // POSIX標準オプション
    force: bool,          // -f: 確認なし（デフォルト）
    interactive: bool,    // -i: 上書き前に確認
    
    // GNU拡張オプション
    no_clobber: bool,     // -n: 既存ファイルを上書きしない
    verbose: bool,        // -v: 移動内容を表示
    update: bool,         // -u: ソースが新しい場合のみ移動
    backup: BackupMode,   // -b, --backup: バックアップを作成
    backup_suffix: String,// -S, --suffix: バックアップサフィックス
    target_dir: Option<String>,  // -t: ターゲットディレクトリ
    no_target_dir: bool,  // -T: ターゲットをディレクトリとして扱わない
    strip_trailing_slashes: bool, // --strip-trailing-slashes
    
    show_help: bool,
    show_version: bool,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum BackupMode {
    #[default]
    None,
    Simple,      // 単純バックアップ (~)
    Numbered,    // 番号付き (.~1~)
    Existing,    // 既存に合わせる
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (opts, targets) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("mv: {}", e);
            eprintln!("詳細は 'mv --help' を参照してください");
            std::process::exit(1);
        }
    };
    
    if opts.show_help {
        print_help();
        std::process::exit(0);
    }
    
    if opts.show_version {
        println!("mv 1.1.0 (Rust Windows版 - POSIX準拠)");
        std::process::exit(0);
    }
    
    // glob展開（-t 指定時は全引数がソース、それ以外は最後の引数が移動先なので展開しない）
    let targets = expand_globs_for_mv(targets, opts.target_dir.is_some());
    
    if let Err(e) = run(&opts, &targets) {
        eprintln!("mv: {}", e);
        std::process::exit(1);
    }
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options {
        backup_suffix: env::var("SIMPLE_BACKUP_SUFFIX").unwrap_or_else(|_| "~".to_string()),
        ..Default::default()
    };
    let mut targets = Vec::new();
    let mut end_of_opts = false;
    let mut i = 1;
    
    while i < args.len() {
        let arg = &args[i];
        
        if end_of_opts {
            targets.push(arg.clone());
            i += 1;
            continue;
        }
        
        if arg == "--" {
            end_of_opts = true;
            i += 1;
            continue;
        }
        
        if arg.starts_with("--") {
            parse_long_option(arg, &args, &mut i, &mut opts)?;
            i += 1;
            continue;
        }
        
        if arg.starts_with('-') && arg.len() > 1 && arg != "-" {
            parse_short_options(arg, &args, &mut i, &mut opts)?;
            i += 1;
            continue;
        }
        
        targets.push(arg.clone());
        i += 1;
    }
    
    // オプションの優先順位を処理
    // POSIX: 最後に指定されたオプションが有効
    // -f が指定されたら -i, -n を無効化（parse時に処理済み）

    if opts.target_dir.is_some() && opts.no_target_dir {
        return Err("--target-directory (-t) と --no-target-directory (-T) は同時に指定できません".to_string());
    }

    Ok((opts, targets))
}

fn parse_long_option(arg: &str, args: &[String], i: &mut usize, opts: &mut Options) -> Result<(), String> {
    let opt = &arg[2..];
    let (name, value) = if let Some(pos) = opt.find('=') {
        (&opt[..pos], Some(&opt[pos + 1..]))
    } else {
        (opt, None)
    };
    
    match name {
        "force" => {
            opts.force = true;
            opts.interactive = false;
            opts.no_clobber = false;
        }
        "interactive" => {
            opts.interactive = true;
            opts.force = false;
        }
        "no-clobber" => {
            opts.no_clobber = true;
            opts.force = false;
        }
        "verbose" => opts.verbose = true,
        "update" => opts.update = true,
        "backup" => {
            opts.backup = match value {
                Some("simple") | Some("never") => BackupMode::Simple,
                Some("numbered") | Some("t") => BackupMode::Numbered,
                Some("existing") | Some("nil") => BackupMode::Existing,
                Some("none") | Some("off") => BackupMode::None,
                Some(v) => return Err(format!("'--backup' の引数が不正です: '{}'", v)),
                None => default_backup_mode(),
            };
        }
        "suffix" => {
            let val = value.ok_or("'--suffix' には引数が必要です")?;
            opts.backup_suffix = val.to_string();
            if opts.backup == BackupMode::None {
                opts.backup = default_backup_mode();
            }
        }
        "target-directory" => {
            let val = if let Some(v) = value {
                v.to_string()
            } else if *i + 1 < args.len() {
                *i += 1;
                args[*i].clone()
            } else {
                return Err("'--target-directory' には引数が必要です".to_string());
            };
            opts.target_dir = Some(val);
        }
        "no-target-directory" => opts.no_target_dir = true,
        "strip-trailing-slashes" => opts.strip_trailing_slashes = true,
        "help" => opts.show_help = true,
        "version" => opts.show_version = true,
        _ => return Err(format!("不明なオプション: '--{}'", name)),
    }
    
    Ok(())
}

fn parse_short_options(arg: &str, args: &[String], i: &mut usize, opts: &mut Options) -> Result<(), String> {
    let chars: Vec<char> = arg[1..].chars().collect();
    let mut j = 0;
    
    while j < chars.len() {
        match chars[j] {
            // POSIX標準
            'f' => {
                opts.force = true;
                opts.interactive = false;
                opts.no_clobber = false;
            }
            'i' => {
                opts.interactive = true;
                opts.force = false;
            }
            // GNU拡張
            'n' => {
                opts.no_clobber = true;
                opts.force = false;
            }
            'v' => opts.verbose = true,
            'u' => opts.update = true,
            'b' => {
                if opts.backup == BackupMode::None {
                    opts.backup = default_backup_mode();
                }
            }
            'T' => opts.no_target_dir = true,
            't' => {
                let rest: String = chars[j + 1..].iter().collect();
                if !rest.is_empty() {
                    opts.target_dir = Some(rest);
                    return Ok(());
                } else if *i + 1 < args.len() {
                    *i += 1;
                    opts.target_dir = Some(args[*i].clone());
                    return Ok(());
                } else {
                    return Err("'-t' には引数が必要です".to_string());
                }
            }
            'S' => {
                let rest: String = chars[j + 1..].iter().collect();
                if !rest.is_empty() {
                    opts.backup_suffix = rest;
                    if opts.backup == BackupMode::None {
                        opts.backup = default_backup_mode();
                    }
                    return Ok(());
                } else if *i + 1 < args.len() {
                    *i += 1;
                    opts.backup_suffix = args[*i].clone();
                    if opts.backup == BackupMode::None {
                        opts.backup = default_backup_mode();
                    }
                    return Ok(());
                } else {
                    return Err("'-S' には引数が必要です".to_string());
                }
            }
            _ => return Err(format!("不正なオプション: '-{}'", chars[j])),
        }
        j += 1;
    }
    
    Ok(())
}

/// Windows向けglob展開（mvコマンド用）
/// 最後の引数は移動先なので展開しない（-t 指定時は全引数がソースなので展開する）
fn expand_globs_for_mv(raw_targets: Vec<String>, expand_all: bool) -> Vec<String> {
    if !expand_all && raw_targets.len() <= 1 {
        return raw_targets;
    }

    let mut result = Vec::new();
    let last_idx = raw_targets.len().saturating_sub(1);

    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };

    for (idx, pattern) in raw_targets.into_iter().enumerate() {
        // 最後の引数（移動先）は展開しない
        if !expand_all && idx == last_idx {
            result.push(pattern);
            continue;
        }

        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
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

fn print_help() {
    println!(r#"使い方: mv [オプション]... [-T] ソース 移動先
     または: mv [オプション]... ソース... ディレクトリ
     または: mv [オプション]... -t ディレクトリ ソース...

ソースを移動先に名前変更、またはソースをディレクトリに移動します。

POSIX標準オプション:
  -f, --force       確認なしで上書き（-i, -n を上書き）
  -i, --interactive 上書き前に確認（-f, -n を上書き）

GNU拡張オプション:
  -n, --no-clobber  既存ファイルを上書きしない（-f, -i を上書き）
  -u, --update      ソースが新しい場合、または移動先が存在しない場合のみ移動
  -v, --verbose     実行内容を表示
  -b                --backup=existing と同様
      --backup[=CONTROL]
                    既存の移動先ファイルをバックアップ
  -S, --suffix=SUFFIX
                    通常のバックアップサフィックスを上書き
  -t, --target-directory=DIRECTORY
                    すべてのソース引数を DIRECTORY に移動
  -T, --no-target-directory
                    移動先を通常のファイルとして扱う
      --strip-trailing-slashes
                    ソース引数から末尾のスラッシュを削除
      --help        このヘルプを表示して終了
      --version     バージョン情報を表示して終了

バックアップサフィックスは '~' です。--suffix または SIMPLE_BACKUP_SUFFIX 
環境変数で変更可能。バージョン管理方法は --backup または VERSION_CONTROL 
環境変数で選択できます:

  none, off       バックアップを作成しない
  numbered, t     番号付きバックアップを作成
  existing, nil   番号付きバックアップがあれば番号付き、なければ単純
  simple, never   常に単純バックアップを作成

終了ステータス:
  0  正常終了
  1  エラー発生

例:
  mv file.txt newname.txt      ファイル名を変更
  mv file.txt dir/             ファイルをディレクトリに移動
  mv -i *.txt backup/          確認付きで移動
  mv -v olddir newdir          ディレクトリ名を変更（詳細表示）
  mv -b file.txt existing.txt  バックアップを作成して上書き"#);
}

fn run(opts: &Options, targets: &[String]) -> io::Result<()> {
    // -t オプションがある場合
    if let Some(ref dir) = opts.target_dir {
        if targets.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "オペランドがありません",
            ));
        }
        let dest = PathBuf::from(dir);
        if !dest.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("'{}' の移動先: ディレクトリではありません", dest.display()),
            ));
        }
        
        let mut has_error = false;
        for src in targets {
            let src_path = strip_slashes(src, opts);
            if let Err(e) = move_to_directory(&src_path, &dest, opts) {
                eprintln!("mv: {}", format_error_with_path(&src_path, &e));
                has_error = true;
            }
        }
        
        if has_error {
            return Err(io::Error::new(io::ErrorKind::Other, "一部のファイルの移動に失敗"));
        }
        return Ok(());
    }
    
    match targets.len() {
        0 => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "オペランドがありません",
            ));
        }
        1 => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{}' の後にオペランドがありません", targets[0]),
            ));
        }
        2 => {
            let src = strip_slashes(&targets[0], opts);
            let dest = Path::new(&targets[1]);

            // 大文字小文字のみのリネームは move_item で直接処理
            // （dest.is_dir() が true でも自分自身の中へ移動しない）
            if dest.is_dir() && !opts.no_target_dir && !is_case_only_rename(&src, dest) {
                move_to_directory(&src, dest, opts)?;
            } else {
                move_item(&src, dest, opts)?;
            }
        }
        _ => {
            if opts.no_target_dir {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("余分なオペランド '{}'", targets[2]),
                ));
            }

            // 複数ソース -> 最後がディレクトリ
            let dest = Path::new(targets.last().unwrap());

            if !dest.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("'{}' の移動先: ディレクトリではありません", dest.display()),
                ));
            }
            
            let mut has_error = false;
            for src in &targets[..targets.len() - 1] {
                let src_path = strip_slashes(src, opts);
                if let Err(e) = move_to_directory(&src_path, dest, opts) {
                    eprintln!("mv: {}", format_error_with_path(&src_path, &e));
                    has_error = true;
                }
            }
            
            if has_error {
                return Err(io::Error::new(io::ErrorKind::Other, "一部のファイルの移動に失敗"));
            }
        }
    }
    
    Ok(())
}

fn strip_slashes(path: &str, opts: &Options) -> PathBuf {
    if opts.strip_trailing_slashes {
        PathBuf::from(path.trim_end_matches(|c| c == '/' || c == '\\'))
    } else {
        PathBuf::from(path)
    }
}

fn move_to_directory(src: &Path, dest_dir: &Path, opts: &Options) -> io::Result<()> {
    let file_name = src.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "無効なファイル名です")
    })?;
    
    let dest = dest_dir.join(file_name);
    move_item(src, &dest, opts)
}

fn move_item(src: &Path, dest: &Path, opts: &Options) -> io::Result<()> {
    // ソース存在チェック
    if !src.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("'{}' を stat できません: そのようなファイルやディレクトリはありません", src.display()),
        ));
    }
    
    // 同一ファイルチェック（正規化して比較）
    let src_canonical = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
    let dest_exists = dest.exists();
    let dest_canonical = if dest_exists {
        fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf())
    } else {
        dest.to_path_buf()
    };

    if src_canonical == dest_canonical {
        // Windowsの大文字小文字を区別しないFSでは、大文字小文字のみの
        // リネームも「同じファイル」になる。この場合はリネームを実行する
        if is_case_only_rename(src, dest) {
            return match fs::rename(src, dest) {
                Ok(()) => {
                    print_verbose(src, dest, None, opts);
                    Ok(())
                }
                Err(e) => Err(io::Error::new(
                    e.kind(),
                    format!("'{}' を '{}' に移動できません: {}", src.display(), dest.display(), format_error(&e)),
                )),
            };
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("'{}' と '{}' は同じファイルです", src.display(), dest.display()),
        ));
    }

    // 自分自身のサブディレクトリへの移動チェック
    if src.is_dir() {
        let dest_full = dest.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .and_then(|p| fs::canonicalize(p).ok())
            .map(|p| p.join(dest.file_name().unwrap_or_default()))
            .unwrap_or_else(|| dest_canonical.clone());
        if is_within(&src_canonical, &dest_full) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{}' を自分自身のサブディレクトリ '{}' に移動できません", src.display(), dest.display()),
            ));
        }
    }

    // 移動先が存在する場合の処理
    let mut backup_path: Option<PathBuf> = None;
    if dest_exists {
        // -n: 上書きしない
        if opts.no_clobber {
            // 静かにスキップ（POSIX準拠）
            return Ok(());
        }

        // -u: 更新チェック
        if opts.update {
            if !is_newer(src, dest)? {
                return Ok(());
            }
        }

        // -i: 確認
        if opts.interactive && !opts.force {
            if !confirm(&format!("mv: '{}' を上書きしますか?", dest.display()))? {
                return Ok(());
            }
        }

        // ディレクトリと非ディレクトリの上書きは不可（GNU準拠）
        let src_is_dir = src.is_dir();
        let dest_is_dir = dest.is_dir();
        if dest_is_dir && !src_is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("ディレクトリ '{}' を非ディレクトリで上書きできません", dest.display()),
            ));
        }
        if !dest_is_dir && src_is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("非ディレクトリ '{}' をディレクトリで上書きできません", dest.display()),
            ));
        }

        // バックアップ
        if opts.backup != BackupMode::None {
            let backup = create_backup_path(dest, opts);
            if let Err(e) = fs::rename(dest, &backup) {
                return Err(io::Error::new(
                    e.kind(),
                    format!("'{}' のバックアップを作成できません: {}", dest.display(), format_error(&e)),
                ));
            }
            backup_path = Some(backup);
        } else if dest_is_dir {
            // 上書きできるのは空ディレクトリのみ（GNU準拠、非空なら失敗）
            remove_readonly(dest)?;
            fs::remove_dir(dest).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("'{}' を '{}' に移動できません: {}", src.display(), dest.display(), format_error(&e)),
                )
            })?;
        } else {
            // 既存ファイルは rename で置き換える（読み取り専用属性のみ解除）
            remove_readonly(dest)?;
        }
    }

    // 移動実行
    match fs::rename(src, dest) {
        Ok(()) => {
            print_verbose(src, dest, backup_path.as_deref(), opts);
            Ok(())
        }
        Err(e) => {
            // 異なるドライブ間の移動はコピー＆削除
            #[cfg(windows)]
            let is_cross_device = e.raw_os_error() == Some(17);
            #[cfg(not(windows))]
            let is_cross_device = e.kind() == io::ErrorKind::CrossesDevices;
            
            if is_cross_device || e.to_string().contains("cross-device") {
                move_across_drives(src, dest, backup_path.as_deref(), opts)
            } else {
                Err(io::Error::new(
                    e.kind(),
                    format!("'{}' を '{}' に移動できません: {}", src.display(), dest.display(), format_error(&e)),
                ))
            }
        }
    }
}

fn move_across_drives(src: &Path, dest: &Path, backup: Option<&Path>, opts: &Options) -> io::Result<()> {
    if src.is_dir() {
        copy_dir_recursive(src, dest)?;
        fs::remove_dir_all(src).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("'{}' を削除できません: {}", src.display(), format_error(&e)),
            )
        })?;
    } else {
        fs::copy(src, dest).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("'{}' を '{}' にコピーできません: {}", src.display(), dest.display(), format_error(&e)),
            )
        })?;
        preserve_times(src, dest);
        remove_readonly(src)?;
        fs::remove_file(src).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("'{}' を削除できません: {}", src.display(), format_error(&e)),
            )
        })?;
    }

    print_verbose(src, dest, backup, opts);

    Ok(())
}

/// コピー後にタイムスタンプを移行する（ベストエフォート）
fn preserve_times(src: &Path, dest: &Path) {
    let Ok(meta) = src.metadata() else { return };
    let mut times = fs::FileTimes::new();
    if let Ok(m) = meta.modified() {
        times = times.set_modified(m);
    }
    if let Ok(a) = meta.accessed() {
        times = times.set_accessed(a);
    }
    if let Ok(f) = fs::OpenOptions::new().write(true).open(dest) {
        let _ = f.set_times(times);
    }
}

fn print_verbose(src: &Path, dest: &Path, backup: Option<&Path>, opts: &Options) {
    if !opts.verbose {
        return;
    }
    match backup {
        Some(b) => println!("'{}' -> '{}' (バックアップ: '{}')", src.display(), dest.display(), b.display()),
        None => println!("'{}' -> '{}'", src.display(), dest.display()),
    }
}

/// 同一ファイルを指すが、最終要素の大文字小文字だけが異なるか
/// （Windowsの大文字小文字を区別しないFSでのリネーム用）
fn is_case_only_rename(src: &Path, dest: &Path) -> bool {
    if !dest.exists() {
        return false;
    }
    let (Ok(a), Ok(b)) = (fs::canonicalize(src), fs::canonicalize(dest)) else {
        return false;
    };
    if a != b {
        return false;
    }
    match (src.file_name(), dest.file_name()) {
        (Some(x), Some(y)) => {
            x != y && x.to_string_lossy().to_lowercase() == y.to_string_lossy().to_lowercase()
        }
        _ => false,
    }
}

/// path が dir の内部（サブディレクトリ以下）にあるか
fn is_within(dir: &Path, path: &Path) -> bool {
    let d = dir.to_string_lossy().to_lowercase();
    let p = path.to_string_lossy().to_lowercase();
    p.len() > d.len()
        && p.starts_with(&d)
        && matches!(p.as_bytes()[d.len()], b'/' | b'\\')
}

/// VERSION_CONTROL 環境変数からデフォルトのバックアップモードを決定
fn default_backup_mode() -> BackupMode {
    match env::var("VERSION_CONTROL").ok().as_deref() {
        Some("none") | Some("off") => BackupMode::None,
        Some("simple") | Some("never") => BackupMode::Simple,
        Some("numbered") | Some("t") => BackupMode::Numbered,
        _ => BackupMode::Existing,
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    
    let mut had_error = false;
    
    for entry_result in fs::read_dir(src)? {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("mv: '{}' 内のエントリを読み取れません: {}", src.display(), format_error(&e));
                had_error = true;
                continue;
            }
        };
        
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("mv: '{}' のメタデータを取得できません: {}", src_path.display(), format_error(&e));
                had_error = true;
                continue;
            }
        };
        
        if metadata.is_dir() {
            if let Err(e) = copy_dir_recursive(&src_path, &dest_path) {
                eprintln!("mv: {}", e);
                had_error = true;
            }
        } else {
            match fs::copy(&src_path, &dest_path) {
                Ok(_) => preserve_times(&src_path, &dest_path),
                Err(e) => {
                    eprintln!("mv: '{}' を '{}' にコピーできません: {}", src_path.display(), dest_path.display(), format_error(&e));
                    had_error = true;
                }
            }
        }
    }
    
    if had_error {
        Err(io::Error::new(io::ErrorKind::Other, "一部のファイルのコピーに失敗しました"))
    } else {
        Ok(())
    }
}

fn create_backup_path(path: &Path, opts: &Options) -> PathBuf {
    match opts.backup {
        BackupMode::None => path.to_path_buf(),
        BackupMode::Simple => {
            PathBuf::from(format!("{}{}", path.display(), opts.backup_suffix))
        }
        BackupMode::Numbered => {
            let mut counter = 1;
            loop {
                let backup = PathBuf::from(format!("{}.~{}~", path.display(), counter));
                if !backup.exists() {
                    return backup;
                }
                counter += 1;
            }
        }
        BackupMode::Existing => {
            // 既存の番号付きバックアップがあるかチェック
            let numbered_pattern = format!("{}.~1~", path.display());
            if Path::new(&numbered_pattern).exists() || has_numbered_backup(path) {
                // 番号付きバックアップを使用
                let mut counter = 1;
                loop {
                    let backup = PathBuf::from(format!("{}.~{}~", path.display(), counter));
                    if !backup.exists() {
                        return backup;
                    }
                    counter += 1;
                }
            } else {
                // 単純バックアップを使用
                PathBuf::from(format!("{}{}", path.display(), opts.backup_suffix))
            }
        }
    }
}

fn has_numbered_backup(path: &Path) -> bool {
    if let Some(parent) = path.parent() {
        if let Some(name) = path.file_name() {
            let prefix = format!("{}.~", name.to_string_lossy());
            if let Ok(entries) = fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let entry_name = entry.file_name().to_string_lossy().to_string();
                    if entry_name.starts_with(&prefix) && entry_name.ends_with('~') {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn is_newer(src: &Path, dest: &Path) -> io::Result<bool> {
    let src_time = src.metadata()?.modified()?;
    let dest_time = dest.metadata()?.modified()?;
    Ok(src_time > dest_time)
}

#[cfg(windows)]
fn remove_readonly(path: &Path) -> io::Result<()> {
    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
    
    if let Ok(metadata) = path.metadata() {
        let attrs = metadata.file_attributes();
        if attrs & FILE_ATTRIBUTE_READONLY != 0 {
            let mut perms = metadata.permissions();
            perms.set_readonly(false);
            fs::set_permissions(path, perms)?;
        }
    }
    
    Ok(())
}

#[cfg(not(windows))]
fn remove_readonly(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn confirm(message: &str) -> io::Result<bool> {
    eprint!("{} ", message);
    io::stderr().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    
    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}

fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::AlreadyExists => "ファイルが存在します".to_string(),
        io::ErrorKind::NotADirectory => "ディレクトリではありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        io::ErrorKind::DirectoryNotEmpty => "ディレクトリは空ではありません".to_string(),
        _ => {
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "そのようなファイルやディレクトリはありません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    17 => "ファイルを別のディスク ドライブに移動できません".to_string(),
                    32 => "別のプロセスがファイルを使用中です".to_string(),
                    123 => "ファイル名、ディレクトリ名、またはボリュームラベルの構文が間違っています".to_string(),
                    145 => "ディレクトリは空ではありません".to_string(),
                    _ => format!("{} (エラーコード: {})", e, code),
                };
            }
            e.to_string()
        }
    }
}

fn format_error_with_path(path: &Path, e: &io::Error) -> String {
    format!("'{}': {}", path.display(), format_error(e))
}
