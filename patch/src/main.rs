use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use glob::{glob_with, MatchOptions};

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum PatchFormat {
    Unified,
    Context,
    Normal,
    Ed,
}

#[derive(Debug)]
struct Config {
    /// パッチファイル（-i）
    patch_file: Option<String>,
    /// ストリップレベル（-p）
    strip_level: usize,
    /// 出力ファイル（-o）
    output_file: Option<String>,
    /// 逆パッチ（-R）
    reverse: bool,
    /// ドライラン（--dry-run）
    dry_run: bool,
    /// バックアップ（-b）
    backup: bool,
    /// バックアップサフィックス（-z, --suffix）
    backup_suffix: String,
    /// 強制実行（-f）
    force: bool,
    /// バッチモード（-t）
    batch: bool,
    /// 詳細表示（--verbose）
    verbose: bool,
    /// サイレントモード（-s, --silent）
    silent: bool,
    /// ディレクトリ（-d）
    directory: Option<String>,
    /// ファズファクター（-F）
    fuzz: usize,
    /// 入力ファイル
    input_file: Option<String>,
    /// POSIX準拠モード
    posix: bool,
    /// 前方参照のみ（--forward）
    forward: bool,
    /// 既に適用済みなら無視（-N）
    ignore_applied: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            patch_file: None,
            strip_level: 0,
            output_file: None,
            reverse: false,
            dry_run: false,
            backup: false,
            backup_suffix: ".orig".to_string(),
            force: false,
            batch: false,
            verbose: false,
            silent: false,
            directory: None,
            fuzz: 2,
            input_file: None,
            posix: false,
            forward: false,
            ignore_applied: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: patch [オプション]... [入力ファイル [パッチファイル]]
パッチファイルを適用してファイルを更新します。

入力オプション:
  -i, --input=PATCHFILE   パッチを読み込むファイル（デフォルト：標準入力）
  -p, --strip=NUM         ファイルパスから先頭NUM個のコンポーネントを削除
  -d, --directory=DIR     DIRに移動してからパッチを適用

出力オプション:
  -o, --output=FILE       結果をFILEに出力（元ファイルを変更しない）
  -r, --reject-file=FILE  リジェクトをFILEに出力
  -b, --backup            変更前にバックアップを作成
  -z, --suffix=SUFFIX     バックアップのサフィックス（デフォルト：.orig）
  -V, --version-control=METHOD  バックアップ方式を指定

動作オプション:
  -R, --reverse           パッチを逆適用
  -N, --forward           既に適用済みのパッチは無視
  -f, --force             確認なしで実行
  -t, --batch             -f と同様だが質問には n で応答
  -F, --fuzz=NUM          ファズファクター（デフォルト：2）
  -l, --ignore-whitespace 空白の違いを緩く比較
  --dry-run               実際には変更せず、適用可能か確認
  --verbose               詳細な情報を表示
  -s, --silent, --quiet   最小限の出力
  --posix                 POSIX準拠モード
      --help              このヘルプを表示
      --version           バージョン情報を表示

パッチ形式:
  ユニファイド形式、コンテキスト形式、通常形式を自動検出します。

例:
  patch < fix.patch                   標準入力からパッチを適用
  patch -p1 < fix.patch               パス先頭を1つ削除して適用
  patch -i fix.patch                  fix.patchを適用
  patch -R < fix.patch                パッチを逆適用（元に戻す）
  patch -b -i fix.patch               バックアップを作成して適用
  patch --dry-run < fix.patch         実際には適用せず確認のみ
  patch -p1 -d /path/to/src < fix.patch  ディレクトリを指定して適用

終了ステータス:
  0  パッチが正常に適用された
  1  一部のハンクが失敗した
  2  エラー発生
"#
    );
}

fn print_version() {
    eprintln!("patch (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn windows_glob_match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    }
}

fn sort_paths_case_insensitive(paths: &mut [String]) {
    paths.sort_by_cached_key(|path| path.to_ascii_lowercase());
}

/// Windows ではシェルが展開しないため、Linux に近い引数展開を内部で行う。
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if !has_glob_meta(pattern) {
        return Ok(vec![pattern.to_string()]);
    }

    let mut paths: Vec<String> = glob_with(pattern, windows_glob_match_options())
        .map_err(|e| format!("globパターンエラー: {}", e))?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if paths.is_empty() {
        Ok(vec![pattern.to_string()])
    } else {
        sort_paths_case_insensitive(&mut paths);
        Ok(paths)
    }
}

