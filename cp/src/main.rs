use std::env;
use std::fs::{self, File, FileTimes, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use glob;

#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file, FileTimesExt, MetadataExt};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// シンボリックリンクの扱い
#[derive(Clone, Copy, PartialEq)]
enum SymlinkMode {
    /// -P: シンボリックリンクをたどらない（-R 時のデフォルト）
    NoFollow,
    /// -H: コマンドライン引数のシンボリックリンクのみたどる
    FollowCommandLine,
    /// -L: すべてのシンボリックリンクをたどる（非 -R 時のデフォルト）
    FollowAll,
}

/// バックアップの種類（GNU準拠）
#[derive(Default, Clone, Copy, PartialEq)]
enum BackupMode {
    #[default]
    None,
    Simple,      // 単純バックアップ (~)
    Numbered,    // 番号付き (.~1~)
    Existing,    // 既存に合わせる
}

#[derive(Clone, Copy, Default)]
enum OverwriteMode {
    #[default]
    Default,
    Force,
    Interactive,
    NoClobber,
}

enum DestinationAction {
    Continue,
    Skip,
}

#[cfg(windows)]
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[cfg(windows)]
type Handle = *mut std::ffi::c_void;

#[cfg(windows)]
#[repr(C)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: u64,
    last_access_time: u64,
    last_write_time: u64,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        h_file: Handle,
        file_information: *mut ByHandleFileInformation,
    ) -> i32;
}

#[derive(Default)]
struct Options {
    // POSIX オプション
    recursive: bool,          // -R: ディレクトリを再帰的にコピー
    preserve: bool,           // -p: 属性を保持
    symlink_mode: Option<SymlinkMode>, // -H, -L, -P: シンボリックリンクの扱い（未指定時は -R なら -P、それ以外は -L）
    overwrite_mode: OverwriteMode, // -f, -i, -n: 上書き制御

    // GNU拡張オプション
    archive: bool,            // -a: アーカイブモード (-pPR と同等)
    verbose: bool,            // -v: コピー内容を表示
    update: bool,             // -u: 更新されたファイルのみコピー
    backup: BackupMode,       // -b, --backup: バックアップを作成
    backup_suffix: String,    // -S, --suffix: バックアップサフィックス
    target_dir: Option<String>,  // -t: ターゲットディレクトリ
    no_target_dir: bool,      // -T: ターゲットをディレクトリとして扱わない

    show_help: bool,
    show_version: bool,
}

