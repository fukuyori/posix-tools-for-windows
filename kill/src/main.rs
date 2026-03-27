use std::env;
use std::mem;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, TerminateProcess, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
};

/// シグナル定義
#[derive(Clone, Copy)]
struct Signal {
    num: i32,
    name: &'static str,
}

/// サポートするシグナル一覧
const SIGNALS: &[Signal] = &[
    Signal { num: 1, name: "HUP" },
    Signal { num: 2, name: "INT" },
    Signal { num: 3, name: "QUIT" },
    Signal { num: 4, name: "ILL" },
    Signal { num: 5, name: "TRAP" },
    Signal { num: 6, name: "ABRT" },
    Signal { num: 7, name: "BUS" },
    Signal { num: 8, name: "FPE" },
    Signal { num: 9, name: "KILL" },
    Signal { num: 10, name: "USR1" },
    Signal { num: 11, name: "SEGV" },
    Signal { num: 12, name: "USR2" },
    Signal { num: 13, name: "PIPE" },
    Signal { num: 14, name: "ALRM" },
    Signal { num: 15, name: "TERM" },
    Signal { num: 16, name: "STKFLT" },
    Signal { num: 17, name: "CHLD" },
    Signal { num: 18, name: "CONT" },
    Signal { num: 19, name: "STOP" },
    Signal { num: 20, name: "TSTP" },
    Signal { num: 21, name: "TTIN" },
    Signal { num: 22, name: "TTOU" },
    Signal { num: 23, name: "URG" },
    Signal { num: 24, name: "XCPU" },
    Signal { num: 25, name: "XFSZ" },
    Signal { num: 26, name: "VTALRM" },
    Signal { num: 27, name: "PROF" },
    Signal { num: 28, name: "WINCH" },
    Signal { num: 29, name: "IO" },
    Signal { num: 30, name: "PWR" },
    Signal { num: 31, name: "SYS" },
];

/// コマンドラインオプション
#[derive(Default)]
struct Options {
    signal: i32,              // 送信するシグナル
    list_signals: bool,       // -l: シグナル一覧表示
    list_table: bool,         // -L: テーブル形式で一覧表示 (GNU拡張)
    list_args: Vec<String>,   // -l の引数（シグナル番号/名前）
    show_help: bool,
    show_version: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let (opts, targets) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("kill: {}", e);
            std::process::exit(1);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("kill 1.0.0 (Rust for Windows)");
        std::process::exit(0);
    }

    // -l オプション: シグナル一覧または変換
    if opts.list_signals {
        if opts.list_args.is_empty() {
            // 引数なし: 全シグナル名を表示
            print_signal_names();
        } else {
            // 引数あり: 番号→名前、名前→番号の変換
            for arg in &opts.list_args {
                print_signal_conversion(arg);
            }
        }
        std::process::exit(0);
    }

    // -L オプション: テーブル形式
    if opts.list_table {
        print_signal_table();
        std::process::exit(0);
    }

    // ターゲットが必要
    if targets.is_empty() {
        eprintln!("kill: PIDを指定してください");
        eprintln!("詳細は 'kill --help' を参照してください");
        std::process::exit(1);
    }

    // シグナル送信
    let mut exit_code = 0;

    // ターゲットを解決（glob 展開含む）
    let resolved_targets = resolve_targets(&targets);

    if resolved_targets.is_empty() {
        eprintln!("kill: マッチするプロセスがありません");
        std::process::exit(1);
    }

    for target in resolved_targets {
        if let Ok(pid) = target.parse::<i32>() {
            // 負のPIDはプロセスグループ（Windowsでは非対応）
            if pid < 0 {
                eprintln!("kill: ({}) - プロセスグループはWindowsでサポートされていません", pid);
                exit_code = 1;
                continue;
            }

            if pid == 0 {
                eprintln!("kill: (0) - 操作は許可されていません");
                exit_code = 1;
                continue;
            }

            if !kill_by_pid(pid as u32, opts.signal) {
                exit_code = 1;
            }
        } else {
            // プロセス名での指定 (GNU拡張的な動作)
            if !kill_by_name(&target, opts.signal) {
                exit_code = 1;
            }
        }
    }

    std::process::exit(exit_code);
}

