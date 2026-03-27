// which - コマンドのパスを表示
// POSIX互換 + GNU拡張

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Options {
    // 標準オプション
    show_all: bool, // -a: すべての一致を表示

    // GNU拡張オプション
    silent: bool,         // -s, --silent: 出力なし（終了コードのみ）
    skip_dot: bool,       // --skip-dot: PATHの「.」をスキップ
    skip_tilde: bool,     // --skip-tilde: PATHの「~」で始まるエントリをスキップ
    show_dot: bool,       // --show-dot: 「.」内で見つかった場合「./cmd」と表示
    show_tilde: bool,     // --show-tilde: HOMEディレクトリを「~」で表示
    tty_only: bool,       // --tty-only: 端末に接続されている場合のみ処理
    read_alias: bool,     // --read-alias: 標準入力からエイリアスを読む
    skip_alias: bool,     // --skip-alias: --read-aliasで読んだエイリアスを無視
    read_functions: bool, // --read-functions: シェル関数を読む
    skip_functions: bool, // --skip-functions: シェル関数を無視

    show_version: bool,
    show_help: bool,
}

#[derive(Clone, Debug)]
struct SearchContext {
    extensions: Vec<String>,
    paths: Vec<PathBuf>,
    home_dir: Option<String>,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let (opts, commands) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("which: {}", e);
            eprintln!("詳細は 'which --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("which (Rust版) 1.0.0");
        println!("POSIX互換 + GNU拡張");
        std::process::exit(0);
    }

    if commands.is_empty() {
        eprintln!("which: コマンド名を指定してください");
        eprintln!("詳細は 'which --help' を参照してください");
        std::process::exit(2);
    }

    // --tty-only: 端末に接続されていない場合は何もしない
    if opts.tty_only && !is_tty() {
        std::process::exit(0);
    }

    let context = SearchContext::from_env();
    let commands = expand_globs(commands, &context);

    let mut all_found = true;

    for cmd in &commands {
        let found = which(cmd, &opts, &context);
        if !found {
            all_found = false;
        }
    }

    // 終了コード: 0=すべて見つかった, 1=1つ以上見つからなかった
    std::process::exit(if all_found { 0 } else { 1 });
}

impl SearchContext {
    fn from_env() -> Self {
        Self {
            extensions: get_pathext(),
            paths: get_path_dirs(),
            home_dir: env::var("USERPROFILE").or_else(|_| env::var("HOME")).ok(),
        }
    }
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options::default();
    let mut commands = Vec::new();
    let mut i = 1;
    let mut end_of_opts = false;

    while i < args.len() {
        let arg = &args[i];

        if end_of_opts {
            commands.push(arg.clone());
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
                "--all" => opts.show_all = true,
                "--silent" | "--quiet" => opts.silent = true,
                "--skip-dot" => opts.skip_dot = true,
                "--skip-tilde" => opts.skip_tilde = true,
                "--show-dot" => opts.show_dot = true,
                "--show-tilde" => opts.show_tilde = true,
                "--tty-only" => opts.tty_only = true,
                "--read-alias" => opts.read_alias = true,
                "--skip-alias" => opts.skip_alias = true,
                "--read-functions" => opts.read_functions = true,
                "--skip-functions" => opts.skip_functions = true,
                "--version" => opts.show_version = true,
                "--help" => opts.show_help = true,
                _ => return Err(format!("不明なオプション: '{}'", arg)),
            }
            i += 1;
            continue;
        }

        // 短縮オプション
        if arg.starts_with('-') && arg.len() > 1 {
            for c in arg[1..].chars() {
                match c {
                    'a' => opts.show_all = true,
                    's' | 'q' => opts.silent = true,
                    'i' => opts.read_alias = true,
                    'v' => opts.show_version = true,
                    'h' => opts.show_help = true,
                    _ => return Err(format!("不正なオプション: '-{}'", c)),
                }
            }
            i += 1;
            continue;
        }

        commands.push(arg.clone());
        i += 1;
    }

    Ok((opts, commands))
}

/// Windows シェルでは展開されないワイルドカードを which 側で補う。
/// パス区切りを含む場合は実ファイルパスに対して、そうでない場合は PATH 上のコマンド名に対して展開する。
fn expand_globs(raw_commands: Vec<String>, context: &SearchContext) -> Vec<String> {
    let mut result = Vec::new();

    for pattern in raw_commands {
        if !has_glob_meta(&pattern) {
            result.push(pattern);
            continue;
        }

        let expanded = if contains_path_separator(&pattern) {
            expand_path_glob(&pattern)
        } else {
            expand_command_glob(&pattern, context)
        };

        if expanded.is_empty() {
            result.push(pattern);
        } else {
            result.extend(expanded);
        }
    }

    result
}

