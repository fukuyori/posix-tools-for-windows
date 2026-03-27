use std::collections::{HashMap, HashSet};
use std::env;
use std::mem;

use glob::{MatchOptions, Pattern};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, LookupAccountSidW, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_READ,
};

/// 出力フォーマットで使用可能なフィールド
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputField {
    Pid,
    Ppid,
    Pgid,
    Uid,
    User,
    Gid,
    Group,
    Pri,
    Ni,
    Vsz,
    Rss,
    Pcpu,
    Pmem,
    Etime,
    Time,
    Tty,
    Stat,
    Comm,
    Args,
    Thcount,
    Stime,
}

impl OutputField {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pid" => Some(Self::Pid),
            "ppid" => Some(Self::Ppid),
            "pgid" | "pgrp" => Some(Self::Pgid),
            "uid" | "euid" => Some(Self::Uid),
            "user" | "uname" | "euser" => Some(Self::User),
            "gid" | "egid" => Some(Self::Gid),
            "group" | "egroup" => Some(Self::Group),
            "pri" | "priority" => Some(Self::Pri),
            "ni" | "nice" => Some(Self::Ni),
            "vsz" | "vsize" => Some(Self::Vsz),
            "rss" | "rssize" | "rsz" => Some(Self::Rss),
            "pcpu" | "%cpu" | "c" => Some(Self::Pcpu),
            "pmem" | "%mem" => Some(Self::Pmem),
            "etime" => Some(Self::Etime),
            "time" | "cputime" => Some(Self::Time),
            "tty" | "tt" | "tname" => Some(Self::Tty),
            "stat" | "state" | "s" => Some(Self::Stat),
            "comm" | "ucmd" => Some(Self::Comm),
            "args" | "cmd" | "command" => Some(Self::Args),
            "thcount" | "nlwp" => Some(Self::Thcount),
            "stime" | "start_time" | "start" | "lstart" => Some(Self::Stime),
            _ => None,
        }
    }

    fn header(&self) -> &'static str {
        match self {
            Self::Pid => "PID",
            Self::Ppid => "PPID",
            Self::Pgid => "PGID",
            Self::Uid => "UID",
            Self::User => "USER",
            Self::Gid => "GID",
            Self::Group => "GROUP",
            Self::Pri => "PRI",
            Self::Ni => "NI",
            Self::Vsz => "VSZ",
            Self::Rss => "RSS",
            Self::Pcpu => "%CPU",
            Self::Pmem => "%MEM",
            Self::Etime => "ELAPSED",
            Self::Time => "TIME",
            Self::Tty => "TTY",
            Self::Stat => "STAT",
            Self::Comm => "COMMAND",
            Self::Args => "COMMAND",
            Self::Thcount => "NLWP",
            Self::Stime => "STIME",
        }
    }

    fn width(&self) -> usize {
        match self {
            Self::Pid | Self::Ppid | Self::Pgid => 8,
            Self::Uid | Self::Gid => 6,
            Self::User | Self::Group => 12,
            Self::Pri | Self::Ni | Self::Stat => 4,
            Self::Vsz | Self::Rss => 10,
            Self::Pcpu | Self::Pmem => 5,
            Self::Etime | Self::Time => 11,
            Self::Tty => 8,
            Self::Comm | Self::Args => 0, // 可変長
            Self::Thcount => 5,
            Self::Stime => 8,
        }
    }

    fn is_right_aligned(&self) -> bool {
        matches!(
            self,
            Self::Pid
                | Self::Ppid
                | Self::Pgid
                | Self::Uid
                | Self::Gid
                | Self::Pri
                | Self::Ni
                | Self::Vsz
                | Self::Rss
                | Self::Pcpu
                | Self::Pmem
                | Self::Thcount
        )
    }
}

/// フィールド指定（ヘッダー名のオーバーライド付き）
#[derive(Debug, Clone)]
struct FieldSpec {
    field: OutputField,
    header: Option<String>,
}

impl FieldSpec {
    fn new(field: OutputField) -> Self {
        Self {
            field,
            header: None,
        }
    }

    fn with_header(field: OutputField, header: String) -> Self {
        Self {
            field,
            header: Some(header),
        }
    }

    fn header(&self) -> &str {
        self.header.as_deref().unwrap_or(self.field.header())
    }
}

/// コマンドラインオプション
#[derive(Default)]
struct Options {
    // POSIX標準オプション
    all: bool,                     // -e, -A: 全プロセス
    full: bool,                    // -f: 完全形式
    long: bool,                    // -l: 長形式
    select_pids: Vec<u32>,         // -p: 指定PID
    select_users: Vec<String>,     // -u, -U: 指定ユーザー
    select_groups: Vec<String>,    // -g, -G: 指定グループ
    select_ttys: Vec<String>,      // -t: 指定端末
    tty_only: bool,                // デフォルト: TTY付きのみ（-eなしの場合）
    output_format: Option<String>, // -o: 出力形式指定
    session_leader_only: bool,     // -d の反対