fn resolve_existing_path(path: &str) -> Result<String, String> {
    if path == "-" || path.is_empty() {
        return Ok(path.to_string());
    }

    if has_glob_meta(path) {
        let mut candidates: Vec<String> = glob_with(path, windows_glob_match_options())
            .map_err(|e| format!("globパターンエラー: {}", e))?
            .filter_map(Result::ok)
            .map(|p| p.to_string_lossy().to_string())
            .filter(|candidate| candidate != path && Path::new(candidate).exists())
            .collect();
        sort_paths_case_insensitive(&mut candidates);

        if candidates.is_empty() {
            Ok(path.to_string())
        } else {
            Ok(candidates[0].clone())
        }
    } else {
        resolve_literal_path_case_insensitively(path)
    }
}

fn resolve_literal_path_case_insensitively(path: &str) -> Result<String, String> {
    let original = Path::new(path);
    let cwd = env::current_dir().map_err(|e| format!("patch: カレントディレクトリ取得失敗: {}", e))?;
    let absolute = if original.is_absolute() {
        original.to_path_buf()
    } else {
        cwd.join(original)
    };

    let resolved = resolve_absolute_literal_path_case_insensitively(&absolute)?;
    if let Ok(relative) = resolved.strip_prefix(&cwd) {
        Ok(relative.to_string_lossy().to_string())
    } else {
        Ok(resolved.to_string_lossy().to_string())
    }
}

fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

fn resolve_absolute_literal_path_case_insensitively(path: &Path) -> Result<PathBuf, String> {
    if let Ok(canonical) = fs::canonicalize(path) {
        return Ok(strip_windows_verbatim_prefix(canonical));
    }

    let mut current = path.to_path_buf();
    let mut unresolved_components = Vec::new();

    while !current.exists() {
        let Some(name) = current.file_name() else {
            return Ok(path.to_path_buf());
        };
        unresolved_components.push(name.to_os_string());

        let Some(parent) = current.parent() else {
            return Ok(path.to_path_buf());
        };
        if parent == current {
            return Ok(path.to_path_buf());
        }
        current = parent.to_path_buf();
    }

    let mut resolved = strip_windows_verbatim_prefix(
        fs::canonicalize(&current).map_err(|e| format!("patch: {}: {}", current.display(), e))?,
    );
    unresolved_components.reverse();

    for component in unresolved_components {
        let target_name = component.to_string_lossy().to_ascii_lowercase();
        let mut matched_path = None;

        for entry in fs::read_dir(&resolved)
            .map_err(|e| format!("patch: {}: {}", resolved.display(), e))?
        {
            let entry = entry.map_err(|e| format!("patch: {}: {}", resolved.display(), e))?;
            if entry.file_name().to_string_lossy().to_ascii_lowercase() == target_name {
                matched_path = Some(entry.path());
                break;
            }
        }

        if let Some(path) = matched_path {
            resolved = path;
        } else {
            resolved.push(component);
        }
    }

    Ok(resolved)
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    parse_args_from(args)
}

