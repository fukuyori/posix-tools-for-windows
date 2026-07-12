use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

#[cfg(unix)]
use libc;

use crate::expression::*;
use crate::messages;
use crate::platform;

/// 出力先の管理
pub struct OutputManager {
    files: Mutex<HashMap<String, BufWriter<File>>>,
}

impl OutputManager {
    pub fn new() -> Self {
        OutputManager {
            files: Mutex::new(HashMap::new()),
        }
    }

    pub fn write(&self, filename: &str, content: &str) -> io::Result<()> {
        let mut files = self.files.lock().unwrap();

        if !files.contains_key(filename) {
            let file = File::create(filename)?;
            files.insert(filename.to_string(), BufWriter::new(file));
        }

        if let Some(writer) = files.get_mut(filename) {
            writer.write_all(content.as_bytes())?;
        }

        Ok(())
    }

    pub fn flush_all(&self) {
        let mut files = self.files.lock().unwrap();
        for writer in files.values_mut() {
            let _ = writer.flush();
        }
    }
}

/// バッチ実行用の1エントリ（コマンドテンプレート＋実行ディレクトリ＋ファイルリスト）
struct BatchEntry {
    template: Vec<String>,
    files: Vec<String>,
    /// -execdir の場合に設定するカレントディレクトリ
    dir: Option<PathBuf>,
    /// テンプレート固定部分のコマンドライン長（文字数換算）
    template_cmdline_len: usize,
    /// files に積まれたコマンドライン長の合計
    files_cmdline_len: usize,
}

/// コマンドライン長の上限を返す。
///
/// * Unix: `sysconf(_SC_ARG_MAX)` から環境変数オーバーヘッド分を引いた値
/// * Windows: CreateProcess のコマンドライン上限は 32,767 UTF-16 コード単位。
///            安全マージンとして 32,000 文字を使用する。
///            実際の制限は引数文字列を空白+引用符でエスケープした後の長さで決まるが、
///            フルパスを展開した後のバイト数で保守的に判定する。
fn get_cmdline_limit() -> usize {
    #[cfg(unix)]
    {
        let raw = unsafe { libc::sysconf(libc::_SC_ARG_MAX) };
        if raw > 0 {
            // 環境変数領域（推定 2 KiB）+ ヘッダ分のマージン
            let margin = 2 * 1024 + 512;
            return (raw as usize).saturating_sub(margin).max(4096);
        }
    }
    // Windows: 32,767 UTF-16 code units が上限。安全マージンを取り 32,000 文字とする。
    // Unix で sysconf が失敗した場合も同値にフォールバックする。
    32_000
}

/// 引数1個がコマンドライン上で占める「文字数換算長」を返す。
///
/// Windows の `CreateProcess` はコマンドライン全体を1つの UTF-16 文字列として受け取る。
/// 各引数は空白で区切られ、スペース・引用符・バックスラッシュを含む場合は
/// `"..."` でエスケープされるため、実際の長さは生文字列より長くなる。
///
/// 計算式:
///   UTF-16 コード単位数          — 日本語等のマルチバイト文字を正確に換算
///   + 1                          — 引数間の空白区切り
///   + 2                          — 最悪ケースの引用符ペア `"`
///
/// スペース・引用符・バックスラッシュを含む場合のさらなるエスケープは
/// Rust の `Command` が内部で処理するため、ここでは 2 文字のマージンで
/// 十分保守的な見積もりとなる。
#[inline]
fn cmdline_len_of(s: &str) -> usize {
    let utf16_len: usize = s.chars().map(|c| c.len_utf16()).sum();
    utf16_len
        + 1  // 引数間の空白
        + 2 // 引用符ペアのマージン（スペースや特殊文字を含む場合）
}

/// バッチ実行用のコマンドバッファ
/// キー = テンプレート'\0'結合 + '\x01' + ディレクトリパス
pub struct BatchExecutor {
    commands: HashMap<String, BatchEntry>,
    /// コマンドライン長の上限（UTF-16 コード単位数）
    cmdline_limit: usize,
    /// いずれかのバッチ実行が失敗（起動失敗 or 非ゼロ終了）したか。
    /// GNU find と同様、`-exec {} +` の失敗は find の終了コードに反映する。
    failed: bool,
}