impl Options {
    /// 有効なシンボリックリンクモードを解決する
    /// POSIX/GNU: -H/-L/-P 未指定時、-R ありなら -P、なしなら -L 相当
    fn effective_symlink_mode(&self) -> SymlinkMode {
        self.symlink_mode.unwrap_or(if self.recursive {
            SymlinkMode::NoFollow
        } else {
            SymlinkMode::FollowAll
        })
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (opts, targets) = match parse_args(&args) {
        Ok(value) => value,
        Err(message) => {
            eprintln!("cp: {}", message);
            std::process::exit(1);
        }
    };
    
    if opts.show_help {
        print_help();
        std::process::exit(0);
    }
    
    if opts.show_version {
        println!("cp 1.1.0 (Rust実装)");
        std::process::exit(0);
    }
    
    let mut exit_code = 0;
    
    if let Err(e) = run(&opts, &targets, &mut exit_code) {
        eprintln!("cp: {}", e);
        exit_code = 1;
    }
    
    std::process::exit(exit_code);
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options {
        backup_suffix: env::var("SIMPLE_BACKUP_SUFFIX").unwrap_or_else(|_| "~".to_string()),
        ..Default::default()
    };
    let mut targets = Vec::new();
    let mut end_of_opts = false;
    let mut skip_next = false;
    
    for (i, arg) in args.iter().skip(1).enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        
        if end_of_opts || !arg.starts_with('-') || arg == "-" {
            targets.push(arg.clone());
            continue;
        }
        
        match arg.as_str() {
            "--" => end_of_opts = true,
            "--recursive" => opts.recursive = true,
            "--force" => opts.overwrite_mode = OverwriteMode::Force,
            "--interactive" => opts.overwrite_mode = OverwriteMode::Interactive,
            "--no-clobber" => opts.overwrite_mode = OverwriteMode::NoClobber,
            "--verbose" => opts.verbose = true,
            "--preserve" => opts.preserve = true,
            "--archive" => {
                opts.archive = true;
                opts.symlink_mode = Some(SymlinkMode::NoFollow);
            }
            "--update" => opts.update = true,
            "--backup" => opts.backup = default_backup_mode(),
            "--no-target-directory" => opts.no_target_dir = true,
            "--no-dereference" => opts.symlink_mode = Some(SymlinkMode::NoFollow),
            "--dereference" => opts.symlink_mode = Some(SymlinkMode::FollowAll),
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            s if s.starts_with("--target-directory=") => {
                opts.target_dir = Some(s.trim_start_matches("--target-directory=").to_string());
            }
            s if s.starts_with("--backup=") => {
                opts.backup = match s.trim_start_matches("--backup=") {
                    "simple" | "never" => BackupMode::Simple,
                    "numbered" | "t" => BackupMode::Numbered,
                    "existing" | "nil" => BackupMode::Existing,
                    "none" | "off" => BackupMode::None,
                    v => return Err(format!("'--backup' の引数が不正です: '{}'", v)),
                };
            }
            s if s.starts_with("--suffix=") => {
                opts.backup_suffix = s.trim_start_matches("--suffix=").to_string();
                if opts.backup == BackupMode::None {
                    opts.backup = default_backup_mode();
                }
            }
            s if s.starts_with("--") => {
                return Err(format!("不明なオプション '{}'", s));
            }
            s => {
                let chars: Vec<char> = s.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        // POSIX オプション
                        'R' => opts.recursive = true,
                        'f' => opts.overwrite_mode = OverwriteMode::Force,
                        'i' => opts.overwrite_mode = OverwriteMode::Interactive,
                        'p' => opts.preserve = true,
                        'H' => opts.symlink_mode = Some(SymlinkMode::FollowCommandLine),
                        'L' => opts.symlink_mode = Some(SymlinkMode::FollowAll),
                        'P' => opts.symlink_mode = Some(SymlinkMode::NoFollow),
                        // GNU拡張
                        'r' => opts.recursive = true,  // -r は -R と同じ
                        'a' => {
                            opts.archive = true;
                            opts.symlink_mode = Some(SymlinkMode::NoFollow);
                        }
                        'd' => opts.symlink_mode = Some(SymlinkMode::NoFollow),
                        'n' => opts.overwrite_mode = OverwriteMode::NoClobber,
                        'v' => opts.verbose = true,
                        'u' => opts.update = true,
                        'b' => {
                            if opts.backup == BackupMode::None {
                                opts.backup = default_backup_mode();
                            }
                        }
                        'S' => {
                            // -S SUFFIX 形式
                            if j + 1 < chars.len() {
                                opts.backup_suffix = chars[j + 1..].iter().collect();
                            } else if let Some(next) = args.get(i + 2) {
                                opts.backup_suffix = next.clone();
                                skip_next = true;
                            } else {
                                return Err("オプション '-S' には引数が必要です".to_string());
                            }
                            if opts.backup == BackupMode::None {
                                opts.backup = default_backup_mode();
                            }
                            if j + 1 < chars.len() {
                                break;
                            }
                        }
                        'T' => opts.no_target_dir = true,
                        't' => {
                            // -t DIR 形式
                            if j + 1 < chars.len() {
                                opts.target_dir = Some(chars[j+1..].iter().collect());
                                break;
                            } else if let Some(next) = args.get(i + 2) {
                                opts.target_dir = Some(next.clone());
                                skip_next = true;
                            } else {
                                return Err("オプション '-t' には引数が必要です".to_string());
                            }
                        }
                        'h' => opts.show_help = true,
                        _ => return Err(format!("不明なオプション '-{}'", chars[j])),
                    }
                    j += 1;
                }
            }
        }
    }
    
    // -a は -pPR と同等（-P は指定時点で設定済み。後続の -L/-H が優先される）
    if opts.archive {
        opts.preserve = true;
        opts.recursive = true;
    }

    if opts.target_dir.is_some() && opts.no_target_dir {
        return Err("--target-directory (-t) と --no-target-directory (-T) は同時に指定できません".to_string());
    }

    // Windows ではシェルが glob 展開しないため、cp 側でオペランドを展開する。
    // 「最後の引数だけコピー先」とは決め打ちせず、展開後のオペランド列に対して
    // 通常の cp ルールを適用することで POSIX 系シェルの見え方に近づける。
    let targets = expand_globs_for_cp(targets);
    
    Ok((opts, targets))
}