    // BSD形式
    aux_style: bool, // aux: BSD形式

    // GNU拡張
    forest: bool,             // --forest, -H: ツリー表示
    sort_key: Option<String>, // --sort: ソートキー
    no_header: bool,          // --no-headers: ヘッダー非表示
    cumulative: bool,         // -S: 累積時間
    cols: Option<usize>,      // --cols, --columns: 出力幅
    rows: Option<usize>,      // --rows, --lines: 出力行数

    // 特殊
    show_help: bool,
    show_version: bool,
}

/// プロセス情報
#[derive(Clone)]
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    pgid: u32,
    name: String,
    user: String,
    uid: u32,
    group: String,
    gid: u32,
    threads: u32,
    priority: i32,
    nice: i32,
    memory_rss: u64,
    memory_vsz: u64,
    cpu_time: u64,
    cpu_percent: f64,
    mem_percent: f64,
    start_time: String,
    elapsed_time: u64,
    status: String,
    tty: String,
}

/// プロセス詳細（取得用）
struct ProcessDetails {
    user: String,
    uid: u32,
    rss: u64,
    vsz: u64,
    cpu_time: u64,
    cpu_percent: f64,
    mem_percent: f64,
    start_time: String,
    elapsed_time: u64,
    status: String,
}

static mut TOTAL_MEMORY_KB: u64 = 0;
static mut SYSTEM_START_TIME: u64 = 0;

fn main() {
    let args: Vec<String> = env::args().collect();

    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ps: {}", e);
            eprintln!("詳細は 'ps --help' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("ps (Rust Windows版) 1.0.0");
        println!("POSIX.1-2017準拠 + GNU拡張");
        std::process::exit(0);
    }

    // システム情報を初期化
    init_system_info();

    // プロセス一覧を取得
    let mut processes = match get_processes() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ps: プロセス情報の取得に失敗しました: {}", e);
            std::process::exit(1);
        }
    };

    // フィルタリング
    processes = filter_processes(processes, &opts);

    if processes.is_empty() && !opts.select_pids.is_empty() {
        // 指定されたPIDが見つからない場合
        for pid in &opts.select_pids {
            eprintln!("ps: プロセスID {} が見つかりません", pid);
        }
        std::process::exit(1);
    }

    // ソート
    sort_processes(&mut processes, &opts.sort_key);

    // 出力形式を決定
    let fields = determine_output_fields(&opts);

    // 出力
    if opts.forest {
        print_forest(&processes, &fields, &opts);
    } else {
        print_processes(&processes, &fields, &opts);
    }

    std::process::exit(0);
}