fn expand_path_glob(pattern: &str) -> Vec<String> {
    let options = glob::MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    let mut matches = Vec::new();
    if let Ok(paths) = glob::glob_with(pattern, options) {
        for path in paths.filter_map(Result::ok) {
            if path.is_file() {
                matches.push(path.to_string_lossy().to_string());
            }
        }
    }
    matches
}

fn expand_command_glob(pattern: &str, context: &SearchContext) -> Vec<String> {
    let options = glob::MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    let compiled = match glob::Pattern::new(pattern) {
        Ok(pattern) => pattern,
        Err(_) => return Vec::new(),
    };

    let mut matched = Vec::new();
    let mut seen = HashSet::new();

    for dir in &context.paths {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let file_name = entry.file_name().to_string_lossy().to_string();
                let Some(candidate_name) = command_name_for_match(&file_name, &context.extensions)
                else {
                    continue;
                };

                if compiled.matches_with(&candidate_name, options) {
                    let key = candidate_name.to_ascii_lowercase();
                    if seen.insert(key) {
                        matched.push(candidate_name);
                    }
                }
            }
        }
    }

    matched
}

fn command_name_for_match(file_name: &str, extensions: &[String]) -> Option<String> {
    let file_name_lower = file_name.to_ascii_lowercase();
    for ext in extensions {
        if file_name_lower.ends_with(ext) {
            let base_len = file_name.len().saturating_sub(ext.len());
            return Some(file_name[..base_len].to_string());
        }
    }

    if Path::new(file_name).extension().is_none() {
        Some(file_name.to_string())
    } else {
        None
    }
}

fn has_glob_meta(value: &str) -> bool {
    value.contains('*') || value.contains('?') || value.contains('[')
}

fn contains_path_separator(value: &str) -> bool {
    value.contains('\\') || value.contains('/') || value.contains(':')
}

fn print_help() {
    println!(
        r#"使い方: which [オプション] コマンド名...

コマンドの実行ファイルのフルパスを表示します。
PATHおよびPATHEXT環境変数を使用して検索します。

標準オプション:
  -a, --all             PATH内のすべての一致を表示（最初の1つだけでなく）

GNU拡張オプション:
  -s, -q, --silent, --quiet
                        出力なし（終了コードのみ返す）
  -i, --read-alias      標準入力からエイリアスを読む（未実装）
      --skip-alias      --read-aliasで読んだエイリアスを無視
      --read-functions  シェル関数を読む（未実装）
      --skip-functions  シェル関数を無視
      --skip-dot        PATHの「.」をスキップ
      --skip-tilde      PATHの「~」で始まるエントリをスキップ
      --show-dot        PATH上の「.」で見つかった場合「./cmd」と表示
      --show-tilde      HOMEディレクトリを「~」で表示
      --tty-only        端末に接続されている場合のみ処理

その他:
  -h, --help            このヘルプを表示
  -v, --version         バージョン情報を表示

終了ステータス:
  0  すべてのコマンドが見つかった
  1  1つ以上のコマンドが見つからなかった
  2  オプションエラー

Windows固有の動作:
  - PATHEXT環境変数の拡張子（.exe, .cmd, .bat等）を自動的に補完
  - ファイル名の大文字小文字を区別しない
  - ワイルドカードを内部で展開し、Windows シェルでも Linux に近い体験に寄せる

POSIX 寄りの動作:
  - カレントディレクトリは PATH に含まれる場合のみ検索
  - パス区切りを含む引数は PATH ではなく、そのパスだけを確認

例:
  which notepad              notepad.exeのパスを表示
  which -a python            PATH内のすべてのpythonを表示
  which cmd powershell       複数コマンドを検索
  which -s notepad && echo found
                             見つかった場合のみメッセージ表示
  which note*                ワイルドカードで検索"#
    );
}