impl BatchExecutor {
    pub fn new() -> Self {
        BatchExecutor {
            commands: HashMap::new(),
            cmdline_limit: get_cmdline_limit(),
            failed: false,
        }
    }

    /// これまでのバッチ実行で失敗があったか
    pub fn had_failure(&self) -> bool {
        self.failed
    }

    /// `dir` は -execdir 時のみ Some を渡す。
    pub fn add(&mut self, command_template: &[String], file: &str, dir: Option<&Path>) {
        let dir_str = dir
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let key = format!("{}\x01{}", command_template.join("\0"), dir_str);

        // テンプレート固定部分のコマンドライン長（初回のみ計算）
        let template_cmdline_len: usize = command_template.iter().map(|s| cmdline_len_of(s)).sum();

        let file_cl = cmdline_len_of(file);

        // ❗4 修正: 「追加前に上限チェック」する。
        //
        // 旧ロジック: 追加 → 判定 → 超えたらフラッシュ（1件超過した状態で実行）
        // 新ロジック: 判定 → 超えるなら先にフラッシュ → 新バッチに追加
        //
        // これにより「フラッシュ後の新バッチ」は必ずコマンドライン上限内に収まる。
        {
            let entry = self
                .commands
                .entry(key.clone())
                .or_insert_with(|| BatchEntry {
                    template: command_template.to_vec(),
                    files: Vec::new(),
                    dir: dir.map(|d| d.to_path_buf()),
                    template_cmdline_len,
                    files_cmdline_len: 0,
                });

            // 追加後の見積もりが上限を超える場合、先にフラッシュする
            let projected = entry.template_cmdline_len + entry.files_cmdline_len + file_cl;
            if !entry.files.is_empty() && projected >= self.cmdline_limit {
                // 現在の entry をフラッシュし、キーを削除
                if let Some(mut e) = self.commands.remove(&key) {
                    if !Self::run_batch(&mut e) {
                        self.failed = true;
                    }
                }
                // 削除後に新エントリを挿入
                self.commands.insert(
                    key.clone(),
                    BatchEntry {
                        template: command_template.to_vec(),
                        files: Vec::new(),
                        dir: dir.map(|d| d.to_path_buf()),
                        template_cmdline_len,
                        files_cmdline_len: 0,
                    },
                );
            }
        }

        // 新しい or 空になったエントリにファイルを追加
        let entry = self.commands.get_mut(&key).unwrap();
        entry.files.push(file.to_string());
        entry.files_cmdline_len += file_cl;
    }

    /// バッチを実行し、成功したかどうかを返す。
    fn run_batch(entry: &mut BatchEntry) -> bool {
        if entry.files.is_empty() {
            return true;
        }

        let mut cmd_parts: Vec<String> = Vec::new();
        let mut found_placeholder = false;

        for part in &entry.template {
            if part == "{}" {
                // {} 単独トークン: ファイル群をそのまま展開
                found_placeholder = true;
                cmd_parts.extend(entry.files.iter().cloned());
            } else {
                cmd_parts.push(part.clone());
            }
        }

        if !found_placeholder {
            cmd_parts.extend(entry.files.iter().cloned());
        }

        if cmd_parts.is_empty() {
            return true;
        }
        let mut cmd = Command::new(&cmd_parts[0]);
        cmd.args(&cmd_parts[1..]);
        if let Some(ref d) = entry.dir {
            cmd.current_dir(d);
        }
        match cmd.status() {
            Ok(status) => status.success(),
            Err(e) => {
                eprintln!("{}", messages::err_exec_failed(&cmd_parts[0], &e.to_string()));
                false
            }
        }
    }

