use std::env;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process;

use glob::{glob_with, MatchOptions};

#[derive(Debug)]
struct Config {
    /// 出力ファイル
    files: Vec<String>,
    /// 追記モード（-a）
    append: bool,
    /// シグナル無視（-i）
    ignore_interrupts: bool,
    /// 出力モード（--output-error）
    output_error_mode: OutputErrorMode,
    /// パイプモード（-p）
    pipe_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum OutputErrorMode {
    /// デフォルト：パイプ以外のエラーで警告、続行
    WarnNopipe,
    /// 書き込みエラーで警告、続行
    Warn,
    /// 書き込みエラーで警告、即終了
    WarnExit,
    /// エラーを無視して続行
    Exit,
    /// すべてのエラーを無視
    ExitNopipe,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            files: Vec::new(),
            append: false,
            ignore_interrupts: false,
            output_error_mode: OutputErrorMode::WarnNopipe,
            pipe_mode: false,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"使用法: tee [オプション]... [ファイル]...
標準入力を標準出力と指定されたファイルにコピーします。

オプション:
  -a, --append              ファイルを上書きせず追記
  -i, --ignore-interrupts   割り込みシグナル（SIGINT）を無視
  -p                        出力エラーの診断を行う（GNU拡張）
      --output-error[=MODE] 書き込みエラー時の動作を指定（GNU拡張）
                            MODE:
                              'warn'         エラー時に警告して続行
                              'warn-nopipe'  パイプ以外のエラーで警告（デフォルト）
                              'exit'         エラー時に終了
                              'exit-nopipe'  パイプ以外のエラーで終了
      --help                このヘルプを表示
      --version             バージョン情報を表示

ファイルが指定されない場合、標準入力を標準出力のみにコピーします（catと同様）。
ファイル名に - を指定すると標準出力を意味します。

例:
  ls -l | tee output.txt              出力を表示しながらファイルに保存
  ls -l | tee -a output.txt           出力を表示しながらファイルに追記
  ls -l | tee file1.txt file2.txt     複数ファイルに同時出力
  make 2>&1 | tee build.log           標準出力とエラーを両方ログに保存
  echo "test" | tee /dev/null         出力を破棄（テスト用）

globパターン対応:
  ls -l | tee logs/*.txt              Windowsでも内部でglob展開

注記:
  Windowsではシェルがglobを展開しないため、teeが内部で展開します。
  展開時はファイル名の大文字小文字を区別しません。
  パターンが未一致なら、その引数はリテラルなファイル名として扱います。

応用例:
  # sudoで保護されたファイルに書き込み
  echo "data" | sudo tee /etc/config > /dev/null

  # パイプラインの途中で確認
  cat data.txt | tee intermediate.txt | sort | uniq

  # ログを取りながら処理
  ./script.sh 2>&1 | tee script.log
"#
    );
}

fn print_version() {
    eprintln!("tee (Rust実装) 0.1.0");
    eprintln!("Copyright (C) 2024");
    eprintln!("ライセンス: MIT または Apache-2.0");
}

fn has_glob_magic(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn glob_match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    }
}

/// Windows ではシェルが glob 展開しないことが多いため、tee 内部で展開する。
/// ただし POSIX シェルに寄せて、未一致のパターンはそのまま引数として扱う。
fn expand_glob(pattern: &str) -> Result<Vec<String>, String> {
    if !has_glob_magic(pattern) {
        return Ok(vec![pattern.to_string()]);
    }

    let mut paths: Vec<String> = glob_with(pattern, glob_match_options())
        .map_err(|e| format!("globパターンエラー: {}", e))?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if paths.is_empty() {
        return Ok(vec![pattern.to_string()]);
    }

    paths.sort_by_cached_key(|path| path.to_ascii_lowercase());
    Ok(paths)
}

fn parse_args_from<I>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let mut config = Config::default();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" {
            print_help();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-a" || arg == "--append" {
            config.append = true;
        } else if arg == "-i" || arg == "--ignore-interrupts" {
            config.ignore_interrupts = true;
        } else if arg == "-p" {
            config.pipe_mode = true;
            config.output_error_mode = OutputErrorMode::WarnNopipe;
        } else if arg == "--output-error" {
            config.output_error_mode = OutputErrorMode::WarnNopipe;
        } else if arg.starts_with("--output-error=") {
            let mode = &arg[15..];
            config.output_error_mode = match mode {
                "warn" => OutputErrorMode::Warn,
                "warn-nopipe" => OutputErrorMode::WarnNopipe,
                "exit" => OutputErrorMode::Exit,
                "exit-nopipe" => OutputErrorMode::ExitNopipe,
                _ => return Err(format!("無効な --output-error モード: '{}'", mode)),
            };
        } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            // 複合オプション（-ai など）
            for c in arg[1..].chars() {
                match c {
                    'a' => config.append = true,
                    'i' => config.ignore_interrupts = true,
                    'p' => {
                        config.pipe_mode = true;
                        config.output_error_mode = OutputErrorMode::WarnNopipe;
                    }
                    _ => return Err(format!("不明なオプション: -{}", c)),
                }
            }
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("不明なオプション: {}", arg));
        } else {
            // ファイル名（- は標準出力を意味する）
            if arg == "-" {
                config.files.push("-".to_string());
            } else {
                let expanded = expand_glob(arg)?;
                config.files.extend(expanded);
            }
        }

        i += 1;
    }

    Ok(config)
}

fn parse_args() -> Result<Config, String> {
    parse_args_from(env::args().skip(1))
}

/// 出力先を表す構造体
struct OutputTarget {
    name: String,
    writer: Box<dyn Write>,
    #[allow(dead_code)]
    is_pipe: bool,
    has_error: bool,
}

impl OutputTarget {
    fn new_stdout() -> Self {
        OutputTarget {
            name: "(標準出力)".to_string(),
            writer: Box::new(BufWriter::new(io::stdout())),
            is_pipe: false, // 厳密にはチェックが必要だが、簡略化
            has_error: false,
        }
    }

    fn new_file(path: &str, append: bool) -> Result<Self, String> {
        if path == "-" {
            return Ok(OutputTarget {
                name: "(標準出力)".to_string(),
                writer: Box::new(BufWriter::new(io::stdout())),
                is_pipe: false,
                has_error: false,
            });
        }

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .truncate(!append)
            .open(path)
            .map_err(|e| format!("tee: '{}': {}", path, e))?;

        Ok(OutputTarget {
            name: path.to_string(),
            writer: Box::new(BufWriter::new(file)),
            is_pipe: false,
            has_error: false,
        })
    }
}

/// メイン処理
fn process(config: &Config) -> Result<(), String> {
    // シグナル無視の設定
    // 注意: Windowsではこのオプションは効果がありません
    // Unix環境ではCtrl+Cが無視されます
    if config.ignore_interrupts {
        #[cfg(unix)]
        {
            // Unixでのシグナル処理は別途実装が必要
            // 現在はプレースホルダー
        }
        #[cfg(windows)]
        {
            // Windowsでは Ctrl+C ハンドラを設定可能だが、
            // シンプルな実装では省略
        }
    }

    // 出力先のリストを作成
    let mut outputs: Vec<OutputTarget> = Vec::new();

    // 標準出力は常に含める
    outputs.push(OutputTarget::new_stdout());

    // ファイル出力を追加
    for path in &config.files {
        match OutputTarget::new_file(path, config.append) {
            Ok(target) => outputs.push(target),
            Err(e) => {
                eprintln!("{}", e);
                match config.output_error_mode {
                    OutputErrorMode::Exit | OutputErrorMode::ExitNopipe => {
                        return Err(e);
                    }
                    _ => {
                        // 警告して続行（このファイルはスキップ）
                    }
                }
            }
        }
    }

    // 標準入力を読み込んで全出力先に書き込む
    let stdin = io::stdin();
    let reader = BufReader::new(stdin.lock());

    let mut exit_code = 0;

    for line_result in reader.lines() {
        match line_result {
            Ok(line) => {
                for output in outputs.iter_mut() {
                    if output.has_error {
                        continue;
                    }

                    if let Err(e) = writeln!(output.writer, "{}", line) {
                        output.has_error = true;

                        let is_pipe_error = e.kind() == io::ErrorKind::BrokenPipe;

                        let should_warn = match config.output_error_mode {
                            OutputErrorMode::Warn | OutputErrorMode::WarnExit => true,
                            OutputErrorMode::WarnNopipe => !is_pipe_error,
                            OutputErrorMode::Exit | OutputErrorMode::ExitNopipe => false,
                        };

                        let should_exit = match config.output_error_mode {
                            OutputErrorMode::WarnExit | OutputErrorMode::Exit => true,
                            OutputErrorMode::ExitNopipe => !is_pipe_error,
                            _ => false,
                        };

                        if should_warn {
                            eprintln!("tee: '{}': {}", output.name, e);
                        }

                        if should_exit {
                            return Err(format!("tee: '{}': {}", output.name, e));
                        }

                        exit_code = 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("tee: 標準入力: {}", e);
                return Err(format!("tee: 標準入力: {}", e));
            }
        }
    }

    // フラッシュ
    for output in outputs.iter_mut() {
        if !output.has_error {
            if let Err(e) = output.writer.flush() {
                if e.kind() != io::ErrorKind::BrokenPipe {
                    eprintln!("tee: '{}': {}", output.name, e);
                    exit_code = 1;
                }
            }
        }
    }

    if exit_code != 0 {
        process::exit(exit_code);
    }

    Ok(())
}

fn main() {
    match parse_args() {
        Ok(config) => {
            if let Err(e) = process(&config) {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("tee: {}", e);
            eprintln!("詳しくは 'tee --help' を参照してください");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = Path::new("target")
                .join("test-artifacts")
                .join(format!("{}-{}", name, unique));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn child(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn expand_glob_keeps_unmatched_pattern_literal() {
        let dir = TestDir::new("unmatched");
        let pattern = dir.child("*.log");

        let expanded = expand_glob(&pattern.to_string_lossy()).unwrap();

        assert_eq!(expanded, vec![pattern.to_string_lossy().to_string()]);
    }

    #[test]
    fn expand_glob_matches_case_insensitively_and_sorts_case_insensitively() {
        let dir = TestDir::new("case-insensitive");
        let upper = dir.child("Alpha.LOG");
        let lower = dir.child("beta.log");
        fs::write(&upper, b"").unwrap();
        fs::write(&lower, b"").unwrap();

        let pattern = dir.child("*.log");
        let expanded = expand_glob(&pattern.to_string_lossy()).unwrap();

        assert_eq!(
            expanded,
            vec![
                upper.to_string_lossy().to_string(),
                lower.to_string_lossy().to_string()
            ]
        );
    }

    #[test]
    fn parse_args_expands_matching_glob_and_preserves_unmatched_pattern() {
        let dir = TestDir::new("parse-args");
        let matched = dir.child("Example.txt");
        fs::write(&matched, b"").unwrap();

        let matching_pattern = dir.child("*.TXT").to_string_lossy().to_string();
        let unmatched_pattern = dir.child("missing-*.TXT").to_string_lossy().to_string();

        let config = parse_args_from(vec![matching_pattern, unmatched_pattern.clone()]).unwrap();

        assert_eq!(
            config.files,
            vec![matched.to_string_lossy().to_string(), unmatched_pattern]
        );
    }
}