fn parse_args_from<I>(raw_args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = raw_args.into_iter().collect();
    let mut config = Config::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-i" || arg == "--input" {
            i += 1;
            if i >= args.len() {
                return Err("-i にはパッチファイルが必要です".to_string());
            }
            config.patch_file = Some(args[i].clone());
        } else if arg.starts_with("--input=") {
            config.patch_file = Some(arg[8..].to_string());
        } else if arg.starts_with("-i") && arg.len() > 2 {
            config.patch_file = Some(arg[2..].to_string());
        } else if arg == "-p" || arg == "--strip" {
            i += 1;
            if i >= args.len() {
                return Err("-p にはストリップレベルが必要です".to_string());
            }
            config.strip_level = args[i]
                .parse()
                .map_err(|_| format!("無効なストリップレベル: '{}'", args[i]))?;
        } else if arg.starts_with("--strip=") {
            config.strip_level = arg[8..]
                .parse()
                .map_err(|_| format!("無効なストリップレベル: '{}'", &arg[8..]))?;
        } else if arg.starts_with("-p") && arg.len() > 2 {
            config.strip_level = arg[2..]
                .parse()
                .map_err(|_| format!("無効なストリップレベル: '{}'", &arg[2..]))?;
        } else if arg == "-o" || arg == "--output" {
            i += 1;
            if i >= args.len() {
                return Err("-o には出力ファイルが必要です".to_string());
            }
            config.output_file = Some(args[i].clone());
        } else if arg.starts_with("--output=") {
            config.output_file = Some(arg[9..].to_string());
        } else if arg == "-d" || arg == "--directory" {
            i += 1;
            if i >= args.len() {
                return Err("-d にはディレクトリが必要です".to_string());
            }
            config.directory = Some(args[i].clone());
        } else if arg.starts_with("--directory=") {
            config.directory = Some(arg[12..].to_string());
        } else if arg == "-R" || arg == "--reverse" {
            config.reverse = true;
        } else if arg == "-N" || arg == "--forward" {
            config.forward = true;
            config.ignore_applied = true;
        } else if arg == "-f" || arg == "--force" {
            config.force = true;
        } else if arg == "-t" || arg == "--batch" {
            config.batch = true;
        } else if arg == "-b" || arg == "--backup" {
            config.backup = true;
        } else if arg == "-z" || arg == "--suffix" {
            i += 1;
            if i >= args.len() {
                return Err("-z にはサフィックスが必要です".to_string());
            }
            config.backup_suffix = args[i].clone();
        } else if arg.starts_with("--suffix=") {
            config.backup_suffix = arg[9..].to_string();
        } else if arg.starts_with("-z") && arg.len() > 2 {
            config.backup_suffix = arg[2..].to_string();
        } else if arg == "-F" || arg == "--fuzz" {
            i += 1;
            if i >= args.len() {
                return Err("-F にはファズファクターが必要です".to_string());
            }
            config.fuzz = args[i]
                .parse()
                .map_err(|_| format!("無効なファズファクター: '{}'", args[i]))?;
        } else if arg.starts_with("--fuzz=") {
            config.fuzz = arg[7..]
                .parse()
                .map_err(|_| format!("無効なファズファクター: '{}'", &arg[7..]))?;
        } else if arg.starts_with("-F") && arg.len() > 2 {
            config.fuzz = arg[2..]
                .parse()
                .map_err(|_| format!("無効なファズファクター: '{}'", &arg[2..]))?;
        } else if arg == "--dry-run" {
            config.dry_run = true;
        } else if arg == "--verbose" {
            config.verbose = true;
        } else if arg == "-s" || arg == "--silent" || arg == "--quiet" {
            config.silent = true;
        } else if arg == "--posix" {
            config.posix = true;
        } else if arg == "-l" || arg == "--ignore-whitespace" {
            // 空白無視（将来の拡張用）
        } else if arg == "-r" || arg == "--reject-file" {
            i += 1;
            // リジェクトファイル（将来の拡張用）
        } else if arg == "-V" || arg == "--version-control" {
            i += 1;
            // バージョン管理方式（将来の拡張用）
        } else if arg == "--" {
            for j in (i + 1)..args.len() {
                positional.extend(expand_glob(&args[j])?);
            }
            break;
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("不明なオプション: {}", arg));
        } else {
            positional.extend(expand_glob(arg)?);
        }

        i += 1;
    }

    // 位置引数の処理
    // POSIX: patch [options] [file]
    // GNU: patch [options] [origfile [patchfile]]
    if !positional.is_empty() {
        // 最初の引数は入力ファイル
        config.input_file = Some(positional[0].clone());
        if positional.len() >= 2 {
            // 2番目の引数はパッチファイル
            config.patch_file = Some(positional[1].clone());
        }
    }

    if let Some(path) = config.input_file.as_deref() {
        config.input_file = Some(resolve_existing_path(path)?);
    }

    if let Some(path) = config.patch_file.as_deref() {
        config.patch_file = Some(resolve_existing_path(path)?);
    }

    Ok(config)
}

/// エンコーディング自動検出
fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return encoding_rs::UTF_8;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16LE;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return encoding_rs::UTF_8;
    }
    encoding_rs::UTF_8
}