    pub fn flush(&mut self) {
        for entry in self.commands.values_mut() {
            if !Self::run_batch(entry) {
                self.failed = true;
            }
        }
        self.commands.clear();
    }
}

/// アクションコンテキスト
pub struct ActionContext<'a> {
    pub path: &'a Path,
    /// メタデータ（遅延取得済みの場合のみ Some）
    pub metadata: Option<&'a std::fs::Metadata>,
    /// シンボリックリンクのメタデータ（非 symlink の場合は None）
    pub symlink_metadata: Option<&'a std::fs::Metadata>,
    pub start_path: &'a Path,
    pub depth: usize,
    pub output_manager: &'a OutputManager,
    pub batch_executor: &'a mut BatchExecutor,
    /// find 全体の終了コード。アクションの失敗（-exec 起動失敗、-delete 失敗など）を
    /// 致命的エラーにせず記録するために使う。
    pub exit_code: &'a mut i32,
}

impl<'a> ActionContext<'a> {
    /// `metadata()` を返す。
    #[inline]
    pub fn metadata(&self) -> Option<&std::fs::Metadata> {
        self.metadata
    }

    #[inline]
    pub fn symlink_metadata(&self) -> Option<&std::fs::Metadata> {
        self.symlink_metadata
    }
}

/// アクションの実行結果
pub enum ActionResult {
    Continue,
    /// -exec cmd \; のコマンドが非ゼロ終了した場合（述語として false）
    False,
    Prune,
    Quit,
}

impl Action {
    pub fn execute(&self, ctx: &mut ActionContext) -> Result<ActionResult, String> {
        match self {
            Action::Print => {
                println!("{}", ctx.path.display());
                Ok(ActionResult::Continue)
            }

            Action::Print0 => {
                print!("{}\0", ctx.path.display());
                let _ = io::stdout().flush();
                Ok(ActionResult::Continue)
            }

            Action::FPrint(file) => {
                ctx.output_manager
                    .write(file, &format!("{}\n", ctx.path.display()))
                    .map_err(|e| messages::err_cannot_open_file(file, &e.to_string()))?;
                Ok(ActionResult::Continue)
            }

            Action::FPrint0(file) => {
                ctx.output_manager
                    .write(file, &format!("{}\0", ctx.path.display()))
                    .map_err(|e| messages::err_cannot_open_file(file, &e.to_string()))?;
                Ok(ActionResult::Continue)
            }

            Action::Printf(format) => {
                let output = format_printf(format, ctx);
                print!("{}", output);
                let _ = io::stdout().flush();
                Ok(ActionResult::Continue)
            }

            Action::FPrintf(file, format) => {
                let output = format_printf(format, ctx);
                ctx.output_manager
                    .write(file, &output)
                    .map_err(|e| messages::err_cannot_open_file(file, &e.to_string()))?;
                Ok(ActionResult::Continue)
            }

            Action::Ls => {
                let output = format_ls(ctx);
                println!("{}", output);
                Ok(ActionResult::Continue)
            }

            Action::FLs(file) => {
                let output = format_ls(ctx);
                ctx.output_manager
                    .write(file, &format!("{}\n", output))
                    .map_err(|e| messages::err_cannot_open_file(file, &e.to_string()))?;
                Ok(ActionResult::Continue)
            }

            Action::Exec {
                command,
                exec_type,
                in_dir,
            } => {
                let (file_arg, dir) = exec_target(ctx, *in_dir);

                match exec_type {
                    ExecType::Each => {
                        // -exec の終了コードは述語の真偽値として機能する。
                        // コマンドの起動失敗は致命的エラーにせず、終了コードに記録して続行する。
                        match execute_command(command, &file_arg, dir.as_deref()) {
                            Ok(true) => {}
                            Ok(false) => return Ok(ActionResult::False),
                            Err(e) => {
                                eprintln!("{}", e);
                                *ctx.exit_code = 1;
                                return Ok(ActionResult::False);
                            }
                        }
                    }
                    ExecType::Batch => {
                        // -exec {} + は常に true。ディレクトリも渡す（-execdir 対応）
                        ctx.batch_executor.add(command, &file_arg, dir.as_deref());
                    }
                }
                Ok(ActionResult::Continue)
            }

            Action::Ok { command, in_dir } => {
                let (file_arg, dir) = exec_target(ctx, *in_dir);

                let cmd_str = command.join(" ");
                eprint!("{}", messages::prompt_exec(&cmd_str, &file_arg));
                let _ = io::stderr().flush();

                let mut response = String::new();
                if io::stdin().read_line(&mut response).is_ok() {
                    let response = response.trim().to_lowercase();
                    if response == "y" || response == "yes" {
                        match execute_command(command, &file_arg, dir.as_deref()) {
                            Ok(true) => {}
                            Ok(false) => return Ok(ActionResult::False),
                            Err(e) => {
                                eprintln!("{}", e);
                                *ctx.exit_code = 1;
                                return Ok(ActionResult::False);
                            }
                        }
                    } else {
                        // ユーザーが拒否した場合も false
                        return Ok(ActionResult::False);
                    }
                }
                Ok(ActionResult::Continue)
            }

            Action::Delete => {
                let path = ctx.path;

                // GNU find と同様、"." の削除は拒否する
                if path == Path::new(".") {
                    eprintln!("{}", messages::err_delete_current_dir());
                    *ctx.exit_code = 1;
                    return Ok(ActionResult::False);
                }

                let result = if ctx.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                    fs::remove_dir(path)
                } else {
                    fs::remove_file(path)
                };

                // 削除失敗は警告して続行（GNU 互換）
                if let Err(e) = result {
                    eprintln!(
                        "{}",
                        messages::err_delete_failed(&path.display().to_string(), &e.to_string())
                    );
                    *ctx.exit_code = 1;
                    return Ok(ActionResult::False);
                }

                Ok(ActionResult::Continue)
            }

            Action::Prune => Ok(ActionResult::Prune),

            Action::Quit => Ok(ActionResult::Quit),
        }
    }
}