fn which(cmd: &str, opts: &Options, context: &SearchContext) -> bool {
    let mut found = false;
    let mut found_paths: Vec<PathBuf> = Vec::new();

    // コマンドがすでにパスを含んでいる場合は、そのパスのみ確認する。
    if contains_path_separator(cmd) {
        let path = PathBuf::from(cmd);
        if let Some(result) = check_executable(&path, &context.extensions) {
            if !opts.silent {
                println!("{}", format_path(&result, opts, &context.home_dir));
            }
            return true;
        }
        if !opts.silent {
            eprintln!("which: {}: そのようなファイルはありません", cmd);
        }
        return false;
    }

    // PATH 内を検索
    for dir in &context.paths {
        if is_dot_path(dir) {
            if opts.skip_dot {
                continue;
            }
        } else if opts.skip_tilde {
            let dir_str = dir.to_string_lossy();
            if dir_str.starts_with('~') || dir_str.starts_with("~/") || dir_str.starts_with("~\\") {
                continue;
            }
        }

        if let Some(result) = check_in_dir(dir, cmd, &context.extensions) {
            if !found_paths.iter().any(|p| same_file(p, &result)) {
                found_paths.push(result.clone());
                if !opts.silent {
                    let display = if opts.show_dot && is_dot_path(dir) {
                        format!(
                            ".\\{}",
                            result.file_name().unwrap_or_default().to_string_lossy()
                        )
                    } else {
                        format_path(&result, opts, &context.home_dir)
                    };
                    println!("{}", display);
                }
                found = true;
                if !opts.show_all {
                    return true;
                }
            }
        }
    }

    if !found && !opts.silent {
        eprintln!("which: {}: コマンドが見つかりません", cmd);
    }

    found
}

fn format_path(path: &Path, opts: &Options, home_dir: &Option<String>) -> String {
    let path_str = path.to_string_lossy().to_string();

    // --show-tilde: HOMEディレクトリを「~」で表示
    if opts.show_tilde {
        if let Some(home) = home_dir {
            let home_normalized = home.replace('/', "\\");
            if path_str
                .to_ascii_lowercase()
                .starts_with(&home_normalized.to_ascii_lowercase())
            {
                return format!("~{}", &path_str[home.len()..]);
            }
        }
    }

    path_str
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn get_pathext() -> Vec<String> {
    // PATHEXT環境変数から実行可能な拡張子を取得
    env::var("PATHEXT")
        .unwrap_or_else(|_| {
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC;.PS1".to_string()
        })
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn get_path_dirs() -> Vec<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect()
}

fn check_in_dir(dir: &Path, cmd: &str, extensions: &[String]) -> Option<PathBuf> {
    let candidate = dir.join(cmd);

    if has_known_extension(cmd, extensions) {
        return resolve_file_case_insensitively(&candidate).map(|path| normalize_path(&path));
    }

    for ext in extensions {
        let with_ext = PathBuf::from(format!("{}{}", candidate.to_string_lossy(), ext));
        if let Some(path) = resolve_file_case_insensitively(&with_ext) {
            return Some(normalize_path(&path));
        }
    }

    resolve_file_case_insensitively(&candidate).map(|path| normalize_path(&path))
}

fn check_executable(path: &Path, extensions: &[String]) -> Option<PathBuf> {
    if let Some(resolved) = resolve_file_case_insensitively(path) {
        return Some(normalize_path(&resolved));
    }

    let path_str = path.to_string_lossy();
    if !has_known_extension(&path_str, extensions) {
        for ext in extensions {
            let with_ext = PathBuf::from(format!("{}{}", path.display(), ext));
            if let Some(resolved) = resolve_file_case_insensitively(&with_ext) {
                return Some(normalize_path(&resolved));
            }
        }
    }

    None
}

fn has_known_extension(value: &str, extensions: &[String]) -> bool {
    let lowered = value.to_ascii_lowercase();
    extensions.iter().any(|ext| lowered.ends_with(ext))
}

fn resolve_file_case_insensitively(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }

    let file_name = path.file_name()?.to_string_lossy();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let resolved_parent = resolve_dir_case_insensitively(parent)?;

    let entries = fs::read_dir(&resolved_parent).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let candidate_name = entry.file_name();
        if candidate_name
            .to_string_lossy()
            .eq_ignore_ascii_case(file_name.as_ref())
        {
            let candidate_path = entry.path();
            if candidate_path.is_file() {
                return Some(candidate_path);
            }
        }
    }

    None
}