/// ファイルを行に読み込む
fn read_lines(path: &str) -> Result<Vec<String>, String> {
    let resolved_path = resolve_existing_path(path)?;
    let p = Path::new(&resolved_path);
    if !p.exists() {
        return Ok(Vec::new()); // 新規ファイルの場合
    }

    let mut raw_bytes = Vec::new();
    File::open(p)
        .map_err(|e| format!("patch: {}: {}", resolved_path, e))?
        .read_to_end(&mut raw_bytes)
        .map_err(|e| format!("patch: {}: {}", resolved_path, e))?;

    let encoding = detect_encoding(&raw_bytes);
    let file = File::open(p).map_err(|e| format!("patch: {}: {}", resolved_path, e))?;
    let decoder = DecodeReaderBytesBuilder::new()
        .encoding(Some(encoding))
        .build(file);
    let reader = BufReader::new(decoder);

    reader
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("patch: {}: {}", resolved_path, e))
}

/// パッチを読み込む
fn read_patch(config: &Config) -> Result<String, String> {
    if let Some(ref path) = config.patch_file {
        let resolved_path = resolve_existing_path(path)?;
        let mut raw_bytes = Vec::new();
        File::open(&resolved_path)
            .map_err(|e| format!("patch: {}: {}", resolved_path, e))?
            .read_to_end(&mut raw_bytes)
            .map_err(|e| format!("patch: {}: {}", resolved_path, e))?;

        let encoding = detect_encoding(&raw_bytes);
        let (content, _, _) = encoding.decode(&raw_bytes);
        Ok(content.to_string())
    } else {
        let mut content = String::new();
        io::stdin()
            .read_to_string(&mut content)
            .map_err(|e| format!("patch: 標準入力: {}", e))?;
        Ok(content)
    }
}

/// パッチ形式を検出
fn detect_patch_format(content: &str) -> PatchFormat {
    for line in content.lines() {
        if line.starts_with("---") && content.contains("\n+++") {
            return PatchFormat::Unified;
        }
        if line.starts_with("***") && content.contains("\n---") {
            return PatchFormat::Context;
        }
        if line.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            if line.contains('c') || line.contains('a') || line.contains('d') {
                return PatchFormat::Normal;
            }
        }
    }
    PatchFormat::Unified
}

/// ハンク情報
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

/// パッチファイルを解析
#[derive(Debug)]
struct PatchFile {
    old_file: String,
    new_file: String,
    hunks: Vec<Hunk>,
}

/// パスからストリップ
fn strip_path(path: &str, level: usize) -> String {
    if level == 0 {
        return path.to_string();
    }

    let components: Vec<&str> = path.split(|c| c == '/' || c == '\\').collect();
    if level >= components.len() {
        components.last().unwrap_or(&path).to_string()
    } else {
        components[level..].join("/")
    }
}

/// ユニファイド形式のパッチを解析
fn parse_unified_patch(content: &str, strip_level: usize) -> Result<Vec<PatchFile>, String> {
    let mut patches = Vec::new();
    let mut current_patch: Option<PatchFile> = None;
    let mut current_hunk: Option<Hunk> = None;
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("---") {
            // 新しいパッチの開始
            if let Some(mut patch) = current_patch.take() {
                if let Some(hunk) = current_hunk.take() {
                    patch.hunks.push(hunk);
                }
                patches.push(patch);
            }

            let old_file = line[3..].trim();
            let old_file = old_file.split('\t').next().unwrap_or(old_file).trim();
            
            i += 1;
            if i >= lines.len() || !lines[i].starts_with("+++") {
                return Err("無効なパッチ形式: +++ 行がありません".to_string());
            }

            let new_file = lines[i][3..].trim();
            let new_file = new_file.split('\t').next().unwrap_or(new_file).trim();

            // /dev/null の処理
            let old_path = if old_file == "/dev/null" {
                String::new()
            } else {
                strip_path(old_file, strip_level)
            };
            let new_path = if new_file == "/dev/null" {
                String::new()
            } else {
                strip_path(new_file, strip_level)
            };

            current_patch = Some(PatchFile {
                old_file: old_path,
                new_file: new_path,
                hunks: Vec::new(),
            });
        } else if line.starts_with("@@") {
            // ハンクヘッダー
            if let Some(hunk) = current_hunk.take() {
                if let Some(ref mut patch) = current_patch {
                    patch.hunks.push(hunk);
                }
            }

            // @@ -old_start,old_count +new_start,new_count @@
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                return Err(format!("無効なハンクヘッダー: {}", line));
            }

            let old_range = parts[1].trim_start_matches('-');
            let new_range = parts[2].trim_start_matches('+');

            let (old_start, old_count) = parse_range(old_range)?;
            let (new_start, new_count) = parse_range(new_range)?;

            current_hunk = Some(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current_hunk {
            if line.starts_with('-') {
                hunk.lines.push(HunkLine::Remove(line[1..].to_string()));
            } else if line.starts_with('+') {
                hunk.lines.push(HunkLine::Add(line[1..].to_string()));
            } else if line.starts_with(' ') || line.is_empty() {
                let content = if line.is_empty() { "" } else { &line[1..] };
                hunk.lines.push(HunkLine::Context(content.to_string()));
            } else if line.starts_with('\\') {
                // "\ No newline at end of file" を無視
            }
        }

        i += 1;
    }

    // 最後のパッチとハンクを追加
    if let Some(mut patch) = current_patch {
        if let Some(hunk) = current_hunk {
            patch.hunks.push(hunk);
        }
        patches.push(patch);
    }

    Ok(patches)
}