/// Windows向けglob展開（cpコマンド用）
/// POSIX シェルと同様に、各オペランドを個別に展開する。
fn expand_globs_for_cp(raw_targets: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();

    // Windowsでは大文字小文字を区別しない
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };

    for pattern in raw_targets {
        // ワイルドカードや文字クラスを含む場合は glob 展開
        if has_glob_meta(&pattern) {
            match glob::glob_with(&pattern, options) {
                Ok(paths) => {
                    let mut matches = Vec::new();
                    for entry in paths {
                        if let Ok(path) = entry {
                            matches.push(path);
                        }
                    }

                    if matches.is_empty() {
                        // マッチなしの場合は元のパターンをそのまま（エラー表示用）
                        result.push(pattern);
                    } else {
                        sort_globbed_paths(&mut matches);
                        result.extend(
                            matches
                                .into_iter()
                                .map(|path: PathBuf| path.to_string_lossy().to_string()),
                        );
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

fn has_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

#[cfg(windows)]
fn sort_globbed_paths(paths: &mut [PathBuf]) {
    paths.sort_by_cached_key(|path| path.to_string_lossy().to_ascii_lowercase());
}

#[cfg(not(windows))]
fn sort_globbed_paths(paths: &mut [PathBuf]) {
    paths.sort_by_cached_key(|path| path.to_string_lossy().into_owned());
}

fn print_help() {
    println!(r#"使い方: cp [オプション] ソース... ターゲット
       cp [オプション] -t ディレクトリ ソース...

ファイルやディレクトリをコピーします。

POSIXオプション:
  -R                  ディレクトリを再帰的にコピー
  -f                  確認なしで強制上書き（書き込み保護を解除）
  -i                  上書き前に確認
  -p                  属性（タイムスタンプ、パーミッション）を保持
  -H                  コマンドライン引数のシンボリックリンクをたどる
  -L                  すべてのシンボリックリンクをたどる（非 -R 時のデフォルト）
  -P                  シンボリックリンクをたどらない（-R 時のデフォルト）

GNU拡張オプション:
  -a, --archive       アーカイブモード（-pPR と同等）
  -r                  -R と同じ
  -d                  シンボリックリンクをリンクとしてコピー（-P と同様）
  -n, --no-clobber    既存ファイルを上書きしない
  -u, --update        ソースが新しい場合のみコピー
  -v, --verbose       コピー内容を表示
  -b                  上書き前にバックアップを作成
      --backup[=CONTROL]
                      バックアップ方法を指定
                        none, off       作成しない
                        numbered, t     番号付き (.~1~)
                        existing, nil   番号付きがあれば番号付き、なければ単純
                        simple, never   常に単純 (~)
  -S, --suffix=SUFFIX バックアップサフィックスを指定（デフォルト ~、
                      SIMPLE_BACKUP_SUFFIX 環境変数でも変更可能）
  -t, --target-directory=DIR
                      ターゲットディレクトリを指定
  -T, --no-target-directory
                      ターゲットをディレクトリとして扱わない
      --help          このヘルプを表示
      --version       バージョンを表示

バックアップ方法は --backup または VERSION_CONTROL 環境変数で選択できます。

例:
  cp file.txt backup.txt       ファイルをコピー
  cp file1.txt file2.txt dir/  複数ファイルをディレクトリへ
  cp -R srcdir dstdir          ディレクトリを再帰コピー
  cp -i *.txt backup/          確認付きでコピー
  cp -a src dst                属性を保持して再帰コピー

Windows補足:
  -P 指定時はシンボリックリンクに加えてジャンクションなどの
  ディレクトリ再解析ポイントもリンクとして複製します"#);
}

fn run(opts: &Options, targets: &[String], exit_code: &mut i32) -> io::Result<()> {
    // -t オプションがある場合
    if let Some(ref dir) = opts.target_dir {
        if targets.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ソースファイルが指定されていません",
            ));
        }
        let dest = PathBuf::from(dir);
        if !dest.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("'{}': ディレクトリではありません", dest.display()),
            ));
        }
        for src in targets {
            if let Err(e) = copy_to_directory(Path::new(src), &dest, opts, true, exit_code) {
                eprintln!("cp: {}", e);
                *exit_code = 1;
            }
        }
        return Ok(());
    }
    
    match targets.len() {
        0 => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ソースファイルが指定されていません",
            ));
        }
        1 => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{}' の後にコピー先を指定してください", targets[0]),
            ));
        }
        2 => {
            let src = Path::new(&targets[0]);
            let dest = Path::new(&targets[1]);
            
            if dest.is_dir() && !opts.no_target_dir {
                copy_to_directory(src, dest, opts, true, exit_code)?;
            } else {
                copy_item(src, dest, opts, true, exit_code)?;
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
                    format!("'{}': ディレクトリではありません", dest.display()),
                ));
            }
            
            for src in &targets[..targets.len() - 1] {
                if let Err(e) = copy_to_directory(Path::new(src), dest, opts, true, exit_code) {
                    eprintln!("cp: {}", e);
                    *exit_code = 1;
                }
            }
        }
    }
    
    Ok(())
}

fn copy_to_directory(src: &Path, dest_dir: &Path, opts: &Options, is_command_line: bool, exit_code: &mut i32) -> io::Result<()> {
    let file_name = src.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, 
            format!("'{}': 無効なファイル名です", src.display()))
    })?;
    
    let dest = dest_dir.join(file_name);
    copy_item(src, &dest, opts, is_command_line, exit_code)
}