/// 引数解析
fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut i = 1;
    let mut had_explicit_selection = false;

    while i < args.len() {
        let arg = &args[i];

        // BSDスタイル（先頭に-なし）
        if !arg.starts_with('-') && i == 1 {
            if arg == "aux" || arg == "ax" || arg == "a" {
                opts.aux_style = true;
                opts.all = true;
                had_explicit_selection = true;
                i += 1;
                continue;
            }
        }

        match arg.as_str() {
            // POSIX標準オプション
            "-e" | "-A" => {
                opts.all = true;
                had_explicit_selection = true;
            }
            "-f" => opts.full = true,
            "-l" => opts.long = true,
            "-a" => {
                // POSIXの-a: 端末に関連付けられた全プロセス（セッションリーダー除く）
                opts.tty_only = false;
                had_explicit_selection = true;
            }
            "-d" => {
                // セッションリーダー以外の全プロセス
                opts.all = true;
                opts.session_leader_only = false;
                had_explicit_selection = true;
            }
            "-S" => opts.cumulative = true,
            "-n" => {
                // POSIX: 出力をソートするnamelistファイル（Windowsでは無視）
            }

            // PID指定
            "-p" | "--pid" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-p' には引数が必要です".to_string());
                }
                parse_pid_list(&args[i], &mut opts.select_pids)?;
                had_explicit_selection = true;
            }

            // 端末指定
            "-t" | "--tty" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-t' には引数が必要です".to_string());
                }
                parse_string_list(&args[i], &mut opts.select_ttys);
                had_explicit_selection = true;
            }

            // ユーザー指定 (POSIX: -u は実効ユーザー, -U は実ユーザー)
            "-u" | "-U" | "--user" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("オプション '{}' には引数が必要です", arg));
                }
                parse_string_list(&args[i], &mut opts.select_users);
                had_explicit_selection = true;
            }

            // グループ指定
            "-g" | "-G" | "--group" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("オプション '{}' には引数が必要です", arg));
                }
                parse_string_list(&args[i], &mut opts.select_groups);
                had_explicit_selection = true;
            }

            // 出力形式指定
            "-o" | "-O" | "--format" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("オプション '{}' には引数が必要です", arg));
                }
                opts.output_format = Some(args[i].clone());
            }

            // GNU拡張
            "-H" | "--forest" => opts.forest = true,
            "--no-header" | "--no-headers" => opts.no_header = true,
            "--sort" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '--sort' には引数が必要です".to_string());
                }
                opts.sort_key = Some(args[i].clone());
            }
            "--cols" | "--columns" | "--width" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("オプション '{}' には引数が必要です", arg));
                }
                opts.cols = args[i].parse().ok();
            }
            "--rows" | "--lines" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("オプション '{}' には引数が必要です", arg));
                }
                opts.rows = args[i].parse().ok();
            }
            "--help" | "-h" => opts.show_help = true,
            "--version" | "-V" => opts.show_version = true,

            // 複合オプション処理
            s if s.starts_with("--sort=") => {
                opts.sort_key = Some(s.trim_start_matches("--sort=").to_string());
            }
            s if s.starts_with("--pid=") => {
                parse_pid_list(s.trim_start_matches("--pid="), &mut opts.select_pids)?;
                had_explicit_selection = true;
            }
            s if s.starts_with("--user=") => {
                parse_string_list(s.trim_start_matches("--user="), &mut opts.select_users);
                had_explicit_selection = true;
            }
            s if s.starts_with("--cols=") => {
                opts.cols = s.trim_start_matches("--cols=").parse().ok();
            }
            s if s.starts_with("-o") && s.len() > 2 => {
                opts.output_format = Some(s[2..].to_string());
            }
            s if s.starts_with("-p") && s.len() > 2 => {
                parse_pid_list(&s[2..], &mut opts.select_pids)?;
                had_explicit_selection = true;
            }
            s if s.starts_with("-u") && s.len() > 2 => {
                parse_string_list(&s[2..], &mut opts.select_users);
                had_explicit_selection = true;
            }
            s if s.starts_with("-g") && s.len() > 2 => {
                parse_string_list(&s[2..], &mut opts.select_groups);
                had_explicit_selection = true;
            }
            s if s.starts_with("-t") && s.len() > 2 => {
                parse_string_list(&s[2..], &mut opts.select_ttys);
                had_explicit_selection = true;
            }

            // 短縮形式オプション群 (-ef など)
            s if s.starts_with('-') && !s.starts_with("--") && s.len() > 2 => {
                for c in s.chars().skip(1) {
                    match c {
                        'e' | 'A' => {
                            opts.all = true;
                            had_explicit_selection = true;
                        }
                        'f' => opts.full = true,
                        'l' => opts.long = true,
                        'a' => {
                            opts.tty_only = false;
                            had_explicit_selection = true;
                        }
                        'd' => {
                            opts.all = true;
                            had_explicit_selection = true;
                        }
                        'H' => opts.forest = true,
                        'S' => opts.cumulative = true,
                        'h' => opts.show_help = true,
                        'V' => opts.show_version = true,
                        'n' => {} // POSIX namelist（無視）
                        _ => {
                            return Err(format!("不正なオプション: '-{}'", c));
                        }
                    }
                }
            }

            // BSDスタイル引数
            "aux" | "-aux" => {
                opts.aux_style = true;
                opts.all = true;
                had_explicit_selection = true;
            }

            s => {
                return Err(format!("不正なオプション: '{}'", s));
            }
        }

        i += 1;
    }

    // デフォルト: -eや-aなどの選択オプションがない場合、全プロセスを表示
    // （Windowsでは端末の概念が異なるため）
    if !had_explicit_selection {
        opts.all = true;
    }

    Ok(opts)
}

/// PIDリストをパース
fn parse_pid_list(s: &str, list: &mut Vec<u32>) -> Result<(), String> {
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<u32>() {
            Ok(pid) => list.push(pid),
            Err(_) => {
                return Err(format!("不正なPID: '{}'", part));
            }
        }
    }
    Ok(())
}

/// 文字列リストをパース
fn parse_string_list(s: &str, list: &mut Vec<String>) {
    for part in s.split(',') {
        let part = part.trim();
        if !part.is_empty() {
            list.push(part.to_string());
        }
    }
}

/// システム情報を初期化
fn init_system_info() {
    unsafe {
        let mut mem_status: MEMORYSTATUSEX = mem::zeroed();
        mem_status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
            TOTAL_MEMORY_KB = mem_status.ullTotalPhys / 1024;
        }

        // システム起動時刻を取得（概算）
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        SYSTEM_START_TIME = now;
    }
}