/// 範囲をパース（"start,count" or "start"）
fn parse_range(s: &str) -> Result<(usize, usize), String> {
    let parts: Vec<&str> = s.split(',').collect();
    let start: usize = parts[0]
        .parse()
        .map_err(|_| format!("無効な範囲: {}", s))?;
    let count: usize = if parts.len() > 1 {
        parts[1].parse().map_err(|_| format!("無効な範囲: {}", s))?
    } else {
        1
    };
    Ok((start, count))
}

/// コンテキスト形式のパッチを解析
fn parse_context_patch(content: &str, strip_level: usize) -> Result<Vec<PatchFile>, String> {
    let mut patches = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("***") && !line.contains("****") {
            let old_file = line[3..].trim();
            let old_file = old_file.split('\t').next().unwrap_or(old_file).trim();
            
            i += 1;
            if i >= lines.len() || !lines[i].starts_with("---") {
                i += 1;
                continue;
            }

            let new_file = lines[i][3..].trim();
            let new_file = new_file.split('\t').next().unwrap_or(new_file).trim();

            let old_path = strip_path(old_file, strip_level);
            let new_path = strip_path(new_file, strip_level);

            let mut patch = PatchFile {
                old_file: old_path,
                new_file: new_path,
                hunks: Vec::new(),
            };

            i += 1;

            // ハンクを解析
            while i < lines.len() {
                if lines[i].starts_with("***************") {
                    i += 1;
                    if i >= lines.len() {
                        break;
                    }
                }

                if lines[i].starts_with("***") && lines[i].contains("****") {
                    // Old section: *** start,end ****
                    let old_range = lines[i]
                        .trim_start_matches('*')
                        .trim()
                        .trim_end_matches('*')
                        .trim();
                    let (old_start, old_end) = parse_context_range(old_range)?;
                    
                    let mut old_lines = Vec::new();
                    i += 1;
                    
                    while i < lines.len() && !lines[i].starts_with("---") {
                        let l = lines[i];
                        if l.starts_with("- ") {
                            old_lines.push(HunkLine::Remove(l[2..].to_string()));
                        } else if l.starts_with("! ") {
                            old_lines.push(HunkLine::Remove(l[2..].to_string()));
                        } else if l.starts_with("  ") {
                            old_lines.push(HunkLine::Context(l[2..].to_string()));
                        } else if l.starts_with("+ ") {
                            // Skip in old section
                        }
                        i += 1;
                    }

                    // New section: --- start,end ----
                    if i < lines.len() && lines[i].starts_with("---") && lines[i].contains("----") {
                        let new_range = lines[i]
                            .trim_start_matches('-')
                            .trim()
                            .trim_end_matches('-')
                            .trim();
                        let (new_start, new_end) = parse_context_range(new_range)?;
                        
                        let mut new_lines = Vec::new();
                        i += 1;
                        
                        while i < lines.len() 
                            && !lines[i].starts_with("***")
                            && !lines[i].starts_with("diff ")
                        {
                            let l = lines[i];
                            if l.starts_with("+ ") {
                                new_lines.push(HunkLine::Add(l[2..].to_string()));
                            } else if l.starts_with("! ") {
                                new_lines.push(HunkLine::Add(l[2..].to_string()));
                            } else if l.starts_with("  ") {
                                new_lines.push(HunkLine::Context(l[2..].to_string()));
                            }
                            i += 1;
                        }

                        // Merge old and new lines
                        let mut hunk_lines = Vec::new();
                        let mut oi = 0;
                        let mut ni = 0;
                        
                        while oi < old_lines.len() || ni < new_lines.len() {
                            match (&old_lines.get(oi), &new_lines.get(ni)) {
                                (Some(HunkLine::Context(c1)), Some(HunkLine::Context(c2))) if c1 == c2 => {
                                    hunk_lines.push(HunkLine::Context(c1.clone()));
                                    oi += 1;
                                    ni += 1;
                                }
                                (Some(HunkLine::Remove(r)), _) => {
                                    hunk_lines.push(HunkLine::Remove(r.clone()));
                                    oi += 1;
                                }
                                (_, Some(HunkLine::Add(a))) => {
                                    hunk_lines.push(HunkLine::Add(a.clone()));
                                    ni += 1;
                                }
                                (Some(HunkLine::Context(c)), _) => {
                                    hunk_lines.push(HunkLine::Context(c.clone()));
                                    oi += 1;
                                }
                                (_, Some(HunkLine::Context(c))) => {
                                    hunk_lines.push(HunkLine::Context(c.clone()));
                                    ni += 1;
                                }
                                _ => {
                                    oi += 1;
                                    ni += 1;
                                }
                            }
                        }

                        patch.hunks.push(Hunk {
                            old_start,
                            old_count: old_end.saturating_sub(old_start) + 1,
                            new_start,
                            new_count: new_end.saturating_sub(new_start) + 1,
                            lines: hunk_lines,
                        });
                    }
                } else if lines[i].starts_with("***") && !lines[i].contains("****") {
                    // 次のファイル
                    break;
                } else {
                    i += 1;
                }
            }

            patches.push(patch);
        } else {
            i += 1;
        }
    }

    Ok(patches)
}