/// -exec / -ok 系のコマンドに渡すファイル引数と実行ディレクトリを決める。
/// -execdir / -okdir では GNU find と同様、ファイル名に `./` を前置して
/// `-` で始まるファイル名がオプションと誤解釈されるのを防ぐ。
fn exec_target(ctx: &ActionContext, in_dir: bool) -> (String, Option<PathBuf>) {
    if !in_dir {
        return (ctx.path.to_string_lossy().into_owned(), None);
    }

    let dir = ctx.path.parent().and_then(|p| {
        if p.as_os_str().is_empty() {
            None
        } else {
            Some(p.to_path_buf())
        }
    });
    let file_arg = match ctx.path.file_name() {
        Some(name) => format!("./{}", name.to_string_lossy()),
        None => ctx.path.to_string_lossy().into_owned(),
    };
    (file_arg, dir)
}

fn execute_command(command: &[String], file: &str, dir: Option<&Path>) -> Result<bool, String> {
    if command.is_empty() {
        return Ok(true);
    }

    let args: Vec<String> = command
        .iter()
        .map(|arg| {
            if arg == "{}" {
                file.to_string()
            } else if arg.contains("{}") {
                arg.replace("{}", file)
            } else {
                arg.clone()
            }
        })
        .collect();

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);

    if let Some(d) = dir {
        cmd.current_dir(d);
    }

    let status = cmd
        .status()
        .map_err(|e| messages::err_exec_failed(&args[0], &e.to_string()))?;

    Ok(status.success())
}