/// 出力フィールドを決定
fn determine_output_fields(opts: &Options) -> Vec<FieldSpec> {
    // -o オプションが指定されている場合
    if let Some(ref fmt) = opts.output_format {
        return parse_output_format(fmt);
    }

    // aux形式
    if opts.aux_style {
        return vec![
            FieldSpec::new(OutputField::User),
            FieldSpec::new(OutputField::Pid),
            FieldSpec::new(OutputField::Pcpu),
            FieldSpec::new(OutputField::Pmem),
            FieldSpec::new(OutputField::Vsz),
            FieldSpec::new(OutputField::Rss),
            FieldSpec::new(OutputField::Tty),
            FieldSpec::new(OutputField::Stat),
            FieldSpec::new(OutputField::Stime),
            FieldSpec::new(OutputField::Time),
            FieldSpec::new(OutputField::Comm),
        ];
    }

    // 長形式 (-l)
    if opts.long {
        return vec![
            FieldSpec::new(OutputField::User),
            FieldSpec::new(OutputField::Pid),
            FieldSpec::new(OutputField::Ppid),
            FieldSpec::new(OutputField::Pri),
            FieldSpec::new(OutputField::Ni),
            FieldSpec::new(OutputField::Vsz),
            FieldSpec::new(OutputField::Rss),
            FieldSpec::new(OutputField::Stat),
            FieldSpec::new(OutputField::Time),
            FieldSpec::new(OutputField::Comm),
        ];
    }

    // 完全形式 (-f)
    if opts.full {
        return vec![
            FieldSpec::new(OutputField::User),
            FieldSpec::new(OutputField::Pid),
            FieldSpec::new(OutputField::Ppid),
            FieldSpec::with_header(OutputField::Pcpu, "C".to_string()),
            FieldSpec::new(OutputField::Stime),
            FieldSpec::new(OutputField::Tty),
            FieldSpec::new(OutputField::Time),
            FieldSpec::new(OutputField::Comm),
        ];
    }

    // デフォルト（POSIX標準）
    vec![
        FieldSpec::new(OutputField::Pid),
        FieldSpec::new(OutputField::Tty),
        FieldSpec::new(OutputField::Time),
        FieldSpec::new(OutputField::Comm),
    ]
}

/// -o オプションの形式をパース
fn parse_output_format(fmt: &str) -> Vec<FieldSpec> {
    let mut fields = Vec::new();

    for part in fmt.split([',', ' ']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // field=header 形式をパース
        if let Some(eq_pos) = part.find('=') {
            let field_name = &part[..eq_pos];
            let header = &part[eq_pos + 1..];
            if let Some(field) = OutputField::from_str(field_name) {
                fields.push(FieldSpec::with_header(field, header.to_string()));
            }
        } else if let Some(field) = OutputField::from_str(part) {
            fields.push(FieldSpec::new(field));
        }
    }

    if fields.is_empty() {
        // パースに失敗した場合はデフォルト
        vec![
            FieldSpec::new(OutputField::Pid),
            FieldSpec::new(OutputField::Tty),
            FieldSpec::new(OutputField::Time),
            FieldSpec::new(OutputField::Comm),
        ]
    } else {
        fields
    }
}

/// プロセス一覧を取得
fn get_processes() -> Result<Vec<ProcessInfo>, String> {
    let mut processes = Vec::new();
    let current_pid = unsafe { GetCurrentProcessId() };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| format!("スナップショットの作成に失敗しました: {}", e))?;

        let mut entry: PROCESSENTRY32 = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf8_lossy(
                    &entry
                        .szExeFile
                        .iter()
                        .take_while(|&&c| c != 0)
                        .map(|&c| c as u8)
                        .collect::<Vec<u8>>(),
                )
                .to_string();

                let details = get_process_details(entry.th32ProcessID);

                // TTY判定（簡易：自プロセスと同じセッションならコンソールあり）
                let tty = if entry.th32ProcessID == current_pid {
                    "cons".to_string()
                } else {
                    "?".to_string()
                };

                processes.push(ProcessInfo {
                    pid: entry.th32ProcessID,
                    ppid: entry.th32ParentProcessID,
                    pgid: entry.th32ParentProcessID, // Windowsにはプロセスグループがないのでppidで代用
                    name,
                    user: details.user,
                    uid: details.uid,
                    group: String::new(),
                    gid: 0,
                    threads: entry.cntThreads,
                    priority: entry.pcPriClassBase as i32,
                    nice: 0, // Windowsにはniceがない
                    memory_rss: details.rss,
                    memory_vsz: details.vsz,
                    cpu_time: details.cpu_time,
                    cpu_percent: details.cpu_percent,
                    mem_percent: details.mem_percent,
                    start_time: details.start_time,
                    elapsed_time: details.elapsed_time,
                    status: details.status,
                    tty,
                });

                if Process32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }

    Ok(processes)
}