fn parse_context_range(s: &str) -> Result<(usize, usize), String> {
    let parts: Vec<&str> = s.split(',').collect();
    let start: usize = parts[0]
        .parse()
        .map_err(|_| format!("無効な範囲: {}", s))?;
    let end: usize = if parts.len() > 1 {
        parts[1].parse().map_err(|_| format!("無効な範囲: {}", s))?
    } else {
        start
    };
    Ok((start, end))
}

/// 通常形式のパッチを解析
fn parse_normal_patch(content: &str, _strip_level: usize) -> Result<Vec<PatchFile>, String> {
    let mut hunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        
        // パターン: NUMcNUM, NUMaNUM, NUMdNUM, NUM,NUMcNUM,NUM など
        if let Some(cmd_pos) = line.find(|c| c == 'a' || c == 'c' || c == 'd') {
            let left = &line[..cmd_pos];
            let cmd = line.chars().nth(cmd_pos).unwrap();
            let right = &line[cmd_pos + 1..];

            let (old_start, old_end) = parse_normal_range(left)?;
            let (new_start, new_end) = parse_normal_range(right)?;

            let mut hunk_lines = Vec::new();
            i += 1;

            match cmd {
                'c' => {
                    // 変更: < 行, ---, > 行
                    while i < lines.len() && lines[i].starts_with("< ") {
                        hunk_lines.push(HunkLine::Remove(lines[i][2..].to_string()));
                        i += 1;
                    }
                    if i < lines.len() && lines[i] == "---" {
                        i += 1;
                    }
                    while i < lines.len() && lines[i].starts_with("> ") {
                        hunk_lines.push(HunkLine::Add(lines[i][2..].to_string()));
                        i += 1;
                    }
                }
                'a' => {
                    // 追加: > 行
                    while i < lines.len() && lines[i].starts_with("> ") {
                        hunk_lines.push(HunkLine::Add(lines[i][2..].to_string()));
                        i += 1;
                    }
                }
                'd' => {
                    // 削除: < 行
                    while i < lines.len() && lines[i].starts_with("< ") {
                        hunk_lines.push(HunkLine::Remove(lines[i][2..].to_string()));
                        i += 1;
                    }
                }
                _ => {}
            }

            hunks.push(Hunk {
                old_start,
                old_count: old_end - old_start + 1,
                new_start,
                new_count: new_end - new_start + 1,
                lines: hunk_lines,
            });
        } else {
            i += 1;
        }
    }

    if hunks.is_empty() {
        return Ok(Vec::new());
    }

    // 通常形式はファイル名がないので、空のファイル名で返す
    Ok(vec![PatchFile {
        old_file: String::new(),
        new_file: String::new(),
        hunks,
    }])
}