/// 引数解析
fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options {
        signal: 15, // デフォルト: SIGTERM
        ..Default::default()
    };
    let mut targets = Vec::new();
    let mut i = 1;
    let mut after_double_dash = false;

    while i < args.len() {
        let arg = &args[i];

        // -- 以降は全てターゲット
        if arg == "--" {
            after_double_dash = true;
            i += 1;
            continue;
        }

        if after_double_dash {
            targets.push(arg.clone());
            i += 1;
            continue;
        }

        // ロングオプション
        if arg.starts_with("--") {
            match arg.as_str() {
                "--list" => opts.list_signals = true,
                "--table" => opts.list_table = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                s if s.starts_with("--signal=") => {
                    let sig = s.trim_start_matches("--signal=");
                    opts.signal = parse_signal(sig)?;
                }
                _ => return Err(format!("不明なオプション: '{}'", arg)),
            }
            i += 1;
            continue;
        }

        // ショートオプション
        if arg.starts_with('-') && arg.len() > 1 {
            let opt_chars = &arg[1..];

            // 数字で始まる場合はシグナル番号
            if opt_chars.chars().next().unwrap().is_ascii_digit() {
                opts.signal = opt_chars
                    .parse()
                    .map_err(|_| format!("不正なシグナル番号: '{}'", opt_chars))?;
                i += 1;
                continue;
            }

            // -l オプション
            if opt_chars == "l" {
                opts.list_signals = true;
                // -l の後の引数を収集
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    opts.list_args.push(args[i].clone());
                    i += 1;
                }
                continue;
            }

            // -L オプション (GNU拡張)
            if opt_chars == "L" {
                opts.list_table = true;
                i += 1;
                continue;
            }

            // -s オプション
            if opt_chars == "s" {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-s' には引数が必要です".to_string());
                }
                opts.signal = parse_signal(&args[i])?;
                i += 1;
                continue;
            }

            // -s に値が続く場合 (-sTERM など)
            if opt_chars.starts_with('s') && opt_chars.len() > 1 {
                opts.signal = parse_signal(&opt_chars[1..])?;
                i += 1;
                continue;
            }

            // -n オプション (POSIX: シグナル番号)
            if opt_chars == "n" {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-n' には引数が必要です".to_string());
                }
                opts.signal = args[i]
                    .parse()
                    .map_err(|_| format!("不正なシグナル番号: '{}'", args[i]))?;
                i += 1;
                continue;
            }

            // シグナル名 (-TERM, -KILL, -HUP など)
            let sig_name = opt_chars.to_uppercase();
            let sig_name = sig_name.trim_start_matches("SIG");

            if let Some(sig) = SIGNALS.iter().find(|s| s.name == sig_name) {
                opts.signal = sig.num;
                i += 1;
                continue;
            }

            return Err(format!("不明なオプション: '-{}'", opt_chars));
        }

        // ターゲット
        targets.push(arg.clone());
        i += 1;
    }

    Ok((opts, targets))
}

/// 引数を解決（glob パターン展開対応）
/// 数値の場合はそのまま返す、glob パターンを含む場合は展開する
fn resolve_targets(targets: &[String]) -> Vec<String> {
    let mut resolved = Vec::new();
    let running_processes = get_running_process_names();

    for target in targets {
        // 数値（PID）の場合はそのまま追加
        if target.parse::<i32>().is_ok() {
            resolved.push(target.clone());
            continue;
        }

        // glob パターン判定
        if is_glob_pattern(target) {
            // glob でプロセス名とマッチ
            let pattern = target.to_lowercase();
            for proc_name in &running_processes {
                if matches_glob_pattern(&proc_name.to_lowercase(), &pattern) {
                    resolved.push(proc_name.clone());
                }
            }
        } else {
            // glob パターンでない場合はそのまま追加
            resolved.push(target.clone());
        }
    }

    // 重複を除去
    resolved.sort();
    resolved.dedup();
    resolved
}