/// プロセス詳細を取得
fn get_process_details(pid: u32) -> ProcessDetails {
    let mut details = ProcessDetails {
        user: String::new(),
        uid: 0,
        rss: 0,
        vsz: 0,
        cpu_time: 0,
        cpu_percent: 0.0,
        mem_percent: 0.0,
        start_time: String::new(),
        elapsed_time: 0,
        status: "?".to_string(),
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid);

        if let Ok(handle) = handle {
            // メモリ情報
            let mut counters: PROCESS_MEMORY_COUNTERS_EX = mem::zeroed();
            counters.cb = mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;

            if GetProcessMemoryInfo(
                handle,
                &mut counters as *mut _ as *mut _,
                mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            )
            .is_ok()
            {
                details.rss = counters.WorkingSetSize as u64 / 1024;
                // VSZ: PagefileUsage（コミットされた仮想メモリ）がLinuxのVSZに近い
                details.vsz = counters.PagefileUsage as u64 / 1024;

                if TOTAL_MEMORY_KB > 0 {
                    details.mem_percent = (details.rss as f64 / TOTAL_MEMORY_KB as f64) * 100.0;
                }
            }

            // 時間情報
            let mut creation_time = mem::zeroed();
            let mut exit_time = mem::zeroed();
            let mut kernel_time = mem::zeroed();
            let mut user_time = mem::zeroed();

            if GetProcessTimes(
                handle,
                &mut creation_time,
                &mut exit_time,
                &mut kernel_time,
                &mut user_time,
            )
            .is_ok()
            {
                let kernel = ((kernel_time.dwHighDateTime as u64) << 32)
                    | (kernel_time.dwLowDateTime as u64);
                let user =
                    ((user_time.dwHighDateTime as u64) << 32) | (user_time.dwLowDateTime as u64);
                details.cpu_time = (kernel + user) / 10_000;

                let creation = ((creation_time.dwHighDateTime as u64) << 32)
                    | (creation_time.dwLowDateTime as u64);
                details.start_time = filetime_to_string(creation);
                details.elapsed_time = calculate_elapsed_time(creation);

                // %CPU: プロセス開始からの平均CPU使用率（Linux ps互換）
                // 計算式: (CPU時間 / 経過時間) * 100
                if details.elapsed_time > 0 {
                    let cpu_seconds = details.cpu_time as f64 / 1000.0;
                    details.cpu_percent = (cpu_seconds / details.elapsed_time as f64) * 100.0;
                    // 100%を超える場合はマルチコア使用（表示上は許容）
                }

                // 状態判定
                // Windowsでは正確なプロセス状態を取得困難なため、
                // CPU使用状況で近似判定
                // S: Sleeping（通常の待機状態）- デフォルト
                details.status = "S".to_string();
            }

            // ユーザー情報
            details.user = get_process_user(handle);

            let _ = CloseHandle(handle);
        }
    }

    details
}

/// プロセスのユーザー名を取得
fn get_process_user(process_handle: HANDLE) -> String {
    unsafe {
        let mut token_handle: HANDLE = HANDLE::default();

        if OpenProcessToken(process_handle, TOKEN_QUERY, &mut token_handle).is_err() {
            return String::new();
        }

        let mut token_info_len: u32 = 0;
        let _ = GetTokenInformation(token_handle, TokenUser, None, 0, &mut token_info_len);

        if token_info_len == 0 {
            let _ = CloseHandle(token_handle);
            return String::new();
        }

        let mut token_info: Vec<u8> = vec![0; token_info_len as usize];

        if GetTokenInformation(
            token_handle,
            TokenUser,
            Some(token_info.as_mut_ptr() as *mut _),
            token_info_len,
            &mut token_info_len,
        )
        .is_err()
        {
            let _ = CloseHandle(token_handle);
            return String::new();
        }

        let token_user = &*(token_info.as_ptr() as *const TOKEN_USER);
        let sid = token_user.User.Sid;

        let mut name_buf: [u16; 256] = [0; 256];
        let mut domain_buf: [u16; 256] = [0; 256];
        let mut name_len: u32 = 256;
        let mut domain_len: u32 = 256;
        let mut sid_type = windows::Win32::Security::SID_NAME_USE::default();

        if LookupAccountSidW(
            PWSTR::null(),
            sid,
            PWSTR(name_buf.as_mut_ptr()),
            &mut name_len,
            PWSTR(domain_buf.as_mut_ptr()),
            &mut domain_len,
            &mut sid_type,
        )
        .is_ok()
        {
            let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let _ = CloseHandle(token_handle);
            return name;
        }

        let _ = CloseHandle(token_handle);
        String::new()
    }
}