fn parse_normal_range(s: &str) -> Result<(usize, usize), String> {
    let parts: Vec<&str> = s.split(',').collect();
    let start: usize = parts[0]
        .parse()
        .map_err(|_| format!("無効な範囲: {}", s))?;
    let end: usize = if parts.len() > 1 {
        parts[1].parse().map_err(|_| format!("無効な範囲: {}", s))?
    } else {
        start
    };
    Ok((start, end))
}

/// ハンクを適用
fn apply_hunk(
    lines: &mut Vec<String>,
    hunk: &Hunk,
    reverse: bool,
    fuzz: usize,
) -> Result<bool, String> {
    let target_line = if reverse {
        hunk.new_start
    } else {
        hunk.old_start
    };

    // ファズを考慮した検索範囲
    let search_start = target_line.saturating_sub(fuzz + 1);
    let search_end = (target_line + fuzz).min(lines.len() + 1);

    // コンテキスト行を取得
    let context_lines: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|l| match l {
            HunkLine::Context(s) => Some(s.as_str()),
            HunkLine::Remove(s) if !reverse => Some(s.as_str()),
            HunkLine::Add(s) if reverse => Some(s.as_str()),
            _ => None,
        })
        .collect();

    // マッチする位置を探す
    for offset in 0..=(search_end - search_start) {
        for &dir in &[0i32, 1, -1] {
            let pos = if dir == 0 {
                target_line.saturating_sub(1)
            } else if dir > 0 {
                target_line.saturating_sub(1) + offset
            } else {
                target_line.saturating_sub(1).saturating_sub(offset)
            };

            if pos > lines.len() {
                continue;
            }

            if matches_at(lines, pos, &context_lines) {
                // ハンクを適用
                apply_at(lines, pos, hunk, reverse);
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// 指定位置でコンテキストがマッチするか
fn matches_at(lines: &[String], pos: usize, context: &[&str]) -> bool {
    if context.is_empty() {
        return true;
    }

    let mut line_idx = pos;
    for ctx in context {
        if line_idx >= lines.len() {
            return false;
        }
        if lines[line_idx].trim_end() != ctx.trim_end() {
            return false;
        }
        line_idx += 1;
    }
    true
}

/// 指定位置にハンクを適用
fn apply_at(lines: &mut Vec<String>, pos: usize, hunk: &Hunk, reverse: bool) {
    let mut new_lines = Vec::new();
    new_lines.extend_from_slice(&lines[..pos]);

    for hunk_line in &hunk.lines {
        match hunk_line {
            HunkLine::Context(s) => {
                new_lines.push(s.clone());
            }
            HunkLine::Add(s) => {
                if !reverse {
                    new_lines.push(s.clone());
                }
            }
            HunkLine::Remove(s) => {
                if reverse {
                    new_lines.push(s.clone());
                }
            }
        }
    }

    // 削除された行数を計算
    let removed_count = hunk
        .lines
        .iter()
        .filter(|l| match l {
            HunkLine::Remove(_) => !reverse,
            HunkLine::Add(_) => reverse,
            HunkLine::Context(_) => true,
        })
        .count();

    let skip = pos + removed_count;
    if skip < lines.len() {
        new_lines.extend_from_slice(&lines[skip..]);
    }

    *lines = new_lines;
}

/// パッチを適用
fn apply_patch(config: &Config) -> Result<i32, String> {
    // ディレクトリ変更
    if let Some(ref dir) = config.directory {
        env::set_current_dir(dir)
            .map_err(|e| format!("patch: {}: {}", dir, e))?;
    }

    let patch_content = read_patch(config)?;
    let format = detect_patch_format(&patch_content);

    let patches = match format {
        PatchFormat::Unified => parse_unified_patch(&patch_content, config.strip_level)?,
        PatchFormat::Context => parse_context_patch(&patch_content, config.strip_level)?,
        PatchFormat::Normal => parse_normal_patch(&patch_content, config.strip_level)?,
        PatchFormat::Ed => {
            return Err("ed形式のパッチは現在サポートされていません".to_string());
        }
    };

    if patches.is_empty() {
        if !config.silent {
            eprintln!("patch: パッチが見つかりませんでした");
        }
        return Ok(0);
    }

    let mut exit_code = 0;

    for patch in &patches {
        let target_file = if !patch.new_file.is_empty() {
            &patch.new_file
        } else if !patch.old_file.is_empty() {
            &patch.old_file
        } else if let Some(ref input) = config.input_file {
            input
        } else {
            eprintln!("patch: ターゲットファイルが指定されていません");
            return Ok(2);
        };

        let target_file = if let Some(ref input) = config.input_file {
            input.clone()
        } else {
            target_file.clone()
        };
        let resolved_target_file = resolve_existing_path(&target_file)?;

        if !config.silent {
            eprintln!("patching file {}", resolved_target_file);
        }

        let mut lines = read_lines(&resolved_target_file)?;
        let mut failed_hunks = 0;

        for (i, hunk) in patch.hunks.iter().enumerate() {
            match apply_hunk(&mut lines, hunk, config.reverse, config.fuzz) {
                Ok(true) => {
                    if config.verbose {
                        eprintln!("Hunk #{} succeeded.", i + 1);
                    }
                }
                Ok(false) => {
                    if !config.silent {
                        eprintln!("Hunk #{} FAILED.", i + 1);
                    }
                    failed_hunks += 1;
                }
                Err(e) => {
                    eprintln!("patch: {}", e);
                    failed_hunks += 1;
                }
            }
        }

        if failed_hunks > 0 {
            if !config.silent {
                eprintln!(
                    "{} out of {} hunks FAILED",
                    failed_hunks,
                    patch.hunks.len()
                );
            }
            exit_code = 1;
        }

        // 出力
        if !config.dry_run {
            let output_path = config.output_file.as_ref().unwrap_or(&resolved_target_file);
            
            // バックアップ
            if config.backup
                && Path::new(&resolved_target_file).exists()
                && config.output_file.is_none()
            {
                let backup_path = format!("{}{}", resolved_target_file, config.backup_suffix);
                fs::copy(&resolved_target_file, &backup_path)
                    .map_err(|e| format!("patch: バックアップ作成失敗: {}", e))?;
            }

            // ディレクトリを作成
            if let Some(parent) = Path::new(output_path).parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("patch: ディレクトリ作成失敗: {}", e))?;
                }
            }

            let mut file = File::create(output_path)
                .map_err(|e| format!("patch: {}: {}", output_path, e))?;

            for line in &lines {
                writeln!(file, "{}", line)
                    .map_err(|e| format!("patch: {}: {}", output_path, e))?;
            }
        }
    }

    Ok(exit_code)
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("patch: {}", e);
            eprintln!("詳しくは 'patch --help' を参照してください");
            process::exit(2);
        }
    };

    match apply_patch(&config) {
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("{}", e);
            process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
        dir.push(format!("patch-tests-{}-{}-{}", prefix, now, seq));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn expand_glob_sorts_case_insensitively_on_windows_style_usage() {
        let dir = make_temp_dir("glob");
        let first = dir.join("Beta.patch");
        let second = dir.join("alpha.patch");
        File::create(&first).unwrap();
        File::create(&second).unwrap();

        let pattern = dir.join("*.patch");
        let expanded = expand_glob(&pattern.to_string_lossy()).unwrap();

        assert_eq!(expanded.len(), 2);
        assert!(expanded[0].ends_with("alpha.patch"));
        assert!(expanded[1].ends_with("Beta.patch"));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolve_existing_path_matches_case_insensitively() {
        let dir = make_temp_dir("resolve");
        let nested = dir.join("Src");
        fs::create_dir_all(&nested).unwrap();
        let path = nested.join("File.TXT");
        fs::write(&path, "hello\n").unwrap();

        let resolved = resolve_existing_path(&dir.join("src/file.txt").to_string_lossy()).unwrap();

        assert!(
            resolved.ends_with("Src\\File.TXT") || resolved.ends_with("Src/File.TXT"),
            "resolved path: {}",
            resolved
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_args_expands_positional_globs() {
        let dir = make_temp_dir("args");
        let original_dir = env::current_dir().unwrap();
        let patch_file = dir.join("change.patch");
        let input_file = dir.join("input.txt");
        fs::write(&patch_file, "--- a\n+++ a\n").unwrap();
        fs::write(&input_file, "content\n").unwrap();

        env::set_current_dir(&dir).unwrap();
        let config = parse_args_from(vec!["*.txt".to_string(), "*.patch".to_string()]).unwrap();
        env::set_current_dir(original_dir).unwrap();

        assert_eq!(config.input_file.as_deref(), Some("input.txt"));
        assert_eq!(config.patch_file.as_deref(), Some("change.patch"));

        fs::remove_dir_all(dir).unwrap();
    }
}