fn resolve_dir_case_insensitively(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return Some(PathBuf::from("."));
    }

    if path.is_dir() {
        return Some(path.to_path_buf());
    }

    let mut resolved = if let Some(prefix) = path.components().next() {
        match prefix {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                PathBuf::from(prefix.as_os_str())
            }
            _ => PathBuf::new(),
        }
    } else {
        PathBuf::new()
    };

    for component in path.components() {
        use std::path::Component;

        match component {
            Component::Prefix(_) | Component::RootDir => {}
            Component::CurDir => {
                if resolved.as_os_str().is_empty() {
                    resolved.push(".");
                }
            }
            Component::ParentDir => resolved.push(".."),
            Component::Normal(part) => {
                let search_dir = if resolved.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    resolved.clone()
                };

                let entries = fs::read_dir(&search_dir).ok()?;
                let mut matched = None;
                for entry in entries.filter_map(Result::ok) {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&part.to_string_lossy())
                    {
                        let candidate_path = entry.path();
                        if candidate_path.is_dir() {
                            matched = Some(candidate_path);
                            break;
                        }
                    }
                }

                resolved = matched?;
            }
        }
    }

    Some(resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    match fs::canonicalize(path) {
        Ok(p) => {
            // \\?\ プレフィックスを除去
            let s = p.to_string_lossy();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                PathBuf::from(stripped)
            } else {
                p
            }
        }
        Err(_) => path.to_path_buf(),
    }
}

fn is_dot_path(path: &Path) -> bool {
    path == Path::new(".")
}

fn is_tty() -> bool {
    // Windowsでの端末判定
    #[cfg(windows)]
    {
        use std::io;
        use std::os::windows::io::AsRawHandle;

        unsafe {
            let handle = io::stdout().as_raw_handle();
            let mut mode: u32 = 0;
            // GetConsoleMode が成功すれば端末
            windows_sys::Win32::System::Console::GetConsoleMode(
                handle as windows_sys::Win32::Foundation::HANDLE,
                &mut mode,
            ) != 0
        }
    }

    #[cfg(not(windows))]
    {
        false
    }
}

trait PathEqIgnoreAsciiCase {
    fn eq_ignore_ascii_case(&self, other: &Path) -> bool;
}

impl PathEqIgnoreAsciiCase for Path {
    fn eq_ignore_ascii_case(&self, other: &Path) -> bool {
        self.to_string_lossy()
            .eq_ignore_ascii_case(&other.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(name: &str) -> PathBuf {
        let mut dir = env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        dir.push(format!("which-tests-{name}-{nanos}-{id}"));
        dir
    }

    fn make_context(paths: Vec<PathBuf>) -> SearchContext {
        SearchContext {
            extensions: vec![".exe".into(), ".cmd".into(), ".bat".into()],
            paths,
            home_dir: None,
        }
    }

    #[test]
    fn does_not_search_current_directory_unless_it_is_in_path() {
        let context = make_context(vec![]);
        let temp = unique_temp_dir("no-dot");
        create_dir_all(&temp).unwrap();
        write(temp.join("tool.exe"), b"stub").unwrap();

        let result = check_in_dir(Path::new("."), "tool", &context.extensions);
        assert!(result.is_none());

        let explicit_context = make_context(vec![temp.clone()]);
        let found = which("tool", &Options::default(), &explicit_context);
        assert!(found);

        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn command_glob_expands_case_insensitively_and_strips_pathext() {
        let temp = unique_temp_dir("glob");
        create_dir_all(&temp).unwrap();
        write(temp.join("NotePad.EXE"), b"stub").unwrap();
        write(temp.join("noteworthy.cmd"), b"stub").unwrap();
        write(temp.join("notes.txt"), b"stub").unwrap();

        let context = make_context(vec![temp.clone()]);
        let expanded = expand_globs(vec!["note*".into()], &context);

        assert_eq!(
            expanded,
            vec!["NotePad".to_string(), "noteworthy".to_string()]
        );

        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn explicit_paths_are_resolved_case_insensitively() {
        let temp = unique_temp_dir("path-case");
        create_dir_all(&temp).unwrap();
        let file = temp.join("PowerShell.EXE");
        write(&file, b"stub").unwrap();

        let lookup = temp.join("powershell.exe");
        let resolved = check_executable(&lookup, &[".exe".into()]);

        assert_eq!(
            resolved.map(|p| normalize_path(&p)),
            Some(normalize_path(&file))
        );

        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn path_glob_expands_files_directly() {
        let temp = unique_temp_dir("path-glob");
        create_dir_all(&temp).unwrap();
        let file = temp.join("python.EXE");
        write(&file, b"stub").unwrap();

        let pattern = format!("{}\\py*", temp.to_string_lossy());
        let expanded = expand_globs(vec![pattern], &make_context(vec![]));

        assert_eq!(expanded, vec![file.to_string_lossy().to_string()]);

        fs::remove_dir_all(temp).unwrap();
    }
}