/// FILETIMEを文字列に変換
fn filetime_to_string(filetime: u64) -> String {
    if filetime == 0 {
        return String::new();
    }

    const FILETIME_UNIX_DIFF: u64 = 116444736000000000;

    if filetime < FILETIME_UNIX_DIFF {
        return String::new();
    }

    let unix_time = (filetime - FILETIME_UNIX_DIFF) / 10_000_000;

    let secs = unix_time % 60;
    let mins = (unix_time / 60) % 60;
    let hours = (unix_time / 3600) % 24;

    // JST (+9時間)
    let hours_jst = (hours + 9) % 24;

    format!("{:02}:{:02}:{:02}", hours_jst, mins, secs)
}

/// 経過時間を計算
fn calculate_elapsed_time(creation_filetime: u64) -> u64 {
    if creation_filetime == 0 {
        return 0;
    }

    const FILETIME_UNIX_DIFF: u64 = 116444736000000000;

    if creation_filetime < FILETIME_UNIX_DIFF {
        return 0;
    }

    let start_unix = (creation_filetime - FILETIME_UNIX_DIFF) / 10_000_000;

    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now > start_unix {
        now - start_unix
    } else {
        0
    }
}

/// 経過時間をフォーマット（POSIX形式: [[dd-]hh:]mm:ss）
fn format_elapsed_time(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;

    if days > 0 {
        format!("{:02}-{:02}:{:02}:{:02}", days, hours, mins, s)
    } else if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, s)
    } else {
        format!("{:02}:{:02}", mins, s)
    }
}

/// CPU時間をフォーマット（POSIX形式: [dd-]hh:mm:ss）
fn format_cpu_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours >= 24 {
        let days = hours / 24;
        let h = hours % 24;
        format!("{:02}-{:02}:{:02}:{:02}", days, h, mins, secs)
    } else if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

/// プロセスをフィルタリング
fn filter_processes(processes: Vec<ProcessInfo>, opts: &Options) -> Vec<ProcessInfo> {
    processes
        .into_iter()
        .filter(|p| {
            // PID指定
            if !opts.select_pids.is_empty() && !opts.select_pids.contains(&p.pid) {
                return false;
            }

            // ユーザー指定
            if !opts.select_users.is_empty() {
                if !opts
                    .select_users
                    .iter()
                    .any(|u| matches_selector(u, &p.user, false))
                {
                    return false;
                }
            }

            // グループ指定
            if !opts.select_groups.is_empty() {
                if !opts
                    .select_groups
                    .iter()
                    .any(|g| matches_selector(g, &p.group, false))
                {
                    return false;
                }
            }

            // TTY指定
            if !opts.select_ttys.is_empty() {
                if !opts
                    .select_ttys
                    .iter()
                    .any(|t| matches_selector(t, &p.tty, true))
                {
                    return false;
                }
            }

            true
        })
        .collect()
}

fn matches_selector(pattern: &str, value: &str, allow_substring_without_glob: bool) -> bool {
    if contains_glob_metachar(pattern) {
        return Pattern::new(pattern)
            .map(|compiled| compiled.matches_with(value, glob_match_options()))
            .unwrap_or(false);
    }

    if allow_substring_without_glob {
        return value.to_lowercase().contains(&pattern.to_lowercase());
    }

    pattern.to_lowercase() == value.to_lowercase()
}

fn contains_glob_metachar(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn glob_match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    }
}

