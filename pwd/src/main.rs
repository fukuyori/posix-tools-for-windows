// pwd - 現在の作業ディレクトリを表示
// POSIX.1-2017準拠 + GNU拡張

use glob::glob_with;
use std::env;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
struct Options {
    logical: bool,   // -L: 論理パス（シンボリックリンクを解決しない）
    physical: bool,  // -P: 物理パス（シンボリックリンクを解決）
    show_help: bool,
    show_version: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pwd: {}", e);
            std::process::exit(2);
        }
    };
    
    if opts.show_help {
        print_help();
        std::process::exit(0);
    }
    
    if opts.show_version {
        println!("pwd (Rust Windows版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張");
        std::process::exit(0);
    }
    
    match get_current_dir(&opts) {
        Ok(path) => {
            println!("{}", path.display());
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("pwd: {}", format_error(&e));
            std::process::exit(1);
        }
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options::default();
    
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            // POSIX標準オプション
            "-L" => {
                opts.logical = true;
                opts.physical = false;
            }
            "-P" => {
                opts.physical = true;
                opts.logical = false;
            }
            // GNU拡張オプション
            "--help" => opts.show_help = true,
            "--version" => opts.show_version = true,
            // 複合オプション処理
            s if s.starts_with('-') && !s.starts_with("--") => {
                for c in s.chars().skip(1) {
                    match c {
                        'L' => {
                            opts.logical = true;
                            opts.physical = false;
                        }
                        'P' => {
                            opts.physical = true;
                            opts.logical = false;
                        }
                        _ => {
                            return Err(format!("不正なオプション: '-{}'", c));
                        }
                    }
                }
            }
            s if s.starts_with("--") => {
                return Err(format!("不明なオプション: '{}'", s));
            }
            // POSIXではpwdは引数を取らない
            _ => {
                return Err(format!("余分な引数: '{}'", arg));
            }
        }
    }
    
    Ok(opts)
}

fn print_help() {
    println!(r#"使い方: pwd [オプション]...

現在の作業ディレクトリの名前を表示します。

オプション:
  -L        論理的な作業ディレクトリを使用
            シンボリックリンクを含むパスを保持（PWD環境変数を使用）
  -P        物理的な作業ディレクトリを使用
            シンボリックリンクを解決してすべて実際のディレクトリに置換
      --help     このヘルプを表示して終了
      --version  バージョン情報を表示して終了

-L と -P の両方が指定された場合、最後に指定したものが有効になります。
オプションが指定されない場合は -P と同じ動作をします。

終了ステータス:
  0  正常終了
  1  作業ディレクトリの取得に失敗
  2  オプションエラー

例:
  pwd       現在の作業ディレクトリを物理パスで表示
  pwd -L    PWD環境変数の値を表示（シンボリックリンクを含む場合あり）
  pwd -P    シンボリックリンクを解決した物理パスを表示"#);
}

fn get_current_dir(opts: &Options) -> io::Result<PathBuf> {
    // -L: 論理パス（PWD環境変数を優先）
    if opts.logical && !opts.physical {
        if let Ok(pwd) = env::var("PWD") {
            let pwd_path = PathBuf::from(&pwd);
            // PWDが有効なディレクトリを指しているか確認
            if pwd_path.is_absolute() && pwd_path.is_dir() && logical_pwd_is_posixish(&pwd_path) {
                // PWDが実際のカレントディレクトリと同じかを確認
                // （同じinode/デバイスを指しているか）
                let cwd = env::current_dir()?;
                if paths_same(&pwd_path, &cwd) {
                    return expand_existing_path_case(&pwd_path);
                }
            }
        }
    }
    
    // -P または デフォルト: 物理パス
    let cwd = env::current_dir()?;
    normalize_path(&cwd)
}

/// 二つのパスが同じディレクトリを指しているか確認
fn paths_same(path1: &PathBuf, path2: &PathBuf) -> bool {
    match (std::fs::canonicalize(path1), std::fs::canonicalize(path2)) {
        (Ok(p1), Ok(p2)) => {
            #[cfg(windows)]
            {
                p1.to_string_lossy().eq_ignore_ascii_case(&p2.to_string_lossy())
            }
            #[cfg(not(windows))]
            {
                p1 == p2
            }
        }
        _ => false,
    }
}

fn logical_pwd_is_posixish(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    !raw
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
        .any(|component| component == "." || component == "..")
}

/// パスを正規化（\\?\プレフィックスを除去）
fn normalize_path(path: &PathBuf) -> io::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(p) => {
            let p = strip_verbatim_prefix(&p);
            // Windows の \\?\ プレフィックスを除去
            Ok(p)
        }
        Err(e) => {
            // canonicalizeに失敗した場合は元のパスを返す
            // （ディレクトリが存在しない等の場合）
            if path.exists() {
                Ok(path.clone())
            } else {
                Err(e)
            }
        }
    }
}

fn expand_existing_path_case(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        let mut expanded = PathBuf::new();

        for component in path.components() {
            match component {
                Component::Prefix(prefix) => expanded.push(prefix.as_os_str()),
                Component::RootDir => expanded.push(component.as_os_str()),
                Component::Normal(part) => {
                    let part = part.to_string_lossy();
                    let pattern = format!("{}*", escape_glob_pattern(&part));
                    let search_root = if expanded.as_os_str().is_empty() {
                        PathBuf::from(".")
                    } else {
                        expanded.clone()
                    };

                    let mut matches = glob_with(
                        &search_root.join(pattern).to_string_lossy(),
                        glob::MatchOptions {
                            case_sensitive: false,
                            require_literal_separator: true,
                            require_literal_leading_dot: false,
                        },
                    )
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.msg.to_string()))?;

                    let next = matches
                        .find_map(Result::ok)
                        .unwrap_or_else(|| expanded.join(part.as_ref()));

                    expanded = next;
                }
                Component::CurDir | Component::ParentDir => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "PWD に '.' または '..' を含めることはできません",
                    ));
                }
            }
        }

        return Ok(strip_verbatim_prefix(&expanded));
    }

    #[cfg(not(windows))]
    {
        Ok(path.to_path_buf())
    }
}

fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{}", stripped))
        } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            path.to_path_buf()
        }
    }

    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

fn escape_glob_pattern(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '*' | '?' | '[' | ']' | '{' | '}' => {
                escaped.push('[');
                escaped.push(ch);
                escaped.push(']');
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// エラーメッセージをフォーマット
fn format_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => {
            "現在の作業ディレクトリが存在しません".to_string()
        }
        io::ErrorKind::PermissionDenied => {
            "現在の作業ディレクトリへのアクセス許可がありません".to_string()
        }
        _ => {
            #[cfg(windows)]
            if let Some(code) = e.raw_os_error() {
                return match code {
                    2 => "現在の作業ディレクトリが見つかりません".to_string(),
                    3 => "パスが見つかりません".to_string(),
                    5 => "アクセスが拒否されました".to_string(),
                    _ => format!("作業ディレクトリの取得に失敗しました: {} (エラーコード: {})", e, code),
                };
            }
            format!("作業ディレクトリの取得に失敗しました: {}", e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dot_components_for_logical_pwd() {
        assert!(!logical_pwd_is_posixish(Path::new(r"C:\work\.\src")));
        assert!(!logical_pwd_is_posixish(Path::new(r"C:\work\..\src")));
        assert!(logical_pwd_is_posixish(Path::new(r"C:\work\src")));
    }

    #[test]
    fn escapes_glob_meta_characters() {
        assert_eq!(escape_glob_pattern("a[b]?*{c}"), "a[[]b[]][?][*][{]c[}]");
    }
}