fn copy_item(src: &Path, dest: &Path, opts: &Options, is_command_line: bool, exit_code: &mut i32) -> io::Result<()> {
    // シンボリックリンクの処理を決定
    let follow_symlink = match opts.effective_symlink_mode() {
        SymlinkMode::NoFollow => false,
        SymlinkMode::FollowCommandLine => is_command_line,
        SymlinkMode::FollowAll => true,
    };
    
    // メタデータ取得（シンボリックリンクをたどるかどうか）
    let metadata = if follow_symlink {
        fs::metadata(src)
    } else {
        fs::symlink_metadata(src)
    };
    
    let metadata = metadata.map_err(|e| {
        io::Error::new(e.kind(), 
            format!("'{}': {}", src.display(), format_error(&e)))
    })?;
    
    // 自己参照チェック
    if let Err(e) = check_same_file(src, dest) {
        if e.kind() == io::ErrorKind::InvalidInput {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{}' と '{}' は同じファイルです", src.display(), dest.display()),
            ));
        }
    }
    
    // シンボリックリンクの場合
    if metadata.file_type().is_symlink() && !follow_symlink {
        return copy_symlink(src, dest, opts);
    }

    #[cfg(windows)]
    if !follow_symlink && is_windows_directory_link(&metadata) {
        return copy_symlink(src, dest, opts);
    }
    
    if metadata.is_dir() {
        if !opts.recursive {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{}': -R を指定しないとディレクトリを省略します", src.display()),
            ));
        }
        // 非ディレクトリをディレクトリで上書きは不可（GNU準拠）
        if dest.exists() && !dest.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("非ディレクトリ '{}' をディレクトリ '{}' で上書きできません", dest.display(), src.display()),
            ));
        }
        copy_directory(src, dest, opts, exit_code)
    } else {
        // ディレクトリを非ディレクトリで上書きは不可（GNU準拠）
        if dest.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("ディレクトリ '{}' を非ディレクトリで上書きできません", dest.display()),
            ));
        }
        copy_file(src, dest, &metadata, opts)
    }
}

/// 同一ファイルかチェック
#[cfg(windows)]
fn check_same_file(src: &Path, dest: &Path) -> io::Result<()> {
    let src_info = get_file_identity(src)?;
    let dest_info = get_file_identity(dest)?;

    if src_info.volume_serial_number == dest_info.volume_serial_number
        && src_info.file_index_high == dest_info.file_index_high
        && src_info.file_index_low == dest_info.file_index_low
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "同じファイルです"));
    }

    Ok(())
}

#[cfg(windows)]
fn get_file_identity(path: &Path) -> io::Result<ByHandleFileInformation> {
    let file = File::open(path)?;
    let mut info = ByHandleFileInformation {
        file_attributes: 0,
        creation_time: 0,
        last_access_time: 0,
        last_write_time: 0,
        volume_serial_number: 0,
        file_size_high: 0,
        file_size_low: 0,
        number_of_links: 0,
        file_index_high: 0,
        file_index_low: 0,
    };

    // SAFETY: `file` is an open handle for the duration of this call and `info`
    // points to writable memory with the Win32 layout expected by the API.
    let result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as Handle, &mut info as *mut _) };

    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(info)
}

#[cfg(windows)]
fn format_windows_symlink_error(src: &Path, error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::PermissionDenied {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "'{}': シンボリックリンクを作成できません。管理者権限または開発者モードが必要な場合があります",
                src.display()
            ),
        ))
    } else {
        Err(error)
    }
}

