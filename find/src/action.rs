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
}

impl BatchExecutor {
    pub fn new() -> Self {
        BatchExecutor {
            commands: HashMap::new(),
            cmdline_limit: get_cmdline_limit(),
        }
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
                    Self::run_batch(&mut e);
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

    fn run_batch(entry: &mut BatchEntry) {
        if entry.files.is_empty() {
            return;
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

        if !cmd_parts.is_empty() {
            let mut cmd = Command::new(&cmd_parts[0]);
            cmd.args(&cmd_parts[1..]);
            if let Some(ref d) = entry.dir {
                cmd.current_dir(d);
            }
            let _ = cmd.status();
        }
    }

    pub fn flush(&mut self) {
        for entry in self.commands.values_mut() {
            Self::run_batch(entry);
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
                let output = format_printf(format, ctx)?;
                print!("{}", output);
                let _ = io::stdout().flush();
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
                let path_str = ctx.path.to_string_lossy().to_string();
                let dir = if *in_dir {
                    ctx.path.parent().map(|p| p.to_path_buf())
                } else {
                    None
                };

                let file_arg = if *in_dir {
                    ctx.path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or(path_str.clone())
                } else {
                    path_str.clone()
                };

                match exec_type {
                    ExecType::Each => {
                        // -exec の終了コードは述語の真偽値として機能する
                        let success = execute_command(command, &file_arg, dir.as_deref())?;
                        if !success {
                            return Ok(ActionResult::False);
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
                let path_str = ctx.path.to_string_lossy().to_string();
                let dir = if *in_dir {
                    ctx.path.parent().map(|p| p.to_path_buf())
                } else {
                    None
                };

                let file_arg = if *in_dir {
                    ctx.path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or(path_str.clone())
                } else {
                    path_str.clone()
                };

                let cmd_str = command.join(" ");
                eprint!("{}", messages::prompt_exec(&cmd_str, &file_arg));
                let _ = io::stderr().flush();

                let mut response = String::new();
                if io::stdin().read_line(&mut response).is_ok() {
                    let response = response.trim().to_lowercase();
                    if response == "y" || response == "yes" {
                        let success = execute_command(command, &file_arg, dir.as_deref())?;
                        if !success {
                            return Ok(ActionResult::False);
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
                let result = if ctx.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                    fs::remove_dir(path)
                } else {
                    fs::remove_file(path)
                };

                result.map_err(|e| {
                    messages::err_delete_failed(&path.display().to_string(), &e.to_string())
                })?;

                Ok(ActionResult::Continue)
            }

            Action::Prune => Ok(ActionResult::Prune),

            Action::Quit => Ok(ActionResult::Quit),
        }
    }
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

fn format_printf(format: &str, ctx: &ActionContext) -> Result<String, String> {
    // -printf はメタデータを参照するフォーマット指定子を含む可能性がある。
    // 取得できない場合でも可能な限り出力する（メタデータが必要な項目は "?" で代替）。
    let meta_opt = ctx.metadata();
    let mut result = String::new();
    let mut chars = format.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('0') => result.push('\0'),
                Some('\\') => result.push('\\'),
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else if c == '%' {
            match chars.next() {
                Some('%') => result.push('%'),
                Some('p') => result.push_str(&ctx.path.display().to_string()),
                Some('f') => {
                    if let Some(name) = ctx.path.file_name() {
                        result.push_str(&name.to_string_lossy());
                    }
                }
                Some('h') => {
                    if let Some(parent) = ctx.path.parent() {
                        result.push_str(&parent.display().to_string());
                    } else {
                        result.push('.');
                    }
                }
                Some('P') => {
                    if let Ok(rel) = ctx.path.strip_prefix(ctx.start_path) {
                        result.push_str(&rel.display().to_string());
                    } else {
                        result.push_str(&ctx.path.display().to_string());
                    }
                }
                Some('H') => result.push_str(&ctx.start_path.display().to_string()),
                Some('d') => result.push_str(&ctx.depth.to_string()),
                Some('s') => {
                    result.push_str(&ctx.metadata().map(|m| m.len()).unwrap_or(0).to_string())
                }
                Some('k') => result.push_str(
                    &((ctx.metadata().map(|m| m.len()).unwrap_or(0) + 1023) / 1024).to_string(),
                ),
                Some('b') => result.push_str(
                    &((ctx.metadata().map(|m| m.len()).unwrap_or(0) + 511) / 512).to_string(),
                ),
                Some('m') => result.push_str(&format!(
                    "{:o}",
                    platform::get_mode(meta_opt.unwrap()) & 0o7777
                )),
                Some('M') => result.push_str(&platform::format_mode_symbolic(
                    platform::get_mode(meta_opt.unwrap()),
                    meta_opt.unwrap(),
                )),
                Some('u') => {
                    let uid = platform::get_uid(meta_opt.unwrap());
                    if let Some(name) = platform::get_user_name(uid) {
                        result.push_str(&name);
                    } else {
                        result.push_str(&uid.to_string());
                    }
                }
                Some('U') => result.push_str(&platform::get_uid(meta_opt.unwrap()).to_string()),
                Some('g') => {
                    let gid = platform::get_gid(meta_opt.unwrap());
                    if let Some(name) = platform::get_group_name(gid) {
                        result.push_str(&name);
                    } else {
                        result.push_str(&gid.to_string());
                    }
                }
                Some('G') => result.push_str(&platform::get_gid(meta_opt.unwrap()).to_string()),
                Some('l') => {
                    if let Ok(target) = fs::read_link(ctx.path) {
                        result.push_str(&target.display().to_string());
                    }
                }
                Some('i') => result.push_str(&platform::get_ino(meta_opt.unwrap()).to_string()),
                Some('n') => result.push_str(&platform::get_nlink(meta_opt.unwrap()).to_string()),
                Some('y') => {
                    let is_symlink = ctx
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                    result.push(platform::get_file_type_char(
                        ctx.symlink_metadata()
                            .unwrap_or_else(|| ctx.metadata().unwrap()),
                        is_symlink,
                    ));
                }
                Some('Y') => {
                    result.push(platform::get_file_type_char(meta_opt.unwrap(), false));
                }
                Some('a') => {
                    if let Ok(atime) = ctx.metadata().and_then(|m| m.accessed().ok()).ok_or(()) {
                        result.push_str(&format_time_default(atime));
                    }
                }
                Some('A') => {
                    if let Some(fmt_char) = chars.next() {
                        if let Ok(atime) = ctx.metadata().and_then(|m| m.accessed().ok()).ok_or(())
                        {
                            result.push_str(&format_time_strftime(atime, fmt_char));
                        }
                    }
                }
                Some('c') => {
                    let ctime = platform::get_ctime(meta_opt.unwrap());
                    result.push_str(&format_time_default(ctime));
                }
                Some('C') => {
                    if let Some(fmt_char) = chars.next() {
                        let ctime = platform::get_ctime(meta_opt.unwrap());
                        result.push_str(&format_time_strftime(ctime, fmt_char));
                    }
                }
                Some('t') => {
                    if let Ok(mtime) = ctx.metadata().and_then(|m| m.modified().ok()).ok_or(()) {
                        result.push_str(&format_time_default(mtime));
                    }
                }
                Some('T') => {
                    if let Some(fmt_char) = chars.next() {
                        if let Ok(mtime) = ctx.metadata().and_then(|m| m.modified().ok()).ok_or(())
                        {
                            result.push_str(&format_time_strftime(mtime, fmt_char));
                        }
                    }
                }
                Some(c) => {
                    result.push('%');
                    result.push(c);
                }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
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
        '@' => datetime.timestamp().to_string(),
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

fn format_ls(ctx: &ActionContext) -> String {
    // -ls はメタデータを必要とするアクション。取得できない場合は空文字を返す。
    let Some(meta) = ctx.metadata() else {
        return format!("{}", ctx.path.display());
    };
    let ino = platform::get_ino(meta);
    let blocks = (meta.len() + 511) / 512;
    let mode_str = platform::format_mode_symbolic(platform::get_mode(meta), meta);
    let nlink = platform::get_nlink(meta);

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