fn format_printf(format: &str, ctx: &ActionContext) -> String {
    // -printf はメタデータを参照するフォーマット指定子を含む可能性がある。
    // 取得できない場合でも可能な限り出力する（メタデータが必要な項目は "?" で代替）。
    let mut result = String::new();
    let mut chars = format.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('a') => result.push('\x07'),
                Some('b') => result.push('\x08'),
                Some('f') => result.push('\x0C'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('v') => result.push('\x0B'),
                Some('\\') => result.push('\\'),
                // \c: この時点で出力を打ち切る（GNU 互換）
                Some('c') => return result,
                // \NNN: 8進数エスケープ（1〜3桁）
                Some(d) if d.is_digit(8) => {
                    let mut val = d.to_digit(8).unwrap();
                    for _ in 0..2 {
                        match chars.peek() {
                            Some(&n) if n.is_digit(8) => {
                                val = val * 8 + n.to_digit(8).unwrap();
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    result.push(char::from_u32(val).unwrap_or('\0'));
                }
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else if c == '%' {
            // printf 風のフラグ・フィールド幅（例: %-8s, %5d）
            let mut left_align = false;
            let mut zero_pad = false;
            while let Some(&f) = chars.peek() {
                match f {
                    '-' => left_align = true,
                    '0' => zero_pad = true,
                    '+' | ' ' | '#' => {}
                    _ => break,
                }
                chars.next();
            }
            let mut width: usize = 0;
            while let Some(&d) = chars.peek() {
                if let Some(v) = d.to_digit(10) {
                    width = width * 10 + v as usize;
                    chars.next();
                } else {
                    break;
                }
            }
            // 精度指定は受理して無視する
            if chars.peek() == Some(&'.') {
                chars.next();
                while chars.peek().map_or(false, |d| d.is_ascii_digit()) {
                    chars.next();
                }
            }

            let Some(dir) = chars.next() else {
                result.push('%');
                break;
            };

            match printf_directive_value(dir, &mut chars, ctx) {
                Some(value) => {
                    let len = value.chars().count();
                    if width > len {
                        let pad = width - len;
                        if left_align {
                            result.push_str(&value);
                            result.extend(std::iter::repeat(' ').take(pad));
                        } else {
                            let pad_char = if zero_pad { '0' } else { ' ' };
                            result.extend(std::iter::repeat(pad_char).take(pad));
                            result.push_str(&value);
                        }
                    } else {
                        result.push_str(&value);
                    }
                }
                None => {
                    // 未知の指定子はリテラルとして出力
                    result.push('%');
                    result.push(dir);
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// %ディレクティブ1個分の値を返す。未知の指定子は None。
/// %A/%B/%C/%T はフォーマット文字を追加で消費する。
fn printf_directive_value(
    dir: char,
    chars: &mut std::iter::Peekable<std::str::Chars>,
    ctx: &ActionContext,
) -> Option<String> {
    let meta = ctx.metadata();

    let value = match dir {
        '%' => "%".to_string(),
        'p' => ctx.path.display().to_string(),
        'f' => ctx
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        'h' => ctx
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".to_string()),
        'P' => match ctx.path.strip_prefix(ctx.start_path) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => ctx.path.display().to_string(),
        },
        'H' => ctx.start_path.display().to_string(),
        'd' => ctx.depth.to_string(),
        's' => meta.map(|m| m.len()).unwrap_or(0).to_string(),
        'k' => {
            // 1KiB ブロック数（512B ブロック数から換算、切り上げ）
            let blocks = meta.map(platform::get_blocks).unwrap_or(0);
            ((blocks + 1) / 2).to_string()
        }
        'b' => meta.map(platform::get_blocks).unwrap_or(0).to_string(),
        'S' => {
            // スパース度: blocks*512 / size
            match meta {
                Some(m) => {
                    let size = m.len();
                    if size == 0 {
                        "1.0".to_string()
                    } else {
                        let sparseness = (platform::get_blocks(m) * 512) as f64 / size as f64;
                        format!("{:.1}", sparseness)
                    }
                }
                None => "?".to_string(),
            }
        }
        'm' => meta
            .map(|m| format!("{:o}", platform::get_mode(m) & 0o7777))
            .unwrap_or_else(|| "?".to_string()),
        'M' => meta
            .map(|m| platform::format_mode_symbolic(platform::get_mode(m), m))
            .unwrap_or_else(|| "??????????".to_string()),
        'u' => meta
            .map(|m| {
                let uid = platform::get_uid(m);
                platform::get_user_name(uid).unwrap_or_else(|| uid.to_string())
            })
            .unwrap_or_else(|| "?".to_string()),
        'U' => meta
            .map(|m| platform::get_uid(m).to_string())
            .unwrap_or_else(|| "?".to_string()),
        'g' => meta
            .map(|m| {
                let gid = platform::get_gid(m);
                platform::get_group_name(gid).unwrap_or_else(|| gid.to_string())
            })
            .unwrap_or_else(|| "?".to_string()),
        'G' => meta
            .map(|m| platform::get_gid(m).to_string())
            .unwrap_or_else(|| "?".to_string()),
        'l' => fs::read_link(ctx.path)
            .map(|t| t.display().to_string())
            .unwrap_or_default(),
        'i' => action_file_ids(ctx)
            .map(|(_, ino, _)| ino.to_string())
            .unwrap_or_else(|| "?".to_string()),
        'n' => action_file_ids(ctx)
            .map(|(_, _, nlink)| nlink.to_string())
            .unwrap_or_else(|| "?".to_string()),
        'D' => action_file_ids(ctx)
            .map(|(dev, _, _)| dev.to_string())
            .unwrap_or_else(|| "?".to_string()),
        'F' => platform::get_fstype(ctx.path).unwrap_or_default(),
        'y' => {
            let is_symlink = ctx
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            match ctx.symlink_metadata().or(meta) {
                Some(m) => platform::get_file_type_char(m, is_symlink).to_string(),
                None => "?".to_string(),
            }
        }
        'Y' => {
            // リンク先を辿ったタイプ。リンク切れは 'N'（GNU 互換）
            let is_symlink = ctx
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if is_symlink {
                match fs::metadata(ctx.path) {
                    Ok(m) => platform::get_file_type_char(&m, false).to_string(),
                    Err(_) => "N".to_string(),
                }
            } else {
                match meta {
                    Some(m) => platform::get_file_type_char(m, false).to_string(),
                    None => "?".to_string(),
                }
            }
        }
        'a' => match meta.and_then(|m| m.accessed().ok()) {
            Some(t) => format_time_default(t),
            None => "?".to_string(),
        },
        'c' => match meta {
            Some(m) => format_time_default(platform::get_ctime(m)),
            None => "?".to_string(),
        },
        't' => match meta.and_then(|m| m.modified().ok()) {
            Some(t) => format_time_default(t),
            None => "?".to_string(),
        },
        'A' | 'B' | 'C' | 'T' => {
            let fmt_char = chars.next()?;
            let time = match dir {
                'A' => meta.and_then(|m| m.accessed().ok()),
                'B' => meta.and_then(platform::get_btime),
                'C' => meta.map(platform::get_ctime),
                _ => meta.and_then(|m| m.modified().ok()),
            };
            match time {
                Some(t) => format_time_strftime(t, fmt_char),
                None => "?".to_string(),
            }
        }
        _ => return None,
    };

    Some(value)
}

fn format_time_default(time: SystemTime) -> String {
    use chrono::{DateTime, Local};

    let datetime: DateTime<Local> = time.into();
    datetime.format("%a %b %e %H:%M:%S %Y").to_string()
}

fn format_time_strftime(time: SystemTime, fmt_char: char) -> String {
    use chrono::{DateTime, Datelike, Local, Timelike};

    let datetime: DateTime<Local> = time.into();

    match fmt_char {
        // エポック秒（小数部付き、GNU 互換）
        '@' => match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => format!("{}.{:09}", d.as_secs(), d.subsec_nanos()),
            Err(_) => datetime.timestamp().to_string(),
        },
        // ISO 風の日付+時刻（GNU の %T+ 互換）
        '+' => datetime.format("%Y-%m-%d+%H:%M:%S").to_string(),
        'H' => format!("{:02}", datetime.hour()),
        'I' => format!("{:02}", (datetime.hour() % 12).max(1)),
        'k' => format!("{:2}", datetime.hour()),
        'l' => format!("{:2}", (datetime.hour() % 12).max(1)),
        'M' => format!("{:02}", datetime.minute()),
        'p' => if datetime.hour() < 12 { "AM" } else { "PM" }.to_string(),
        'P' => if datetime.hour() < 12 { "am" } else { "pm" }.to_string(),
        'r' => datetime.format("%I:%M:%S %p").to_string(),
        'S' => format!("{:02}", datetime.second()),
        'T' | 'X' => datetime.format("%H:%M:%S").to_string(),
        'Z' => datetime.format("%Z").to_string(),
        'a' => datetime.format("%a").to_string(),
        'A' => datetime.format("%A").to_string(),
        'b' | 'h' => datetime.format("%b").to_string(),
        'B' => datetime.format("%B").to_string(),
        'c' => datetime.format("%c").to_string(),
        'd' => format!("{:02}", datetime.day()),
        'D' => datetime.format("%m/%d/%y").to_string(),
        'j' => format!("{:03}", datetime.ordinal()),
        'm' => format!("{:02}", datetime.month()),
        'U' => format!("{:02}", datetime.iso_week().week()),
        'w' => datetime.weekday().num_days_from_sunday().to_string(),
        'W' => format!("{:02}", datetime.iso_week().week()),
        'x' => datetime.format("%x").to_string(),
        'y' => format!("{:02}", datetime.year() % 100),
        'Y' => datetime.year().to_string(),
        _ => format!("%{}", fmt_char),
    }
}

/// (デバイス番号, inode 番号, ハードリンク数) を返す。
/// Unix は取得済みメタデータから、Windows はハンドル問い合わせで取得する。
fn action_file_ids(ctx: &ActionContext) -> Option<(u64, u64, u64)> {
    #[cfg(windows)]
    {
        // シンボリックリンクの場合はリンク自体の情報（lstat 相当）
        platform::get_file_ids(ctx.path, ctx.symlink_metadata().is_none())
    }

    #[cfg(not(windows))]
    {
        ctx.metadata().map(|m| {
            (
                platform::get_dev(m),
                platform::get_ino(m),
                platform::get_nlink(m),
            )
        })
    }
}

fn format_ls(ctx: &ActionContext) -> String {
    // -ls はメタデータを必要とするアクション。取得できない場合は空文字を返す。
    let Some(meta) = ctx.metadata() else {
        return format!("{}", ctx.path.display());
    };
    let (_, ino, nlink) = action_file_ids(ctx).unwrap_or((0, 0, 1));
    let blocks = (meta.len() + 511) / 512;
    let mode_str = platform::format_mode_symbolic(platform::get_mode(meta), meta);

    let user = platform::get_user_name(platform::get_uid(meta))
        .unwrap_or_else(|| platform::get_uid(meta).to_string());

    let group = platform::get_group_name(platform::get_gid(meta))
        .unwrap_or_else(|| platform::get_gid(meta).to_string());

    let size = meta.len();

    let mtime_str = meta
        .modified()
        .map(|t| format_time_ls(t))
        .unwrap_or_else(|_| "            ".to_string());

    let path_str = ctx.path.display().to_string();

    let link_target = if ctx
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        fs::read_link(ctx.path)
            .map(|t| format!(" -> {}", t.display()))
            .unwrap_or_default()
    } else {
        String::new()
    };

    format!(
        "{:7} {:4} {} {:3} {:8} {:8} {:8} {} {}{}",
        ino, blocks, mode_str, nlink, user, group, size, mtime_str, path_str, link_target
    )
}

fn format_time_ls(time: SystemTime) -> String {
    use chrono::{DateTime, Local};

    let datetime: DateTime<Local> = time.into();
    let now: DateTime<Local> = Local::now();

    let six_months_ago = now - chrono::Duration::days(180);

    if datetime > six_months_ago {
        datetime.format("%b %e %H:%M").to_string()
    } else {
        datetime.format("%b %e  %Y").to_string()
    }
}