/// 同一ファイルかチェック
#[cfg(not(windows))]
fn check_same_file(src: &Path, dest: &Path) -> io::Result<()> {
    let src_canonical = fs::canonicalize(src).ok();
    let dest_canonical = fs::canonicalize(dest).ok();
    
    if let (Some(s), Some(d)) = (src_canonical, dest_canonical) {
        if s == d {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "同じファイルです"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(src: &Path, dest: &Path, opts: &Options) -> io::Result<()> {
    use std::os::unix::fs as unix_fs;

    if matches!(
        prepare_destination_for_overwrite(src, dest, opts)?,
        DestinationAction::Skip
    ) {
        return Ok(());
    }
    
    let link_target = fs::read_link(src)?;
    unix_fs::symlink(&link_target, dest)?;
    
    if opts.verbose {
        println!("'{}' -> '{}' (シンボリックリンク)", src.display(), dest.display());
    }
    
    Ok(())
}

#[cfg(windows)]
fn copy_symlink(src: &Path, dest: &Path, opts: &Options) -> io::Result<()> {
    if matches!(
        prepare_destination_for_overwrite(src, dest, opts)?,
        DestinationAction::Skip
    ) {
        return Ok(());
    }

    let link_target = fs::read_link(src)?;
    let target_is_dir = determine_windows_symlink_target_is_dir(src, &link_target)?;

    let result = if target_is_dir {
        symlink_dir(&link_target, dest)
    } else {
        symlink_file(&link_target, dest)
    };

    if let Err(e) = result {
        return format_windows_symlink_error(src, e);
    }

    if opts.verbose {
        println!("'{}' -> '{}' (シンボリックリンク)", src.display(), dest.display());
    }

    Ok(())
}

fn prepare_destination_for_overwrite(
    _src: &Path,
    dest: &Path,
    opts: &Options,
) -> io::Result<DestinationAction> {
    if !dest.exists() && dest.symlink_metadata().is_err() {
        return Ok(DestinationAction::Continue);
    }

    if matches!(opts.overwrite_mode, OverwriteMode::NoClobber) {
        if opts.verbose {
            eprintln!("スキップ: '{}' (既存)", dest.display());
        }
        return Ok(DestinationAction::Skip);
    }

    if matches!(opts.overwrite_mode, OverwriteMode::Interactive)
        && !confirm(&format!("'{}' を上書きしますか?", dest.display()))?
    {
        return Ok(DestinationAction::Skip);
    }

    if opts.backup != BackupMode::None {
        create_backup(dest, opts)?;
        return Ok(DestinationAction::Continue);
    }

    if matches!(opts.overwrite_mode, OverwriteMode::Force) {
        remove_readonly(dest)?;
    }

    remove_path(dest)?;
    Ok(DestinationAction::Continue)
}

fn remove_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir(path)
            } else {
                fs::remove_file(path)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
fn determine_windows_symlink_target_is_dir(src: &Path, link_target: &Path) -> io::Result<bool> {
    if let Ok(metadata) = fs::symlink_metadata(src) {
        if metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Ok(true);
        }
    }

    match fs::metadata(src) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let candidate = if link_target.is_absolute() {
                link_target.to_path_buf()
            } else {
                src.parent().unwrap_or_else(|| Path::new(".")).join(link_target)
            };

            fs::metadata(candidate)
                .map(|metadata| metadata.is_dir())
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "'{}': 実体の種別を判定できないシンボリックリンクは Windows で複製できません",
                            src.display()
                        ),
                    )
                })
        }
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
fn is_windows_directory_link(metadata: &Metadata) -> bool {
    let attrs = metadata.file_attributes();
    attrs & FILE_ATTRIBUTE_DIRECTORY != 0
        && attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && !metadata.file_type().is_symlink()
}

fn copy_file(src: &Path, dest: &Path, src_metadata: &Metadata, opts: &Options) -> io::Result<()> {
    // 既存ファイルチェック
    if dest.exists() {
        // -n: 上書きしない
        if matches!(opts.overwrite_mode, OverwriteMode::NoClobber) {
            if opts.verbose {
                eprintln!("スキップ: '{}' (既存)", dest.display());
            }
            return Ok(());
        }
        
        // -u: 更新チェック
        if opts.update {
            if !is_newer(src, dest)? {
                if opts.verbose {
                    eprintln!("スキップ: '{}' (最新)", dest.display());
                }
                return Ok(());
            }
        }
        
        // -i: 確認（-f が指定されていなければ）
        if matches!(opts.overwrite_mode, OverwriteMode::Interactive) {
            if !confirm(&format!("'{}' を上書きしますか?", dest.display()))? {
                return Ok(());
            }
        }
        
        // -b: バックアップ
        if opts.backup != BackupMode::None {
            create_backup(dest, opts)?;
        }
        
        // -f: 読み取り専用を解除して削除
        if matches!(opts.overwrite_mode, OverwriteMode::Force) {
            remove_readonly(dest)?;
            // 上書きできない場合は削除を試みる
            if let Err(_) = fs::metadata(dest).and_then(|m| {
                if m.permissions().readonly() {
                    fs::remove_file(dest)
                } else {
                    Ok(())
                }
            }) {
                let _ = fs::remove_file(dest);
            }
        }
    }
    
    // 親ディレクトリを作成
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    
    // コピー実行
    copy_file_contents(src, dest)?;
    
    // -p: 属性保持
    if opts.preserve {
        preserve_attributes(src, dest, src_metadata)?;
    }
    
    if opts.verbose {
        println!("'{}' -> '{}'", src.display(), dest.display());
    }
    
    Ok(())
}