/// glob パターンかどうかを判定
fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

/// glob パターンマッチング（簡易版）
fn matches_glob_pattern(name: &str, pattern: &str) -> bool {
    glob_match(name.as_bytes(), pattern.as_bytes())
}

/// glob マッチングの実装
fn glob_match(name: &[u8], pattern: &[u8]) -> bool {
    let mut n_idx = 0;
    let mut p_idx = 0;
    let mut n_star = 0;
    let mut p_star = 0;

    while n_idx < name.len() {
        if p_idx < pattern.len() {
            match pattern[p_idx] {
                b'?' => {
                    n_idx += 1;
                    p_idx += 1;
                    continue;
                }
                b'*' => {
                    p_star = p_idx;
                    n_star = n_idx;
                    p_idx += 1;
                    continue;
                }
                _ if pattern[p_idx] == name[n_idx] => {
                    n_idx += 1;
                    p_idx += 1;
                    continue;
                }
                _ => {}
            }
        }

        if p_star > 0 {
            p_idx = p_star + 1;
            n_star += 1;
            n_idx = n_star;
        } else {
            return false;
        }
    }

    while p_idx < pattern.len() && pattern[p_idx] == b'*' {
        p_idx += 1;
    }

    p_idx == pattern.len()
}

/// 実行中のプロセス名一覧を取得
fn get_running_process_names() -> Vec<String> {
    let mut process_names = Vec::new();

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return process_names,
        };

        let mut entry: PROCESSENTRY32 = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snapshot, &mut entry).is_ok() {
            loop {
                let exe_name = String::from_utf8_lossy(
                    &entry
                        .szExeFile
                        .iter()
                        .take_while(|&&c| c != 0)
                        .map(|&c| c as u8)
                        .collect::<Vec<u8>>(),
                )
                .to_string();

                if !exe_name.is_empty() {
                    process_names.push(exe_name);
                }

                if Process32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }

    process_names
}


/// シグナル名/番号をパース
fn parse_signal(s: &str) -> Result<i32, String> {
    // 数値の場合
    if let Ok(num) = s.parse::<i32>() {
        if num < 0 || num > 64 {
            return Err(format!("不正なシグナル番号: '{}'", num));
        }
        return Ok(num);
    }

    // 名前の場合
    let name = s.to_uppercase();
    let name = name.trim_start_matches("SIG");

    SIGNALS
        .iter()
        .find(|sig| sig.name == name)
        .map(|sig| sig.num)
        .ok_or_else(|| format!("不明なシグナル名: '{}'", s))
}

/// シグナル名一覧を表示（POSIX形式: スペース区切り）
fn print_signal_names() {
    let names: Vec<&str> = SIGNALS.iter().map(|s| s.name).collect();
    println!("{}", names.join(" "));
}

/// シグナル番号↔名前の変換を表示
fn print_signal_conversion(arg: &str) {
    if let Ok(num) = arg.parse::<i32>() {
        // 番号→名前
        if let Some(sig) = SIGNALS.iter().find(|s| s.num == num) {
            println!("{}", sig.name);
        } else {
            eprintln!("kill: {}: 不明なシグナル", num);
        }
    } else {
        // 名前→番号
        let name = arg.to_uppercase();
        let name = name.trim_start_matches("SIG");

        if let Some(sig) = SIGNALS.iter().find(|s| s.name == name) {
            println!("{}", sig.num);
        } else {
            eprintln!("kill: {}: 不明なシグナル", arg);
        }
    }
}