/// プロセスをソート
fn sort_processes(processes: &mut [ProcessInfo], sort_key: &Option<String>) {
    let key = match sort_key {
        Some(k) => k.as_str(),
        None => {
            processes.sort_by_key(|p| p.pid);
            return;
        }
    };

    let descending = key.starts_with('-');
    let key = key.trim_start_matches(['-', '+']);

    match key.to_lowercase().as_str() {
        "pid" => processes.sort_by_key(|p| p.pid),
        "ppid" => processes.sort_by_key(|p| p.ppid),
        "name" | "cmd" | "comm" | "args" => {
            processes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
        "user" | "uname" => {
            processes.sort_by(|a, b| a.user.to_lowercase().cmp(&b.user.to_lowercase()));
        }
        "mem" | "rss" | "%mem" | "pmem" => {
            processes.sort_by(|a, b| {
                a.memory_rss
                    .partial_cmp(&b.memory_rss)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        "vsz" | "vsize" => processes.sort_by_key(|p| p.memory_vsz),
        "cpu" | "time" | "%cpu" | "pcpu" => processes.sort_by_key(|p| p.cpu_time),
        "threads" | "nlwp" | "thcount" => processes.sort_by_key(|p| p.threads),
        "pri" | "priority" => processes.sort_by_key(|p| p.priority),
        "start" | "stime" | "etime" => {
            processes.sort_by(|a, b| a.start_time.cmp(&b.start_time));
        }
        _ => processes.sort_by_key(|p| p.pid),
    }

    if descending {
        processes.reverse();
    }
}

/// フィールド値を取得
fn get_field_value(proc: &ProcessInfo, field: &OutputField) -> String {
    match field {
        OutputField::Pid => proc.pid.to_string(),
        OutputField::Ppid => proc.ppid.to_string(),
        OutputField::Pgid => proc.pgid.to_string(),
        OutputField::Uid => proc.uid.to_string(),
        OutputField::User => proc.user.clone(),
        OutputField::Gid => proc.gid.to_string(),
        OutputField::Group => proc.group.clone(),
        OutputField::Pri => proc.priority.to_string(),
        OutputField::Ni => proc.nice.to_string(),
        OutputField::Vsz => proc.memory_vsz.to_string(),
        OutputField::Rss => proc.memory_rss.to_string(),
        OutputField::Pcpu => format!("{:.1}", proc.cpu_percent),
        OutputField::Pmem => format!("{:.1}", proc.mem_percent),
        OutputField::Etime => format_elapsed_time(proc.elapsed_time),
        OutputField::Time => format_cpu_time(proc.cpu_time),
        OutputField::Tty => proc.tty.clone(),
        OutputField::Stat => {
            // Linux ps互換のSTAT表示
            // 基本状態 + 修飾子
            let mut stat = proc.status.clone();
            // l: マルチスレッド（Linux: is multi-threaded）
            if proc.threads > 1 {
                stat.push('l');
            }
            stat
        }
        OutputField::Comm => proc.name.clone(),
        OutputField::Args => proc.name.clone(),
        OutputField::Thcount => proc.threads.to_string(),
        OutputField::Stime => proc.start_time.clone(),
    }
}

/// プロセス一覧を出力
fn print_processes(processes: &[ProcessInfo], fields: &[FieldSpec], opts: &Options) {
    // ヘッダー出力
    if !opts.no_header {
        print_header(fields);
    }

    // 各プロセスを出力
    for proc in processes {
        print_process_line(proc, fields, "", "");
    }
}

/// ヘッダーを出力
fn print_header(fields: &[FieldSpec]) {
    let mut line = String::new();

    for (i, spec) in fields.iter().enumerate() {
        let header = spec.header();
        let width = spec.field.width();

        if i > 0 {
            line.push(' ');
        }

        if width == 0 {
            // 可変長フィールド
            line.push_str(header);
        } else if spec.field.is_right_aligned() {
            line.push_str(&format!("{:>width$}", header, width = width));
        } else {
            line.push_str(&format!("{:<width$}", header, width = width));
        }
    }

    println!("{}", line);
}

/// プロセス行を出力
fn print_process_line(proc: &ProcessInfo, fields: &[FieldSpec], prefix: &str, tree_prefix: &str) {
    let mut line = String::new();

    for (i, spec) in fields.iter().enumerate() {
        let mut value = get_field_value(proc, &spec.field);
        let width = spec.field.width();

        // COMM/ARGSフィールドにはツリープレフィックスを追加
        if matches!(spec.field, OutputField::Comm | OutputField::Args) && !prefix.is_empty() {
            value = format!("{}{}{}", prefix, tree_prefix, value);
        }

        if i > 0 {
            line.push(' ');
        }

        if width == 0 {
            // 可変長フィールド
            line.push_str(&value);
        } else if spec.field.is_right_aligned() {
            line.push_str(&format!("{:>width$}", value, width = width));
        } else {
            // 左寄せでも幅を超える場合は切り詰め
            if value.chars().count() > width {
                let truncated: String = value.chars().take(width).collect();
                line.push_str(&truncated);
            } else {
                line.push_str(&format!("{:<width$}", value, width = width));
            }
        }
    }

    println!("{}", line);
}

/// ツリー形式で出力
fn print_forest(processes: &[ProcessInfo], fields: &[FieldSpec], opts: &Options) {
    let pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
    let children_map: HashMap<u32, Vec<&ProcessInfo>> = {
        let mut map: HashMap<u32, Vec<&ProcessInfo>> = HashMap::new();
        for proc in processes {
            map.entry(proc.ppid).or_default().push(proc);
        }
        map
    };

    // ルートプロセスを特定（親がリスト内にないか、ppid=0）
    let roots: Vec<&ProcessInfo> = processes
        .iter()
        .filter(|p| !pids.contains(&p.ppid) || p.ppid == 0)
        .collect();

    // ヘッダー出力
    if !opts.no_header {
        print_header(fields);
    }

    // ツリーを出力
    for root in roots {
        print_tree_node(root, fields, &children_map, "", true);
    }
}

/// ツリーノードを出力
fn print_tree_node(
    proc: &ProcessInfo,
    fields: &[FieldSpec],
    children_map: &HashMap<u32, Vec<&ProcessInfo>>,
    prefix: &str,
    is_last: bool,
) {
    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─"
    } else {
        "├─"
    };

    print_process_line(proc, fields, prefix, connector);

    // 子プロセスを取得
    let children = children_map.get(&proc.pid);

    if let Some(children) = children {
        let filtered: Vec<&&ProcessInfo> = children.iter().filter(|c| c.pid != proc.pid).collect();

        let new_prefix = if prefix.is_empty() {
            String::new()
        } else if is_last {
            format!("{}  ", prefix)
        } else {
            format!("{}│ ", prefix)
        };

        for (i, child) in filtered.iter().enumerate() {
            let child_is_last = i == filtered.len() - 1;
            print_tree_node(child, fields, children_map, &new_prefix, child_is_last);
        }
    }
}

/// ヘルプを表示
fn print_help() {
    println!(
        r#"使い方: ps [オプション]

プロセスの状態を表示します。

プロセス選択オプション（POSIX標準）:
  -e, -A              全プロセスを選択
  -a                  端末に関連付けられた全プロセス（セッションリーダー除く）
  -d                  セッションリーダー以外の全プロセス
  -p, --pid <PID,...> 指定したPIDのプロセスのみ
  -t, --tty <tty,...> 指定した端末のプロセス
  -u, -U, --user <ユーザー,...>
                      指定したユーザーのプロセス
  -g, -G, --group <グループ,...>
                      指定したグループのプロセス

出力形式オプション（POSIX標準）:
  -f                  完全形式で出力
  -l                  長形式で出力
  -o, --format <形式> 出力形式を指定
                      形式: field[=header],...
                      使用可能なフィールド:
                        pid    - プロセスID
                        ppid   - 親プロセスID
                        pgid   - プロセスグループID
                        uid    - ユーザーID
                        user   - ユーザー名
                        gid    - グループID
                        group  - グループ名
                        pri    - 優先度
                        ni     - nice値
                        vsz    - 仮想メモリサイズ(KB)
                        rss    - 常駐メモリサイズ(KB)
                        pcpu   - CPU使用率(%)
                        pmem   - メモリ使用率(%)
                        etime  - 経過時間
                        time   - CPU時間
                        tty    - 端末
                        stat   - プロセス状態
                        comm   - コマンド名
                        args   - コマンドライン
                        nlwp   - スレッド数
                        stime  - 開始時刻

BSD形式オプション:
  aux                 BSD形式で出力（USER PID %CPU %MEM VSZ RSS...）

GNU拡張オプション:
  --sort <[+|-]key>   指定キーでソート（-で降順）
                      キー: pid, ppid, name, user, mem, vsz, cpu,
                            threads, pri, start
  -H, --forest        プロセス階層をツリー表示
  --no-headers        ヘッダーを表示しない
  -S                  子プロセスのCPU時間を累積
  --cols=N, --columns=N
                      出力幅を指定
  --rows=N, --lines=N 出力行数を指定

セレクタ補足:
  -u/-U, -g/-G, -t    *, ?, [...] の glob パターンを使用可能
                      Windows でも ps 内部で評価し、Linux の未引用ワイルドカード利用に近い挙動を提供

情報オプション:
  -h, --help          このヘルプを表示
  -V, --version       バージョン情報を表示

終了ステータス:
  0  正常終了
  1  エラー発生
  2  オプションエラー

使用例:
  ps                  全プロセスを表示
  ps -ef              全プロセスを完全形式で表示
  ps -el              全プロセスを長形式で表示
  ps aux              BSD形式で表示
  ps -p 1234          PID 1234のプロセスを表示
  ps -p 1234,5678     複数のPIDを指定
  ps -u Administrator Administratorのプロセスを表示
  ps -o pid,user,comm カスタム形式で表示
  ps -o pid=プロセス,comm=コマンド
                      カスタムヘッダーで表示
  ps --forest         ツリー形式で表示
  ps --sort=-mem      メモリ使用量の降順でソート
  ps --sort=+pid      PIDの昇順でソート

注意:
  WindowsではUNIXのTTY、プロセスグループ等の概念が異なるため、
  一部のフィールドは近似値または固定値となります。"#
    );
}

#[cfg(test)]
mod tests {
    use super::matches_selector;

    #[test]
    fn exact_user_match_is_case_insensitive() {
        assert!(matches_selector("SYSTEM", "system", false));
    }

    #[test]
    fn glob_user_match_works_on_windows_side() {
        assert!(matches_selector(
            "NT AUTHORITY\\*",
            "nt authority\\system",
            false
        ));
    }

    #[test]
    fn tty_selector_keeps_existing_substring_behavior_without_glob() {
        assert!(matches_selector("con", "console", true));
    }

    #[test]
    fn tty_selector_supports_glob() {
        assert!(matches_selector("con*", "console", true));
    }
}