/// ファイル内容をコピー
fn copy_file_contents(src: &Path, dest: &Path) -> io::Result<u64> {
    let mut src_file = File::open(src).map_err(|e| {
        io::Error::new(e.kind(),
            format!("'{}' を開けません: {}", src.display(), format_error(&e)))
    })?;
    let mut dest_file = File::create(dest).map_err(|e| {
        io::Error::new(e.kind(),
            format!("通常ファイル '{}' を作成できません: {}", dest.display(), format_error(&e)))
    })?;

    io::copy(&mut src_file, &mut dest_file)
}

fn copy_directory(src: &Path, dest: &Path, opts: &Options, exit_code: &mut i32) -> io::Result<()> {
    // 自己参照チェック（サブディレクトリへのコピー防止）
    // dest が未作成でも、存在する祖先まで正規化して判定する
    // （ここを通すと dest 作成後の read_dir が dest 自身を拾い、無限再帰になる）
    let src_canonical = fs::canonicalize(src)?;
    let dest_full = resolve_full_path(dest);
    if is_within(&src_canonical, &dest_full) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("'{}' を自身のサブディレクトリ '{}' にコピーできません",
                src.display(), dest.display()),
        ));
    }

    // ディレクトリ作成
    if !dest.exists() {
        fs::create_dir_all(dest)?;
        if opts.verbose {
            println!("ディレクトリ作成: '{}'", dest.display());
        }
    }
    
    // ディレクトリ内容をコピー
    let entries = fs::read_dir(src)?;
    
    for entry_result in entries {
        // エントリの読み取りエラーを適切に処理
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("cp: '{}' 内のエントリを読み取れません: {}", src.display(), format_error(&e));
                *exit_code = 1;
                continue;
            }
        };
        
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        
        if let Err(e) = copy_item(&src_path, &dest_path, opts, false, exit_code) {
            eprintln!("cp: {}", e);
            *exit_code = 1;
            // POSIX: エラーがあっても続行
        }
    }
    
    // -p: ディレクトリの属性も保持
    if opts.preserve {
        let src_metadata = src.metadata()?;
        preserve_attributes(src, dest, &src_metadata)?;
    }
    
    Ok(())
}

