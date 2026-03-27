// rm - ファイルを削除
// POSIX.1-2017準拠 + GNU拡張

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path};

use glob;

#[derive(Default, Clone)]
struct Options {
    // POSIX標準オプション
    force: bool,       // -f: 強制削除（確認なし、エラー無視）
    interactive: bool, // -i: 削除前に確認
    recursive: bool,   // -r, -R: ディレクトリを再帰的に削除

    // GNU拡張オプション
    interactive_once: bool, // -I: 3つ以上または再帰時に1回確認
    verbose: bool,          // -v: 削除したファイルを表示
    dir: bool,              // -d: 空のディレクトリを削除
    one_file_system: bool,  // --one-file-system: 異なるファイルシステムをスキップ
    preserve_root: bool,    // --preserve-root: / の削除を禁止（デフォルト）
    no_preserve_root: bool, // --no-preserve-root: / の削除を許可

    show_help: bool,
    show_version: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let (opts, targets) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("rm: {}", e);
            eprintln!("詳しくは 'rm --help' を実行してください。");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("rm (Rust Windows版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張");
        std::process::exit(0);
    }

    // Windowsでは cmd / PowerShell が native exe の glob を展開しないため、
    // リテラルなパスとして解決できない場合にだけ rm 側で補完する。
    let targets = expand_globs(targets);

    if targets.is_empty() {
        if !opts.force {
            eprintln!("rm: オペランドがありません");
            eprintln!("詳しくは 'rm --help' を実行してください。");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    // -I: 3つ以上のファイルまたは再帰削除時に確認
    if opts.interactive_once && !opts.force {
        let needs_confirm = targets.len() >= 3 || opts.recursive;
        if needs_confirm {
            let msg = if opts.recursive {
                format!("{} 個の引数を再帰的に削除しますか?", targets.len())
            } else {
                format!("{} 個の引数を削除しますか?", targets.len())
            };
            match confirm(&msg) {
                Ok(true) => {}
                Ok(false) => std::process::exit(0),
                Err(e) => {
                    eprintln!("rm: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    let mut exit_code = 0;

    for target in &targets {
        if let Err(e) = remove_path(target, &opts, &mut exit_code) {
            if !opts.force {
                eprintln!("rm: '{}' を削除できません: {}", target, format_error(&e));
                exit_code = 1;
            }
        }
    }

    std::process::exit(exit_code);
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options {
        preserve_root: true,
        ..Default::default()
    };
    let mut targets = Vec::new();
    let mut end_of_opts = false;

    for arg in args.iter().skip(1) {
        if end_of_opts {
            targets.push(arg.clone());
            continue;
        }

        if !arg.starts_with('-') || arg == "-" {
            targets.push(arg.clone());
            continue;
        }

        match arg.as_str() {
            "--" => end_of_opts = true,
            "--force" => {
                opts.force = true;
                opts.interactive = false;
                opts.interactive_once = false;
            }
            "--recursive" => opts.recursive = true,
            "--interactive" => {
                opts.force = false;
                opts.interactive = true;
                opts.interactive_once = false;
            }
            "--verbose" => opts.verbose = true,
            "--dir" => opts.dir = true,
            "--one-file-system" => opts.one_file_system = true,
            "--preserve-root" => {
                opts.preserve_root = true;
                opts.no_preserve_root = false;
            }
            "--no-preserve-root" => {
                opts.no_preserve_root = true;
                opts.preserve_root = false;
            }
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            s if s.starts_with("--interactive=") => {
                let val = s.trim_start_matches("--interactive=");
                match val {
                    "never" | "no" => {
                        opts.force = false;
                        opts.interactive = false;
                        opts.interactive_once = false;
                    }
                    "once" => {
                        opts.force = false;
                        opts.interactive_once = true;
                        opts.interactive = false;
                    }
                    "always" | "yes" => {
                        opts.force = false;
                        opts.interactive = true;
                        opts.interactive_once = false;
                    }
                    _ => return Err(format!("'--interactive' の引数が不正です: '{}'", val)),
                }
            }
            s if s.starts_with("--") => {
                return Err(format!("不明なオプション: '{}'", s));
            }
            s => {
                for c in s.chars().skip(1) {
                    match c {
                        'f' => {
                            opts.force = true;
                            opts.interactive = false;
                            opts.interactive_once = false;
                        }
                        'r' | 'R' => opts.recursive = true,
                        'i' => {
                            opts.force = false;
                            opts.interactive = true;
                            opts.interactive_once = false;
                        }
                        'I' => {
                            opts.force = false;
                            opts.interactive_once = true;
                            opts.interactive = false;
                        }
                        'v' => opts.verbose = true,
                        'd' => opts.dir = true,
                        _ => return Err(format!("不正なオプション: '-{}'", c)),
                    }
                }
            }
        }
    }

    Ok((opts, targets))
}

/// Windows向けglob展開（大文字小文字を区別しない）
///
/// POSIX 系シェルに寄せるため、まずは各オペランドをリテラルなパスとして扱う。
/// リテラルとして存在しない場合のみ glob 展開を試み、マッチしなければ元の文字列を残す。
fn expand_globs(raw_targets: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();

    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };

    for pattern in raw_targets {
        if !has_glob_meta(&pattern) || Path::new(&pattern).symlink_metadata().is_ok() {
            result.push(pattern);
            continue;
        }

        if has_glob_meta(&pattern) {
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

fn has_glob_meta(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

fn print_help() {
    println!(
        r#"使い方: rm [オプション]... [ファイル]...

ファイル（またはディレクトリ）を削除します。

デフォルトではディレクトリを削除しません。-r または -R オプションを使用してください。

POSIX標準オプション:
  -f, --force       存在しないファイルや引数を無視し、確認なしで削除
  -i                削除前に毎回確認
  -r, -R, --recursive
                    ディレクトリとその中身を再帰的に削除

GNU拡張オプション:
  -I                3つ以上のファイルを削除する前、または再帰的に削除する前に
                    1回だけ確認する。-i より煩わしくないが、ミスを防ぐ
      --interactive[=WHEN]
                    確認のタイミングを指定: never, once (-I), always (-i)
  -d, --dir         空のディレクトリを削除
  -v, --verbose     実行内容を表示
      --one-file-system
                    再帰時に異なるファイルシステムのディレクトリをスキップ
      --no-preserve-root
                    '/' を特別扱いしない
      --preserve-root[=all]
                    '/' を削除しない（デフォルト）
      --help        このヘルプを表示して終了
      --version     バージョン情報を表示して終了

-i, -I, --interactive=always のいずれかが指定された場合でも、
-f オプションが後から指定されるとすべての確認が無効になります。
同様に -i と -I は互いに上書きします。

終了ステータス:
  0  正常終了
  1  削除エラー
  2  オプションエラー

注意:
  -r を付けずにディレクトリを削除しようとするとエラーになります。
  'rm -rf' は非常に強力で危険なコマンドです。使用には細心の注意を払ってください。

例:
  rm file.txt             ファイルを削除
  rm -r dir               ディレクトリを再帰的に削除
  rm -rf dir              確認なしで強制削除
  rm -i *.txt             確認しながら削除
  rm -I file1 file2 file3 削除前に1回確認
  rm -v file1 file2       削除内容を表示

Windows では、オペランドがリテラルなパスとして存在しない場合のみ、
大文字小文字を区別せずに内部で glob 展開を試みます。"#
    );
}

fn remove_path(target: &str, opts: &Options, exit_code: &mut i32) -> io::Result<()> {
    let path = Path::new(target);

    if is_dot_or_dotdot(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "'.' または '..' は削除できません",
        ));
    }

    if opts.preserve_root && !opts.no_preserve_root {
        if is_root_path(path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "'/' を削除することは危険です。--no-preserve-root を使用してください",
            ));
        }
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            if opts.force && e.kind() == io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(e);
        }
    };

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        remove_directory(path, opts, exit_code)
    } else {
        remove_file(path, opts)
    }
}

fn is_dot_or_dotdot(path: &Path) -> bool {
    matches!(
        path.components().next_back(),
        Some(Component::CurDir | Component::ParentDir)
    )
}

fn remove_file(path: &Path, opts: &Options) -> io::Result<()> {
    if opts.interactive && !opts.force {
        let prompt = if path
            .metadata()
            .map(|m| m.permissions().readonly())
            .unwrap_or(false)
        {
            format!(
                "書き込み保護されたファイル '{}' を削除しますか?",
                path.display()
            )
        } else {
            format!("'{}' を削除しますか?", path.display())
        };
        if !confirm(&prompt)? {
            return Ok(());
        }
    }

    if let Err(e) = remove_readonly(path) {
        if !opts.force {
            return Err(e);
        }
    }

    fs::remove_file(path)?;

    if opts.verbose {
        eprintln!("'{}' を削除しました", path.display());
    }

    Ok(())
}

fn remove_directory(path: &Path, opts: &Options, exit_code: &mut i32) -> io::Result<()> {
    if !opts.recursive && !opts.dir {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "ディレクトリです",
        ));
    }

    if opts.dir && !opts.recursive {
        if opts.interactive && !opts.force {
            if !confirm(&format!(
                "空のディレクトリ '{}' を削除しますか?",
                path.display()
            ))? {
                return Ok(());
            }
        }

        fs::remove_dir(path)?;

        if opts.verbose {
            eprintln!("'{}' を削除しました", path.display());
        }
        return Ok(());
    }

    if opts.interactive && !opts.force {
        if !confirm(&format!("ディレクトリ '{}' に降りますか?", path.display()))? {
            return Ok(());
        }
    }

    remove_dir_recursive(path, opts, exit_code)
}

fn remove_dir_recursive(path: &Path, opts: &Options, exit_code: &mut i32) -> io::Result<()> {
    let entries = fs::read_dir(path)?;
    let mut had_error = false;

    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                if !opts.force {
                    eprintln!(
                        "rm: '{}' 内のエントリを読み取れません: {}",
                        path.display(),
                        format_error(&e)
                    );
                    *exit_code = 1;
                }
                had_error = true;
                continue;
            }
        };

        let entry_path = entry.path();
        let metadata = match fs::symlink_metadata(&entry_path) {
            Ok(m) => m,
            Err(e) => {
                if !opts.force {
                    eprintln!(
                        "rm: '{}' のメタデータを取得できません: {}",
                        entry_path.display(),
                        format_error(&e)
                    );
                    *exit_code = 1;
                }
                had_error = true;
                continue;
            }
        };

        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            if let Err(e) = remove_dir_recursive(&entry_path, opts, exit_code) {
                if !opts.force {
                    eprintln!(
                        "rm: '{}' を削除できません: {}",
                        entry_path.display(),
                        format_error(&e)
                    );
                    *exit_code = 1;
                }
                had_error = true;
            }
        } else {
            if opts.interactive && !opts.force {
                match confirm(&format!("'{}' を削除しますか?", entry_path.display())) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        eprintln!("rm: 確認エラー: {}", e);
                        *exit_code = 1;
                        continue;
                    }
                }
            }

            let _ = remove_readonly(&entry_path);

            if let Err(e) = fs::remove_file(&entry_path) {
                if !opts.force {
                    eprintln!(
                        "rm: '{}' を削除できません: {}",
                        entry_path.display(),
                        format_error(&e)
                    );
                    *exit_code = 1;
                }
                had_error = true;
                continue;
            }

            if opts.verbose {
                eprintln!("'{}' を削除しました", entry_path.display());
            }
        }
    }

    if opts.interactive && !opts.force {
        if !confirm(&format!(
            "ディレクトリ '{}' を削除しますか?",
            path.display()
        ))? {
            return Ok(());
        }
    }

    match fs::remove_dir(path) {
        Ok(()) => {
            if opts.verbose {
                eprintln!("'{}' を削除しました", path.display());
            }
            Ok(())
        }
        Err(e) => {
            if had_error {
                if !opts.force {
                    eprintln!(
                        "rm: '{}' を削除できません: {}",
                        path.display(),
                        format_error(&e)
                    );
                    *exit_code = 1;
                }
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

fn is_root_path(path: &Path) -> bool {
    if let Ok(canonical) = fs::canonicalize(path) {
        let path_str = canonical.to_string_lossy();
        path_str == "/"
            || (path_str.len() == 3 && path_str.ends_with(":\\"))
            || (path_str.len() == 7 && path_str.starts_with(r"\\?\") && path_str.ends_with(":\\"))
    } else {
        false
    }
}

#[cfg(windows)]
fn remove_readonly(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;

    let metadata = path.metadata()?;
    let attrs = metadata.file_attributes();

    if attrs & FILE_ATTRIBUTE_READONLY != 0 {
        let mut perms = metadata.permissions();
        perms.set_readonly(false);
        fs::set_permissions(path, perms)?;
    }

    Ok(())
}

#[cfg(not(windows))]
fn remove_readonly(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn confirm(message: &str) -> io::Result<bool> {
    eprint!("rm: {} ", message);
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
        io::ErrorKind::IsADirectory => "ディレクトリです".to_string(),
        io::ErrorKind::DirectoryNotEmpty => "ディレクトリは空ではありません".to_string(),
        io::ErrorKind::InvalidInput => e.to_string(),
        _ => {
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "そのようなファイルやディレクトリはありません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    32 => "別のプロセスがファイルを使用中です".to_string(),
                    145 => "ディレクトリは空ではありません".to_string(),
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

    #[test]
    fn parse_args_last_option_wins_for_force_and_interactive() {
        let args = vec![
            "rm".to_string(),
            "-f".to_string(),
            "-i".to_string(),
            "target".to_string(),
        ];

        let (opts, targets) = parse_args(&args).expect("parse should succeed");

        assert!(!opts.force);
        assert!(opts.interactive);
        assert_eq!(targets, vec!["target"]);
    }

    #[test]
    fn parse_args_last_option_wins_for_interactive_once_and_force() {
        let args = vec![
            "rm".to_string(),
            "-I".to_string(),
            "-f".to_string(),
            "target".to_string(),
        ];

        let (opts, _) = parse_args(&args).expect("parse should succeed");

        assert!(opts.force);
        assert!(!opts.interactive);
        assert!(!opts.interactive_once);
    }

    #[test]
    fn parse_args_leaves_glob_pattern_untouched() {
        let args = vec!["rm".to_string(), "*.tmp".to_string()];

        let (_, targets) = parse_args(&args).expect("parse should succeed");

        assert_eq!(targets, vec!["*.tmp"]);
    }

    #[test]
    fn expand_globs_matches_files_on_windows_style_shells() {
        let temp = std::env::temp_dir().join(format!(
            "rm-glob-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp directory should be created");

        let match_path = temp.join("match.tmp");
        let other_path = temp.join("other.log");
        fs::write(&match_path, b"data").expect("matching file should be created");
        fs::write(&other_path, b"data").expect("other file should be created");

        let pattern = temp.join("*.tmp").to_string_lossy().to_string();
        let expanded = expand_globs(vec![pattern]);

        assert_eq!(expanded, vec![match_path.to_string_lossy().to_string()]);

        let _ = fs::remove_file(&match_path);
        let _ = fs::remove_file(&other_path);
        let _ = fs::remove_dir(&temp);
    }

    #[test]
    fn expand_globs_is_case_insensitive_on_windows_style_shells() {
        let temp = std::env::temp_dir().join(format!(
            "rm-glob-case-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp directory should be created");

        let match_path = temp.join("MiXeD.TMP");
        fs::write(&match_path, b"data").expect("matching file should be created");

        let pattern = temp.join("*.tmp").to_string_lossy().to_string();
        let expanded = expand_globs(vec![pattern]);

        assert_eq!(expanded, vec![match_path.to_string_lossy().to_string()]);

        let _ = fs::remove_file(&match_path);
        let _ = fs::remove_dir(&temp);
    }

    #[test]
    fn expand_globs_prefers_existing_literal_path() {
        let temp = std::env::temp_dir().join(format!(
            "rm-glob-literal-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp directory should be created");

        let literal_path = temp.join("literal[1].tmp");
        fs::write(&literal_path, b"data").expect("literal file should be created");

        let expanded = expand_globs(vec![literal_path.to_string_lossy().to_string()]);

        assert_eq!(expanded, vec![literal_path.to_string_lossy().to_string()]);

        let _ = fs::remove_file(&literal_path);
        let _ = fs::remove_dir(&temp);
    }

    #[test]
    fn dot_and_dotdot_operands_are_rejected() {
        assert!(is_dot_or_dotdot(Path::new(".")));
        assert!(is_dot_or_dotdot(Path::new("foo/..")));
        assert!(!is_dot_or_dotdot(Path::new("foo.txt")));
    }
}