/// シグナル一覧をテーブル形式で表示（GNU拡張 -L）
fn print_signal_table() {
    for (i, sig) in SIGNALS.iter().enumerate() {
        print!("{:2}) {:<8}", sig.num, sig.name);
        if (i + 1) % 4 == 0 {
            println!();
        }
    }
    // 最後の行が4つ未満の場合
    if SIGNALS.len() % 4 != 0 {
        println!();
    }
}

/// PIDでプロセスを終了
fn kill_by_pid(pid: u32, signal: i32) -> bool {
    unsafe {
        let handle = match OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => {
                eprintln!("kill: ({}) - そのようなプロセスはありません", pid);
                return false;
            }
        };

        let result = TerminateProcess(handle, signal as u32);
        let _ = CloseHandle(handle);

        if result.is_ok() {
            // 成功時は何も出力しない（POSIXの動作）
            true
        } else {
            eprintln!("kill: ({}) - 操作は許可されていません", pid);
            false
        }
    }
}

/// プロセス名でプロセスを終了
fn kill_by_name(name: &str, signal: i32) -> bool {
    let pids = find_pids_by_name(name);

    if pids.is_empty() {
        eprintln!("kill: {}: そのようなプロセスはありません", name);
        return false;
    }

    let mut success = true;

    for pid in pids {
        if !kill_by_pid(pid, signal) {
            success = false;
        }
    }

    success
}

/// プロセス名からPIDを検索
fn find_pids_by_name(name: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    let name_lower = name.to_lowercase();
    let name_with_exe = format!("{}.exe", name_lower);

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return pids,
        };

        let mut entry: PROCESSENTRY32 = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snapshot, &mut entry).is_ok() {
            loop {
                let exe_name = String::from_utf8_lossy(
                    &entry
                        .szExeFile
                        .iter()
                        .take_while(|&&c| c != 0)
                        .map(|&c| c as u8)
                        .collect::<Vec<u8>>(),
                )
                .to_string();

                let exe_lower = exe_name.to_lowercase();

                // 完全一致（.exeあり/なし）
                if exe_lower == name_lower || exe_lower == name_with_exe {
                    pids.push(entry.th32ProcessID);
                }

                if Process32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }

    pids
}

/// ヘルプを表示
fn print_help() {
    println!(
        r#"使い方: kill [-s シグナル | -シグナル] PID...
       kill -l [シグナル]...
       kill -L

プロセスにシグナルを送信します。

オプション:
  -s シグナル           送信するシグナルを名前または番号で指定
  -l [シグナル]...      シグナル名を表示、または番号↔名前を変換
  -L, --table           シグナル一覧をテーブル形式で表示
  -シグナル名           シグナルを名前で指定 (-TERM, -KILL, -HUP など)
  -シグナル番号         シグナルを番号で指定 (-9, -15 など)
      --help            このヘルプを表示
      --version         バージョン情報を表示

シグナル:
  デフォルトのシグナルは TERM (15) です。
  よく使われるシグナル:
    HUP (1)     ハングアップ
    INT (2)     割り込み（Ctrl+C相当）
    QUIT (3)    終了
    KILL (9)    強制終了（捕捉不可）
    TERM (15)   終了要求（デフォルト）

  注意: Windowsでは全てのシグナルがTerminateProcessとして
        処理されます。シグナル番号は終了コードとして使用されます。

使用例:
  kill 1234             PID 1234にSIGTERMを送信
  kill -9 1234          PID 1234を強制終了
  kill -KILL 1234       PID 1234を強制終了（同上）
  kill -s TERM 1234     PID 1234にSIGTERMを送信
  kill 1234 5678        複数プロセスにシグナル送信
  kill -l               シグナル名一覧を表示
  kill -l 9             シグナル番号9の名前を表示
  kill -l KILL          シグナルKILLの番号を表示
  kill notepad.exe      notepad.exeを終了（拡張機能）"#
    );
}