/// 存在しないパスでも、存在する祖先まで正規化した完全パスを返す
fn resolve_full_path(path: &Path) -> PathBuf {
    let mut cur = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = fs::canonicalize(&cur) {
            let mut result = canonical;
            for name in tail.iter().rev() {
                result.push(name);
            }
            return result;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = if parent.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    parent.to_path_buf()
                };
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// path が dir 自身またはその内部にあるか（Windowsは大文字小文字を区別しない）
fn is_within(dir: &Path, path: &Path) -> bool {
    let d = dir.to_string_lossy().to_lowercase();
    let p = path.to_string_lossy().to_lowercase();
    p == d
        || (p.len() > d.len()
            && p.starts_with(&d)
            && matches!(p.as_bytes()[d.len()], b'/' | b'\\'))
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

fn create_backup(path: &Path, opts: &Options) -> io::Result<()> {
    let backup = create_backup_path(path, opts);
    fs::rename(path, &backup)?;
    if opts.verbose {
        println!("バックアップ: '{}' -> '{}'", path.display(), backup.display());
    }
    Ok(())
}

fn create_backup_path(path: &Path, opts: &Options) -> PathBuf {
    match opts.backup {
        BackupMode::None | BackupMode::Simple => {
            PathBuf::from(format!("{}{}", path.display(), opts.backup_suffix))
        }
        BackupMode::Numbered => next_numbered_backup(path),
        BackupMode::Existing => {
            if has_numbered_backup(path) {
                next_numbered_backup(path)
            } else {
                PathBuf::from(format!("{}{}", path.display(), opts.backup_suffix))
            }
        }
    }
}

fn next_numbered_backup(path: &Path) -> PathBuf {
    let mut counter = 1;
    loop {
        let backup = PathBuf::from(format!("{}.~{}~", path.display(), counter));
        if !backup.exists() {
            return backup;
        }
        counter += 1;
    }
}

fn has_numbered_backup(path: &Path) -> bool {
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
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
    false
}

fn is_newer(src: &Path, dest: &Path) -> io::Result<bool> {
    let src_time = src.metadata()?.modified()?;
    let dest_time = dest.metadata()?.modified()?;
    Ok(src_time > dest_time)
}

fn preserve_attributes(_src: &Path, dest: &Path, src_metadata: &Metadata) -> io::Result<()> {
    preserve_times(dest, src_metadata)?;
    
    // パーミッション
    preserve_permissions(dest, src_metadata)?;
    
    Ok(())
}

#[cfg(windows)]
fn preserve_permissions(dest: &Path, src_metadata: &Metadata) -> io::Result<()> {
    let perms = src_metadata.permissions();
    match fs::set_permissions(dest, perms) {
        Ok(()) => Ok(()),
        Err(_) if src_metadata.is_dir() => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
fn preserve_permissions(dest: &Path, src_metadata: &Metadata) -> io::Result<()> {
    let perms = src_metadata.permissions();
    fs::set_permissions(dest, perms)
}

#[cfg(windows)]
fn preserve_times(dest: &Path, src_metadata: &Metadata) -> io::Result<()> {
    if src_metadata.is_dir() {
        return Ok(());
    }

    let mut times = FileTimes::new();

    if let Ok(accessed) = src_metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    if let Ok(modified) = src_metadata.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(created) = src_metadata.created() {
        times = times.set_created(created);
    }

    let file = File::options().write(true).open(dest)?;
    file.set_times(times)
}

#[cfg(not(windows))]
fn preserve_times(dest: &Path, src_metadata: &Metadata) -> io::Result<()> {
    if let Ok(modified) = src_metadata.modified() {
        if let Err(e) = set_file_mtime(dest, modified) {
            if !src_metadata.is_dir() {
                return Err(e);
            }
        }
    }

    Ok(())
}

#[cfg(windows)]
#[cfg_attr(not(test), allow(dead_code))]
fn set_file_mtime(path: &Path, mtime: std::time::SystemTime) -> io::Result<()> {
    let file = File::options().write(true).open(path)?;
    let times = FileTimes::new().set_modified(mtime);
    file.set_times(times)
}

#[cfg(unix)]
fn set_file_mtime(path: &Path, mtime: std::time::SystemTime) -> io::Result<()> {
    let file = File::options().write(true).open(path)?;
    let times = FileTimes::new().set_modified(mtime);
    file.set_times(times)
}

#[cfg(not(any(windows, unix)))]
fn set_file_mtime(_path: &Path, _mtime: std::time::SystemTime) -> io::Result<()> {
    Ok(())
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
    eprint!("{} [y/N]: ", message);
    io::stderr().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    
    let input = input.trim().to_lowercase();
    Ok(matches!(input.as_str(), "y" | "yes" | "はい" | "h"))
}

/// io::Errorを日本語メッセージに変換
fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "そのようなファイルやディレクトリはありません".to_string(),
        io::ErrorKind::PermissionDenied => "許可がありません".to_string(),
        io::ErrorKind::AlreadyExists => "既に存在します".to_string(),
        io::ErrorKind::NotADirectory => "ディレクトリではありません".to_string(),
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        io::ErrorKind::DirectoryNotEmpty => "ディレクトリが空ではありません".to_string(),
        io::ErrorKind::ReadOnlyFilesystem => "読み取り専用ファイルシステムです".to_string(),
        io::ErrorKind::InvalidInput => e.to_string(),
        _ => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_same_file, expand_globs_for_cp, parse_args, preserve_attributes, run,
        set_file_mtime, OverwriteMode,
    };
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("cp-glob-test-{unique}"));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_cp(args: Vec<String>) -> io::Result<i32> {
        let (opts, targets) = parse_args(&args).unwrap();
        let mut exit_code = 0;
        run(&opts, &targets, &mut exit_code)?;
        Ok(exit_code)
    }

    #[test]
    fn expands_all_operands_including_last_one() {
        let dir = TestDir::new();
        fs::write(dir.path.join("a.txt"), b"a").unwrap();
        fs::write(dir.path.join("b.txt"), b"b").unwrap();
        fs::create_dir(dir.path.join("dest")).unwrap();

        let expanded = expand_globs_for_cp(vec![
            dir.path.join("*.txt").to_string_lossy().to_string(),
            dir.path.join("d*").to_string_lossy().to_string(),
        ]);

        assert_eq!(
            expanded,
            vec![
                dir.path.join("a.txt").to_string_lossy().to_string(),
                dir.path.join("b.txt").to_string_lossy().to_string(),
                dir.path.join("dest").to_string_lossy().to_string(),
            ]
        );
    }

    #[test]
    fn keeps_unmatched_pattern_for_error_reporting() {
        let dir = TestDir::new();
        let pattern = dir.path.join("missing*.txt").to_string_lossy().to_string();
        let dest = dir.path.join("dest").to_string_lossy().to_string();

        let expanded = expand_globs_for_cp(vec![pattern.clone(), dest.clone()]);

        assert_eq!(expanded, vec![pattern, dest]);
    }

    #[test]
    fn expands_character_classes() {
        let dir = TestDir::new();
        fs::write(dir.path.join("a.txt"), b"a").unwrap();
        fs::write(dir.path.join("b.txt"), b"b").unwrap();
        let dest = dir.path.join("dest").to_string_lossy().to_string();

        let expanded = expand_globs_for_cp(vec![
            dir.path.join("[ab].txt").to_string_lossy().to_string(),
            dest.clone(),
        ]);

        assert_eq!(
            expanded,
            vec![
                dir.path.join("a.txt").to_string_lossy().to_string(),
                dir.path.join("b.txt").to_string_lossy().to_string(),
                dest,
            ]
        );
    }

    #[test]
    fn last_overwrite_option_wins() {
        let args = vec![
            "cp".to_string(),
            "-fin".to_string(),
            "src".to_string(),
            "dst".to_string(),
        ];

        let (opts, targets) = parse_args(&args).unwrap();

        assert!(matches!(opts.overwrite_mode, OverwriteMode::NoClobber));
        assert_eq!(targets, vec!["src".to_string(), "dst".to_string()]);
    }

    #[test]
    fn long_overwrite_option_overrides_previous_short_option() {
        let args = vec![
            "cp".to_string(),
            "-f".to_string(),
            "--interactive".to_string(),
            "src".to_string(),
            "dst".to_string(),
        ];

        let (opts, _) = parse_args(&args).unwrap();

        assert!(matches!(opts.overwrite_mode, OverwriteMode::Interactive));
    }

    #[test]
    fn unknown_option_is_reported_as_error() {
        let args = vec!["cp".to_string(), "--unknown".to_string()];

        let err = parse_args(&args).err().unwrap();

        assert_eq!(err, "不明なオプション '--unknown'");
    }

    #[test]
    fn target_directory_option_requires_argument() {
        let args = vec!["cp".to_string(), "-t".to_string()];

        let err = parse_args(&args).err().unwrap();

        assert_eq!(err, "オプション '-t' には引数が必要です");
    }

    #[test]
    fn set_file_mtime_updates_file_timestamp() {
        let dir = TestDir::new();
        let path = dir.path.join("timestamp.txt");
        fs::write(&path, b"mtime").unwrap();
        let target_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        set_file_mtime(&path, target_time).unwrap();

        let actual = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(actual, target_time);
    }

    #[test]
    fn preserve_attributes_allows_directory_targets() {
        let dir = TestDir::new();
        let src = dir.path.join("srcdir");
        let dest = dir.path.join("destdir");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&dest).unwrap();
        let metadata = fs::metadata(&src).unwrap();

        preserve_attributes(&src, &dest, &metadata).unwrap();
    }

    #[test]
    fn check_same_file_detects_hard_link() {
        let dir = TestDir::new();
        let src = dir.path.join("original.txt");
        let alias = dir.path.join("alias.txt");
        fs::write(&src, b"same").unwrap();
        fs::hard_link(&src, &alias).unwrap();

        let err = check_same_file(&src, &alias).err().unwrap();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "同じファイルです");
    }

    #[test]
    fn run_copies_globbed_sources_into_directory() {
        let dir = TestDir::new();
        let src1 = dir.path.join("a.txt");
        let src2 = dir.path.join("b.txt");
        let dest = dir.path.join("dest");
        fs::write(&src1, b"a").unwrap();
        fs::write(&src2, b"b").unwrap();
        fs::create_dir(&dest).unwrap();

        let exit_code = run_cp(vec![
            "cp".to_string(),
            dir.path.join("*.txt").to_string_lossy().to_string(),
            dest.to_string_lossy().to_string(),
        ])
        .unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"b");
    }

    #[test]
    fn run_no_clobber_keeps_existing_destination() {
        let dir = TestDir::new();
        let src = dir.path.join("source.txt");
        let dest = dir.path.join("dest.txt");
        fs::write(&src, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();

        let exit_code = run_cp(vec![
            "cp".to_string(),
            "-n".to_string(),
            src.to_string_lossy().to_string(),
            dest.to_string_lossy().to_string(),
        ])
        .unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(fs::read(&dest).unwrap(), b"old");
    }

    #[test]
    fn run_preserve_keeps_modified_time_for_file() {
        let dir = TestDir::new();
        let src = dir.path.join("source.txt");
        let dest = dir.path.join("dest.txt");
        let target_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_123);
        fs::write(&src, b"content").unwrap();
        set_file_mtime(&src, target_time).unwrap();

        let exit_code = run_cp(vec![
            "cp".to_string(),
            "-p".to_string(),
            src.to_string_lossy().to_string(),
            dest.to_string_lossy().to_string(),
        ])
        .unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(fs::metadata(&dest).unwrap().modified().unwrap(), target_time);
    }

    #[cfg(windows)]
    #[test]
    fn windows_symlink_permission_error_is_localized() {
        let err = super::format_windows_symlink_error(
            PathBuf::from("source-link").as_path(),
            io::Error::new(io::ErrorKind::PermissionDenied, "Access is denied."),
        )
        .err()
        .unwrap();

        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("管理者権限または開発者モード"));
    }
}
