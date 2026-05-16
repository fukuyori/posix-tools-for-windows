// top - プロセス監視ツール
// Unix top互換 + GNU拡張

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use windows::core::{PCSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, LookupAccountSidW, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::{
    GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, GetSystemTimes, OpenProcess, OpenProcessToken, SetPriorityClass,
    TerminateProcess, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_QUERY_INFORMATION,
    PROCESS_SET_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ,
};

#[derive(Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    user: String,
    threads: u32,
    priority: i32,
    memory_rss: u64,
    memory_vsz: u64,
    cpu_percent: f64,
    mem_percent: f64,
    gpu_percent: Option<f64>,
    gpu_mem_percent: Option<f64>,
    gpu_memory_mb: Option<u64>,
    cpu_time: u64,
    status: String,
    kernel_time: u64,
    user_time: u64,
}

struct SystemInfo {
    uptime_secs: u64,
    total_procs: usize,
    running_procs: usize,
    sleeping_procs: usize,
    cpu_user: f64,
    cpu_system: f64,
    cpu_idle: f64,
    mem_total: u64,
    mem_used: u64,
    mem_free: u64,
    swap_total: u64,
    swap_used: u64,
    swap_free: u64,
    gpus: Vec<GpuInfo>,
    vulkan: Option<VulkanInfo>,
}

#[derive(Clone, Debug, Default)]
struct GpuInfo {
    name: String,
    utilization_percent: Option<f64>,
    memory_total_mb: Option<u64>,
    memory_used_mb: Option<u64>,
    temperature_c: Option<u32>,
}

#[derive(Clone, Debug, Default)]
struct GpuProcessInfo {
    gpu_percent: Option<f64>,
    gpu_mem_percent: Option<f64>,
    gpu_memory_mb: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct VulkanInfo {
    api_version: VulkanVersion,
    devices: Vec<VulkanDeviceInfo>,
}

#[derive(Clone, Debug, Default)]
struct VulkanDeviceInfo {
    index: usize,
    name: Option<String>,
    device_type: Option<String>,
    api_version: Option<String>,
    driver_name: Option<String>,
    driver_info: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct VulkanVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

#[derive(Clone, Copy, PartialEq, Default)]
enum SortKey {
    Pid,
    #[default]
    Cpu,
    Mem,
    Time,
    Name,
    User,
    Res,
}

#[derive(Default)]
struct Options {
    // Unix top互換オプション
    delay: f64,                  // -d: 更新間隔（秒）
    iterations: Option<u32>,     // -n: 更新回数
    batch_mode: bool,            // -b: バッチモード
    secure_mode: bool,           // -s: セキュアモード（キルなど無効）
    user_filter: Option<String>, // -u, -U: ユーザーフィルタ
    pid_filter: Vec<u32>,        // -p: PIDフィルタ

    // GNU top拡張
    sort_field: SortKey, // -o: ソートフィールド
    sort_reverse: bool,
    show_threads: bool, // -H: スレッド表示

    show_help: bool,
    show_version: bool,
}

struct TopState {
    processes: Vec<ProcessInfo>,
    system: SystemInfo,
    sort_key: SortKey,
    sort_reverse: bool,
    scroll_offset: usize,
    selected_index: usize,
    show_help: bool,
    message: Option<String>,
    prev_cpu_times: CpuTimes,
    prev_proc_times: HashMap<u32, (u64, u64, Instant)>,
    refresh_interval: Duration,
    iterations_left: Option<u32>,
    batch_mode: bool,
    secure_mode: bool,
    user_filter: Option<String>,
    pid_filter: Vec<u32>,
    vulkan_info: Option<VulkanInfo>,
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();

    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("top: {}", e);
            eprintln!("詳細は 'top -h' を参照してください");
            std::process::exit(2);
        }
    };

    if opts.show_help {
        print_help();
        std::process::exit(0);
    }

    if opts.show_version {
        println!("top (Rust版) 1.0.0");
        println!("Unix top互換 + GNU拡張");
        std::process::exit(0);
    }

    let mut state = TopState {
        processes: Vec::new(),
        system: SystemInfo::default(),
        sort_key: opts.sort_field,
        sort_reverse: opts.sort_reverse,
        scroll_offset: 0,
        selected_index: 0,
        show_help: false,
        message: None,
        prev_cpu_times: get_cpu_times(),
        prev_proc_times: HashMap::new(),
        refresh_interval: Duration::from_secs_f64(opts.delay),
        iterations_left: opts.iterations,
        batch_mode: opts.batch_mode,
        secure_mode: opts.secure_mode,
        user_filter: opts.user_filter,
        pid_filter: opts.pid_filter,
        vulkan_info: get_vulkan_info(),
    };

    if state.batch_mode {
        run_batch_mode(&mut state)
    } else {
        run_interactive_mode(&mut state)
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options {
        delay: 1.0,
        sort_field: SortKey::Cpu,
        sort_reverse: true,
        ..Default::default()
    };

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];

        match arg.as_str() {
            "-h" | "--help" => opts.show_help = true,
            "-v" | "--version" => opts.show_version = true,
            "-b" | "--batch" => opts.batch_mode = true,
            "-s" | "--secure" => opts.secure_mode = true,
            "-H" | "--threads" => opts.show_threads = true,
            "-d" | "--delay" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-d' には引数が必要です".to_string());
                }
                opts.delay = args[i]
                    .parse()
                    .map_err(|_| format!("無効な遅延時間: '{}'", args[i]))?;
                if opts.delay < 0.1 {
                    opts.delay = 0.1;
                }
            }
            "-n" | "--iterations" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-n' には引数が必要です".to_string());
                }
                opts.iterations = Some(
                    args[i]
                        .parse()
                        .map_err(|_| format!("無効な回数: '{}'", args[i]))?,
                );
            }
            "-u" | "-U" | "--user" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-u' には引数が必要です".to_string());
                }
                opts.user_filter = Some(args[i].clone());
            }
            "-p" | "--pid" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-p' には引数が必要です".to_string());
                }
                for pid_str in args[i].split(',') {
                    let pid: u32 = pid_str
                        .trim()
                        .parse()
                        .map_err(|_| format!("無効なPID: '{}'", pid_str))?;
                    opts.pid_filter.push(pid);
                }
            }
            "-o" | "--sort" => {
                i += 1;
                if i >= args.len() {
                    return Err("オプション '-o' には引数が必要です".to_string());
                }
                let field = args[i].trim_start_matches(['+', '-'].as_ref());
                opts.sort_reverse = !args[i].starts_with('+');
                opts.sort_field = match field.to_uppercase().as_str() {
                    "PID" => SortKey::Pid,
                    "%CPU" | "CPU" => SortKey::Cpu,
                    "%MEM" | "MEM" => SortKey::Mem,
                    "TIME" | "TIME+" => SortKey::Time,
                    "COMMAND" | "CMD" | "NAME" => SortKey::Name,
                    "USER" => SortKey::User,
                    "RES" | "RSS" => SortKey::Res,
                    _ => return Err(format!("不明なソートフィールド: '{}'", field)),
                };
            }
            // -dNUM形式
            s if s.starts_with("-d") && s.len() > 2 => {
                opts.delay = s[2..]
                    .parse()
                    .map_err(|_| format!("無効な遅延時間: '{}'", &s[2..]))?;
            }
            // -nNUM形式
            s if s.starts_with("-n") && s.len() > 2 => {
                opts.iterations = Some(
                    s[2..]
                        .parse()
                        .map_err(|_| format!("無効な回数: '{}'", &s[2..]))?,
                );
            }
            // -pPID形式
            s if s.starts_with("-p") && s.len() > 2 => {
                for pid_str in s[2..].split(',') {
                    let pid: u32 = pid_str
                        .trim()
                        .parse()
                        .map_err(|_| format!("無効なPID: '{}'", pid_str))?;
                    opts.pid_filter.push(pid);
                }
            }
            s if s.starts_with('-') => {
                // 複合オプション処理（-bn5 など）
                for c in s[1..].chars() {
                    match c {
                        'b' => opts.batch_mode = true,
                        's' => opts.secure_mode = true,
                        'H' => opts.show_threads = true,
                        '0'..='9' | '.' => {
                            // 数字の場合は遅延時間として扱う
                            let num_start = s[1..]
                                .find(|c: char| c.is_ascii_digit() || c == '.')
                                .unwrap_or(0);
                            if let Ok(d) = s[1 + num_start..].parse::<f64>() {
                                opts.delay = d;
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                // 不明な引数は無視
            }
        }
        i += 1;
    }

    Ok(opts)
}

fn print_help() {
    println!(
        r#"top - プロセスをリアルタイムで表示

使い方: top [オプション]

Unix top互換オプション:
  -h, --help           このヘルプを表示
  -v, --version        バージョン情報を表示
  -b, --batch          バッチモード（非対話型、出力をファイルに保存可能）
  -d SEC, --delay=SEC  更新間隔を秒で指定（デフォルト: 1.0）
  -n NUM, --iterations=NUM
                       更新回数を指定（バッチモードで有用）
  -u USER, --user=USER 指定ユーザーのプロセスのみ表示
                       '*' '?' '[]' '[!...]' と '\' エスケープに対応
                       Windows版のため大文字小文字は区別しない
  -p PID, --pid=PID    指定PIDのプロセスのみ表示（カンマ区切りで複数可）
  -s, --secure         セキュアモード（プロセス終了などの操作を無効化）

GNU top拡張オプション:
  -o FIELD, --sort=FIELD
                       ソートフィールドを指定
                       FIELD: PID, %CPU, %MEM, TIME, COMMAND, USER, RES
                       '+' を付けると昇順、'-' または無しで降順
  -H, --threads        スレッド情報を表示

対話モードのキー操作:
  q, Esc, Ctrl+C   終了
  P                CPU使用率順でソート（デフォルト）
  M                メモリ使用率順でソート
  T                CPU時間順でソート
  N                PID順でソート
  R                ソート順を反転
  
  ↑↓, j/k          プロセス選択
  PgUp/PgDn        ページ移動
  Home/End         先頭/末尾へ移動
  
  K, F9            選択プロセスを終了（セキュアモードでは無効）
  r                プロセスの優先度を変更（セキュアモードでは無効）
  d, s             更新間隔を変更
  
  h, ?, F1         このヘルプを表示

表示列:
  ヘッダー  NVIDIA GPU が検出できる場合は使用率、VRAM、温度を表示
            Vulkan ランタイムが検出できる場合は API バージョンを表示
  PID      プロセスID
  USER     実行ユーザー
  PRI      優先度（Windowsの基本優先度クラス）
  %CPU     CPU使用率
  %MEM     メモリ使用率
  %GPU     GPU使用率（取得できる環境のみ）
  GMEM     GPUメモリ使用率またはGPUメモリ使用量（取得できる環境のみ）
  VSZ      仮想メモリサイズ
  RSS      物理メモリ使用量
  S        状態（R=実行中, S=スリープ）
  THR      スレッド数
  TIME+    累積CPU時間
  COMMAND  コマンド名

例:
  top                          通常の対話モード
  top -d 0.5                   0.5秒間隔で更新
  top -b -n 10                 バッチモードで10回出力
  top -u Administrator         Administratorを含むユーザーのみ
  top -u 'adm*'                adm で始まるユーザーのみ
  top -p 1234,5678             PID 1234と5678のみ表示
  top -o %MEM                  メモリ使用率順でソート
  top -b -n 1 > processes.txt  プロセス一覧をファイルに保存"#
    );
}

fn run_interactive_mode(state: &mut TopState) -> io::Result<()> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide)?;

    let result = run_top(state, &mut stdout);

    execute!(stdout, Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn run_batch_mode(state: &mut TopState) -> io::Result<()> {
    loop {
        update_data(state);
        print_batch_output(state)?;

        if let Some(ref mut n) = state.iterations_left {
            *n = n.saturating_sub(1);
            if *n == 0 {
                break;
            }
        }

        std::thread::sleep(state.refresh_interval);
    }
    Ok(())
}

fn print_batch_output(state: &TopState) -> io::Result<()> {
    let sys = &state.system;

    // ヘッダー
    println!(
        "top - {} up {}, {} tasks",
        chrono::Local::now().format("%H:%M:%S"),
        format_uptime(sys.uptime_secs),
        sys.total_procs
    );
    println!(
        "Tasks: {:>4} total, {:>3} running, {:>3} sleeping",
        sys.total_procs, sys.running_procs, sys.sleeping_procs
    );
    println!(
        "%Cpu(s): {:>5.1} us, {:>5.1} sy, {:>5.1} id",
        sys.cpu_user, sys.cpu_system, sys.cpu_idle
    );
    println!(
        "MiB Mem: {:>9} total, {:>9} free, {:>9} used",
        format_mib(sys.mem_total),
        format_mib(sys.mem_free),
        format_mib(sys.mem_used)
    );
    println!(
        "MiB Swap:{:>9} total, {:>9} free, {:>9} used",
        format_mib(sys.swap_total),
        format_mib(sys.swap_free),
        format_mib(sys.swap_used)
    );
    for gpu in &sys.gpus {
        println!("{}", format_gpu_line(gpu));
    }
    if let Some(vulkan) = &sys.vulkan {
        for line in format_vulkan_lines(vulkan) {
            println!("{}", line);
        }
    }
    println!();

    // カラムヘッダー
    println!(
        "{:>7} {:<10} {:>3} {:>5} {:>5} {:>5} {:>5} {:>9} {:>9} {:>1} {:>4} {:>9} {}",
        "PID",
        "USER",
        "PRI",
        "%CPU",
        "%MEM",
        "%GPU",
        "GMEM",
        "VSZ",
        "RSS",
        "S",
        "THR",
        "TIME+",
        "COMMAND"
    );

    // プロセス一覧
    for proc in &state.processes {
        println!(
            "{:>7} {:<10} {:>3} {:>5.1} {:>5.1} {:>5} {:>5} {:>9} {:>9} {:>1} {:>4} {:>9} {}",
            proc.pid,
            truncate_str(&proc.user, 10),
            proc.priority,
            proc.cpu_percent,
            proc.mem_percent,
            format_optional_percent(proc.gpu_percent),
            format_gpu_memory(proc.gpu_mem_percent, proc.gpu_memory_mb),
            format_kb(proc.memory_vsz),
            format_kb(proc.memory_rss),
            proc.status,
            proc.threads,
            format_time(proc.cpu_time),
            proc.name
        );
    }
    println!();

    Ok(())
}

fn run_top(state: &mut TopState, stdout: &mut io::Stdout) -> io::Result<()> {
    loop {
        update_data(state);
        draw(state, stdout)?;

        // イテレーション制限チェック
        if let Some(ref mut n) = state.iterations_left {
            *n = n.saturating_sub(1);
            if *n == 0 {
                break;
            }
        }

        if event::poll(state.refresh_interval)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }

                match handle_key(state, key, stdout)? {
                    Action::Quit => break,
                    Action::Continue => {}
                }
            }
        }
    }

    Ok(())
}

enum Action {
    Continue,
    Quit,
}

fn handle_key(state: &mut TopState, key: KeyEvent, stdout: &mut io::Stdout) -> io::Result<Action> {
    state.message = None;

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(Action::Quit)
        }

        // ソート
        KeyCode::Char('P') => {
            state.sort_key = SortKey::Cpu;
            state.sort_reverse = true;
        }
        KeyCode::Char('M') => {
            state.sort_key = SortKey::Mem;
            state.sort_reverse = true;
        }
        KeyCode::Char('T') => {
            state.sort_key = SortKey::Time;
            state.sort_reverse = true;
        }
        KeyCode::Char('N') => {
            state.sort_key = SortKey::Pid;
            state.sort_reverse = false;
        }
        KeyCode::Char('R') => {
            state.sort_reverse = !state.sort_reverse;
        }

        // スクロール
        KeyCode::Up | KeyCode::Char('k') => {
            if state.selected_index > 0 {
                state.selected_index -= 1;
                if state.selected_index < state.scroll_offset {
                    state.scroll_offset = state.selected_index;
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.selected_index < state.processes.len().saturating_sub(1) {
                state.selected_index += 1;
                let (_, rows) = terminal::size()?;
                let visible_rows = (rows as usize).saturating_sub(8);
                if state.selected_index >= state.scroll_offset + visible_rows {
                    state.scroll_offset = state.selected_index - visible_rows + 1;
                }
            }
        }
        KeyCode::PageUp => {
            let (_, rows) = terminal::size()?;
            let page = (rows as usize).saturating_sub(8);
            state.selected_index = state.selected_index.saturating_sub(page);
            state.scroll_offset = state.scroll_offset.saturating_sub(page);
        }
        KeyCode::PageDown => {
            let (_, rows) = terminal::size()?;
            let page = (rows as usize).saturating_sub(8);
            state.selected_index =
                (state.selected_index + page).min(state.processes.len().saturating_sub(1));
            state.scroll_offset =
                (state.scroll_offset + page).min(state.processes.len().saturating_sub(1));
        }
        KeyCode::Home => {
            state.selected_index = 0;
            state.scroll_offset = 0;
        }
        KeyCode::End => {
            state.selected_index = state.processes.len().saturating_sub(1);
            let (_, rows) = terminal::size()?;
            let visible_rows = (rows as usize).saturating_sub(8);
            state.scroll_offset = state.processes.len().saturating_sub(visible_rows);
        }

        // Kill（セキュアモードでは無効）
        KeyCode::Char('K') | KeyCode::F(9) => {
            if state.secure_mode {
                state.message = Some("セキュアモードでは無効です".to_string());
            } else if let Some(proc) = state.processes.get(state.selected_index) {
                let pid = proc.pid;
                let name = proc.name.clone();
                if kill_process(pid) {
                    state.message = Some(format!("プロセス {} ({}) を終了しました", pid, name));
                } else {
                    state.message = Some(format!("プロセス {} の終了に失敗しました", pid));
                }
            }
        }

        // Renice（セキュアモードでは無効）
        KeyCode::Char('r') => {
            if state.secure_mode {
                state.message = Some("セキュアモードでは無効です".to_string());
            } else if let Some(proc) = state.processes.get(state.selected_index) {
                renice_dialog(state, proc.pid, stdout)?;
            }
        }

        // ヘルプ
        KeyCode::Char('h') | KeyCode::Char('?') | KeyCode::F(1) => {
            state.show_help = !state.show_help;
        }

        // 更新間隔
        KeyCode::Char('d') | KeyCode::Char('s') => {
            change_delay_dialog(state, stdout)?;
        }

        // ユーザーフィルタ
        KeyCode::Char('u') | KeyCode::Char('U') => {
            user_filter_dialog(state, stdout)?;
        }

        _ => {}
    }

    Ok(Action::Continue)
}

fn update_data(state: &mut TopState) {
    // CPU使用率計算
    let current_cpu = get_cpu_times();
    let cpu_delta_idle = current_cpu.idle.saturating_sub(state.prev_cpu_times.idle);
    let cpu_delta_kernel = current_cpu
        .kernel
        .saturating_sub(state.prev_cpu_times.kernel);
    let cpu_delta_user = current_cpu.user.saturating_sub(state.prev_cpu_times.user);
    let cpu_total = cpu_delta_idle + cpu_delta_kernel + cpu_delta_user;

    let (cpu_idle, cpu_system, cpu_user) = if cpu_total > 0 {
        (
            (cpu_delta_idle as f64 / cpu_total as f64) * 100.0,
            (cpu_delta_kernel as f64 / cpu_total as f64) * 100.0,
            (cpu_delta_user as f64 / cpu_total as f64) * 100.0,
        )
    } else {
        (100.0, 0.0, 0.0)
    };

    state.prev_cpu_times = current_cpu;

    // メモリ情報
    let (mem_total, mem_free, swap_total, swap_free) = get_memory_info();

    // プロセス一覧
    let now = Instant::now();
    let gpu_processes = get_gpu_process_info();
    let mut processes = get_processes(&state.prev_proc_times, &gpu_processes, now);

    // フィルタリング
    if let Some(ref user) = state.user_filter {
        processes.retain(|p| matches_filter_pattern(user, &p.user));
    }

    if !state.pid_filter.is_empty() {
        processes.retain(|p| state.pid_filter.contains(&p.pid));
    }

    // プロセス別CPU時間を保存
    let mut new_proc_times = HashMap::new();
    for proc in &processes {
        new_proc_times.insert(proc.pid, (proc.kernel_time, proc.user_time, now));
    }
    state.prev_proc_times = new_proc_times;

    // 統計
    let running = processes.iter().filter(|p| p.status == "R").count();
    let sleeping = processes.len() - running;

    // ソート
    sort_processes(&mut processes, state.sort_key, state.sort_reverse);

    state.processes = processes;
    state.system = SystemInfo {
        uptime_secs: unsafe { GetTickCount64() / 1000 },
        total_procs: state.processes.len(),
        running_procs: running,
        sleeping_procs: sleeping,
        cpu_user,
        cpu_system,
        cpu_idle,
        mem_total,
        mem_used: mem_total - mem_free,
        mem_free,
        swap_total,
        swap_used: swap_total - swap_free,
        swap_free,
        gpus: get_gpu_info(),
        vulkan: state.vulkan_info.clone(),
    };
}

type VkEnumerateInstanceVersion = unsafe extern "system" fn(*mut u32) -> i32;

const VK_SUCCESS: i32 = 0;

fn get_vulkan_info() -> Option<VulkanInfo> {
    let library = unsafe { LoadLibraryA(PCSTR(c"vulkan-1.dll".as_ptr().cast())) }.ok()?;

    let version = unsafe {
        let proc = GetProcAddress(
            library,
            PCSTR(c"vkEnumerateInstanceVersion".as_ptr().cast()),
        );
        match proc {
            Some(proc) => {
                let enumerate_instance_version: VkEnumerateInstanceVersion = mem::transmute(proc);
                let mut raw_version = 0;
                if enumerate_instance_version(&mut raw_version) == VK_SUCCESS {
                    Some(VulkanVersion::from_raw(raw_version))
                } else {
                    None
                }
            }
            None => Some(VulkanVersion {
                major: 1,
                minor: 0,
                patch: 0,
            }),
        }
    };

    unsafe {
        let _ = FreeLibrary(library);
    }

    version.map(|api_version| VulkanInfo {
        api_version,
        devices: get_vulkan_devices_from_summary(),
    })
}

fn get_vulkan_devices_from_summary() -> Vec<VulkanDeviceInfo> {
    let output = match Command::new("vulkaninfo").arg("--summary").output() {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    parse_vulkaninfo_summary(&String::from_utf8_lossy(&output.stdout))
}

fn parse_vulkaninfo_summary(summary: &str) -> Vec<VulkanDeviceInfo> {
    let mut devices = Vec::new();
    let mut current: Option<VulkanDeviceInfo> = None;

    for line in summary.lines() {
        let trimmed = line.trim();
        if let Some(index) = parse_vulkan_device_header(trimmed) {
            if let Some(device) = current.take() {
                devices.push(device);
            }
            current = Some(VulkanDeviceInfo {
                index,
                ..Default::default()
            });
            continue;
        }

        let Some(device) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "apiVersion" => device.api_version = Some(value.to_string()),
            "deviceType" => device.device_type = Some(format_vulkan_device_type(value)),
            "deviceName" => device.name = Some(value.to_string()),
            "driverName" => device.driver_name = Some(value.to_string()),
            "driverInfo" => device.driver_info = Some(value.to_string()),
            _ => {}
        }
    }

    if let Some(device) = current {
        devices.push(device);
    }

    devices
}

fn parse_vulkan_device_header(line: &str) -> Option<usize> {
    line.strip_prefix("GPU")
        .and_then(|rest| rest.strip_suffix(':'))
        .and_then(|index| index.parse().ok())
}

fn format_vulkan_device_type(device_type: &str) -> String {
    device_type
        .strip_prefix("PHYSICAL_DEVICE_TYPE_")
        .unwrap_or(device_type)
        .replace('_', " ")
        .to_ascii_lowercase()
}

fn get_gpu_info() -> Vec<GpuInfo> {
    get_nvidia_gpu_info()
}

fn get_nvidia_gpu_info() -> Vec<GpuInfo> {
    let output = match run_nvidia_smi(&[
        "--query-gpu=name,utilization.gpu,memory.total,memory.used,temperature.gpu",
        "--format=csv,noheader,nounits",
    ]) {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(parse_nvidia_smi_gpu_line)
        .collect()
}

fn get_gpu_process_info() -> HashMap<u32, GpuProcessInfo> {
    let mut processes = get_nvidia_pmon_process_info();
    for (pid, info) in get_nvidia_compute_process_info() {
        processes.entry(pid).or_insert(info);
    }
    processes
}

fn get_nvidia_pmon_process_info() -> HashMap<u32, GpuProcessInfo> {
    let output = match run_nvidia_smi(&["pmon", "-c", "1"]) {
        Ok(output) if output.status.success() => output,
        _ => return HashMap::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(parse_nvidia_pmon_process_line)
        .collect()
}

fn parse_nvidia_pmon_process_line(line: &str) -> Option<(u32, GpuProcessInfo)> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() < 5 || line.trim_start().starts_with('#') {
        return None;
    }

    let pid = fields[1].parse().ok()?;
    let gpu_percent = parse_optional_f64(fields[3]);
    let gpu_mem_percent = parse_optional_f64(fields[4]);

    Some((
        pid,
        GpuProcessInfo {
            gpu_percent,
            gpu_mem_percent,
            gpu_memory_mb: None,
        },
    ))
}

fn get_nvidia_compute_process_info() -> HashMap<u32, GpuProcessInfo> {
    let output = match run_nvidia_smi(&[
        "--query-compute-apps=pid,used_memory",
        "--format=csv,noheader,nounits",
    ]) {
        Ok(output) if output.status.success() => output,
        _ => return HashMap::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(parse_nvidia_compute_process_line)
        .collect()
}

fn parse_nvidia_compute_process_line(line: &str) -> Option<(u32, GpuProcessInfo)> {
    let fields: Vec<_> = line.split(',').map(str::trim).collect();
    if fields.len() < 2 {
        return None;
    }

    Some((
        fields[0].parse().ok()?,
        GpuProcessInfo {
            gpu_percent: None,
            gpu_mem_percent: None,
            gpu_memory_mb: parse_optional_u64(fields[1]),
        },
    ))
}

fn run_nvidia_smi(args: &[&str]) -> io::Result<std::process::Output> {
    let mut last_error = None;
    for command in nvidia_smi_candidates() {
        match Command::new(&command).args(args).output() {
            Ok(output) => return Ok(output),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "nvidia-smi was not found")))
}

fn nvidia_smi_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from("nvidia-smi")];
    if let Some(program_files) = env::var_os("ProgramFiles") {
        candidates.push(
            Path::new(&program_files)
                .join("NVIDIA Corporation")
                .join("NVSMI")
                .join("nvidia-smi.exe"),
        );
    }
    candidates.push(PathBuf::from(
        r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
    ));
    candidates.push(PathBuf::from(r"C:\Windows\System32\nvidia-smi.exe"));
    candidates
}

fn parse_nvidia_smi_gpu_line(line: &str) -> Option<GpuInfo> {
    let fields: Vec<_> = line.split(',').map(str::trim).collect();
    if fields.len() < 5 || fields[0].is_empty() {
        return None;
    }

    Some(GpuInfo {
        name: fields[0].to_string(),
        utilization_percent: parse_optional_f64(fields[1]),
        memory_total_mb: parse_optional_u64(fields[2]),
        memory_used_mb: parse_optional_u64(fields[3]),
        temperature_c: parse_optional_u32(fields[4]),
    })
}

fn parse_optional_f64(value: &str) -> Option<f64> {
    if value.eq_ignore_ascii_case("[not supported]") || value.eq_ignore_ascii_case("N/A") {
        None
    } else {
        value.parse().ok()
    }
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    if value.eq_ignore_ascii_case("[not supported]") || value.eq_ignore_ascii_case("N/A") {
        None
    } else {
        value.parse().ok()
    }
}

fn parse_optional_u32(value: &str) -> Option<u32> {
    if value.eq_ignore_ascii_case("[not supported]") || value.eq_ignore_ascii_case("N/A") {
        None
    } else {
        value.parse().ok()
    }
}

fn get_cpu_times() -> CpuTimes {
    unsafe {
        let mut idle_time = mem::zeroed();
        let mut kernel_time = mem::zeroed();
        let mut user_time = mem::zeroed();

        if GetSystemTimes(
            Some(&mut idle_time),
            Some(&mut kernel_time),
            Some(&mut user_time),
        )
        .is_ok()
        {
            let idle = ((idle_time.dwHighDateTime as u64) << 32) | (idle_time.dwLowDateTime as u64);
            let kernel =
                ((kernel_time.dwHighDateTime as u64) << 32) | (kernel_time.dwLowDateTime as u64);
            let user = ((user_time.dwHighDateTime as u64) << 32) | (user_time.dwLowDateTime as u64);

            CpuTimes {
                idle,
                kernel: kernel - idle,
                user,
            }
        } else {
            CpuTimes {
                idle: 0,
                kernel: 0,
                user: 0,
            }
        }
    }
}

fn get_memory_info() -> (u64, u64, u64, u64) {
    unsafe {
        let mut mem_status: MEMORYSTATUSEX = mem::zeroed();
        mem_status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;

        if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
            let total = mem_status.ullTotalPhys / 1024;
            let free = mem_status.ullAvailPhys / 1024;
            let swap_total = mem_status.ullTotalPageFile / 1024;
            let swap_free = mem_status.ullAvailPageFile / 1024;
            (total, free, swap_total, swap_free)
        } else {
            (0, 0, 0, 0)
        }
    }
}

fn get_processes(
    prev_times: &HashMap<u32, (u64, u64, Instant)>,
    gpu_processes: &HashMap<u32, GpuProcessInfo>,
    now: Instant,
) -> Vec<ProcessInfo> {
    let mut processes = Vec::new();
    let (mem_total, _, _, _) = get_memory_info();

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return processes,
        };

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

                let details = get_process_details(entry.th32ProcessID, mem_total, prev_times, now);
                let gpu_info = gpu_processes.get(&entry.th32ProcessID);

                processes.push(ProcessInfo {
                    pid: entry.th32ProcessID,
                    name,
                    user: details.user,
                    threads: entry.cntThreads,
                    priority: entry.pcPriClassBase as i32,
                    memory_rss: details.rss,
                    memory_vsz: details.vsz,
                    cpu_percent: details.cpu_percent,
                    mem_percent: details.mem_percent,
                    gpu_percent: gpu_info.and_then(|info| info.gpu_percent),
                    gpu_mem_percent: gpu_info.and_then(|info| info.gpu_mem_percent),
                    gpu_memory_mb: gpu_info.and_then(|info| info.gpu_memory_mb),
                    cpu_time: details.cpu_time,
                    status: details.status,
                    kernel_time: details.kernel_time,
                    user_time: details.user_time,
                });

                if Process32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }

    processes
}

struct ProcessDetails {
    user: String,
    rss: u64,
    vsz: u64,
    cpu_percent: f64,
    mem_percent: f64,
    cpu_time: u64,
    status: String,
    kernel_time: u64,
    user_time: u64,
}

fn get_process_details(
    pid: u32,
    mem_total: u64,
    prev_times: &HashMap<u32, (u64, u64, Instant)>,
    now: Instant,
) -> ProcessDetails {
    let mut details = ProcessDetails {
        user: String::new(),
        rss: 0,
        vsz: 0,
        cpu_percent: 0.0,
        mem_percent: 0.0,
        cpu_time: 0,
        status: "S".to_string(),
        kernel_time: 0,
        user_time: 0,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid);

        if let Ok(handle) = handle {
            // メモリ
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
                details.vsz = counters.PrivateUsage as u64 / 1024;

                if mem_total > 0 {
                    details.mem_percent = (details.rss as f64 / mem_total as f64) * 100.0;
                }
            }

            // CPU時間
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

                details.kernel_time = kernel;
                details.user_time = user;
                details.cpu_time = (kernel + user) / 10_000;

                // CPU使用率計算
                if let Some(&(prev_kernel, prev_user, prev_time)) = prev_times.get(&pid) {
                    let elapsed = now.duration_since(prev_time).as_micros() as u64;
                    if elapsed > 0 {
                        let delta_kernel = kernel.saturating_sub(prev_kernel);
                        let delta_user = user.saturating_sub(prev_user);
                        let delta_total = delta_kernel + delta_user;
                        // 100ナノ秒単位 -> マイクロ秒
                        let cpu_time_us = delta_total / 10;
                        details.cpu_percent = (cpu_time_us as f64 / elapsed as f64) * 100.0;
                    }
                }

                if details.cpu_percent > 0.1 {
                    details.status = "R".to_string();
                }
            }

            details.user = get_process_user(handle);
            let _ = CloseHandle(handle);
        }
    }

    details
}

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

fn sort_processes(processes: &mut Vec<ProcessInfo>, key: SortKey, reverse: bool) {
    processes.sort_by(|a, b| {
        let cmp = match key {
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::Cpu => a
                .cpu_percent
                .partial_cmp(&b.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Mem => a
                .mem_percent
                .partial_cmp(&b.mem_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Res => a.memory_rss.cmp(&b.memory_rss),
            SortKey::Time => a.cpu_time.cmp(&b.cpu_time),
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::User => a.user.to_lowercase().cmp(&b.user.to_lowercase()),
        };

        if reverse {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

fn draw(state: &TopState, stdout: &mut io::Stdout) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;

    execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

    // ヘルプ画面
    if state.show_help {
        draw_help(stdout)?;
        return Ok(());
    }

    let header_height = header_height(&state.system);

    // システム情報ヘッダー
    draw_header(stdout, &state.system, cols)?;

    // カラムヘッダー（1行）
    draw_column_header(stdout, state.sort_key, cols, header_height)?;

    // プロセス一覧
    let process_start_row = header_height + 1;
    let visible_rows = (rows as usize).saturating_sub(process_start_row as usize + 1);
    let end_index = (state.scroll_offset + visible_rows).min(state.processes.len());

    for (i, proc) in state.processes[state.scroll_offset..end_index]
        .iter()
        .enumerate()
    {
        let row = process_start_row + i as u16;
        let is_selected = state.scroll_offset + i == state.selected_index;

        execute!(stdout, MoveTo(0, row))?;

        if is_selected {
            execute!(
                stdout,
                SetBackgroundColor(Color::DarkBlue),
                SetForegroundColor(Color::White)
            )?;
        }

        let line = format!(
            "{:>7} {:<10} {:>3} {:>5.1} {:>5.1} {:>5} {:>5} {:>9} {:>9} {:>1} {:>4} {:>9} {}",
            proc.pid,
            truncate_str(&proc.user, 10),
            proc.priority,
            proc.cpu_percent,
            proc.mem_percent,
            format_optional_percent(proc.gpu_percent),
            format_gpu_memory(proc.gpu_mem_percent, proc.gpu_memory_mb),
            format_kb(proc.memory_vsz),
            format_kb(proc.memory_rss),
            proc.status,
            proc.threads,
            format_time(proc.cpu_time),
            truncate_str(&proc.name, (cols as usize).saturating_sub(87))
        );

        execute!(
            stdout,
            Print(format!("{:<width$}", line, width = cols as usize))
        )?;

        if is_selected {
            execute!(stdout, ResetColor)?;
        }
    }

    // メッセージ
    if let Some(ref msg) = state.message {
        execute!(
            stdout,
            MoveTo(0, rows - 1),
            SetBackgroundColor(Color::Yellow),
            SetForegroundColor(Color::Black),
            Print(format!("{:<width$}", msg, width = cols as usize)),
            ResetColor
        )?;
    }

    stdout.flush()?;
    Ok(())
}

fn header_height(sys: &SystemInfo) -> u16 {
    6 + sys.gpus.len() as u16 + sys.vulkan.as_ref().map(vulkan_line_count).unwrap_or(0) as u16
}

fn draw_header(stdout: &mut io::Stdout, sys: &SystemInfo, cols: u16) -> io::Result<()> {
    let uptime = format_uptime(sys.uptime_secs);

    // 1行目: top情報
    execute!(
        stdout,
        MoveTo(0, 0),
        SetForegroundColor(Color::White),
        Print(format!(
            "top - {} up {}, {} tasks",
            chrono::Local::now().format("%H:%M:%S"),
            uptime,
            sys.total_procs
        )),
        ResetColor
    )?;

    // 2行目: タスク情報
    execute!(
        stdout,
        MoveTo(0, 1),
        Print(format!(
            "Tasks: {:>4} total, {:>3} running, {:>3} sleeping",
            sys.total_procs, sys.running_procs, sys.sleeping_procs
        ))
    )?;

    // 3行目: CPU情報
    execute!(
        stdout,
        MoveTo(0, 2),
        Print(format!(
            "%Cpu(s): {:>5.1} us, {:>5.1} sy, {:>5.1} id",
            sys.cpu_user, sys.cpu_system, sys.cpu_idle
        ))
    )?;

    // 4行目: メモリ情報
    execute!(
        stdout,
        MoveTo(0, 3),
        Print(format!(
            "MiB Mem: {:>9} total, {:>9} free, {:>9} used",
            format_mib(sys.mem_total),
            format_mib(sys.mem_free),
            format_mib(sys.mem_used)
        ))
    )?;

    // 5行目: スワップ情報
    execute!(
        stdout,
        MoveTo(0, 4),
        Print(format!(
            "MiB Swap:{:>9} total, {:>9} free, {:>9} used",
            format_mib(sys.swap_total),
            format_mib(sys.swap_free),
            format_mib(sys.swap_used)
        ))
    )?;

    for (i, gpu) in sys.gpus.iter().enumerate() {
        execute!(
            stdout,
            MoveTo(0, 5 + i as u16),
            Print(truncate_str(&format_gpu_line(gpu), cols as usize))
        )?;
    }

    if let Some(vulkan) = &sys.vulkan {
        for (i, line) in format_vulkan_lines(vulkan).iter().enumerate() {
            execute!(
                stdout,
                MoveTo(0, 5 + sys.gpus.len() as u16 + i as u16),
                Print(truncate_str(line, cols as usize))
            )?;
        }
    }

    // 空行
    execute!(stdout, MoveTo(0, header_height(sys) - 1), Print(""))?;

    Ok(())
}

fn draw_column_header(
    stdout: &mut io::Stdout,
    sort_key: SortKey,
    cols: u16,
    row: u16,
) -> io::Result<()> {
    execute!(
        stdout,
        MoveTo(0, row),
        SetBackgroundColor(Color::Green),
        SetForegroundColor(Color::Black)
    )?;

    let header = format!(
        "{:>7} {:<10} {:>3} {:>5} {:>5} {:>5} {:>5} {:>9} {:>9} {:>1} {:>4} {:>9} {}",
        "PID",
        if sort_key == SortKey::User {
            "USER*"
        } else {
            "USER"
        },
        "PRI",
        if sort_key == SortKey::Cpu {
            "%CPU*"
        } else {
            "%CPU"
        },
        if sort_key == SortKey::Mem {
            "%MEM*"
        } else {
            "%MEM"
        },
        "%GPU",
        "GMEM",
        "VSZ",
        if sort_key == SortKey::Res {
            "RSS*"
        } else {
            "RSS"
        },
        "S",
        "THR",
        if sort_key == SortKey::Time {
            "TIME+*"
        } else {
            "TIME+"
        },
        if sort_key == SortKey::Name {
            "COMMAND*"
        } else {
            "COMMAND"
        }
    );

    execute!(
        stdout,
        Print(format!("{:<width$}", header, width = cols as usize)),
        ResetColor
    )?;

    Ok(())
}

fn draw_help(stdout: &mut io::Stdout) -> io::Result<()> {
    let help_text = vec![
        "top - ヘルプ",
        "",
        "キー操作:",
        "  q, Esc, Ctrl+C  終了",
        "  P               CPU使用率順でソート",
        "  M               メモリ使用率順でソート",
        "  T               CPU時間順でソート",
        "  N               PID順でソート",
        "  R               ソート順を反転",
        "  ↑↓, j/k         プロセス選択",
        "  PgUp/PgDn       ページ移動",
        "  Home/End        先頭/末尾へ",
        "  K, F9           選択プロセスを終了",
        "  r               優先度変更",
        "  d, s            更新間隔変更",
        "  u, U            ユーザーフィルタ (* ? [] 対応, 大文字小文字無視)",
        "  h, ?, F1        このヘルプ",
        "",
        "GPU:",
        "  nvidia-smi が利用できる環境では使用率、VRAM、温度をヘッダーに表示",
        "  nvidia-smi pmon が利用できる環境ではプロセス別GPU使用率を表示",
        "  vulkaninfo が利用できる環境では Vulkan デバイス詳細もヘッダーに表示",
        "",
        "何かキーを押すと戻ります...",
    ];

    for (i, line) in help_text.iter().enumerate() {
        execute!(stdout, MoveTo(2, i as u16 + 1), Print(line))?;
    }

    stdout.flush()?;
    Ok(())
}

fn kill_process(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid);

        if let Ok(handle) = handle {
            let result = TerminateProcess(handle, 1).is_ok();
            let _ = CloseHandle(handle);
            result
        } else {
            false
        }
    }
}

fn renice_dialog(state: &mut TopState, pid: u32, stdout: &mut io::Stdout) -> io::Result<()> {
    let (_cols, rows) = terminal::size()?;

    execute!(
        stdout,
        MoveTo(0, rows - 1),
        Clear(ClearType::CurrentLine),
        Print("優先度 (1:低, 2:通常以下, 3:通常, 4:通常以上, 5:高): ")
    )?;
    stdout.flush()?;

    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind != crossterm::event::KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char(c) if ('1'..='5').contains(&c) => {
                    let priority = match c {
                        '1' => IDLE_PRIORITY_CLASS,
                        '2' => BELOW_NORMAL_PRIORITY_CLASS,
                        '3' => NORMAL_PRIORITY_CLASS,
                        '4' => ABOVE_NORMAL_PRIORITY_CLASS,
                        '5' => HIGH_PRIORITY_CLASS,
                        _ => NORMAL_PRIORITY_CLASS,
                    };

                    if set_process_priority(pid, priority) {
                        state.message = Some(format!("PID {} の優先度を変更しました", pid));
                    } else {
                        state.message = Some("優先度の変更に失敗しました".to_string());
                    }
                    break;
                }
                KeyCode::Esc => break,
                _ => {}
            }
        }
    }

    Ok(())
}

fn set_process_priority(
    pid: u32,
    priority: windows::Win32::System::Threading::PROCESS_CREATION_FLAGS,
) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid);

        if let Ok(handle) = handle {
            let result = SetPriorityClass(handle, priority).is_ok();
            let _ = CloseHandle(handle);
            result
        } else {
            false
        }
    }
}

fn change_delay_dialog(state: &mut TopState, stdout: &mut io::Stdout) -> io::Result<()> {
    let (_cols, rows) = terminal::size()?;

    execute!(
        stdout,
        MoveTo(0, rows - 1),
        Clear(ClearType::CurrentLine),
        Print("更新間隔(秒): ")
    )?;
    stdout.flush()?;

    let mut input = String::new();

    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind != crossterm::event::KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Enter => {
                    if let Ok(secs) = input.parse::<f64>() {
                        if secs >= 0.1 && secs <= 60.0 {
                            state.refresh_interval = Duration::from_secs_f64(secs);
                            state.message = Some(format!("更新間隔: {}秒", secs));
                        }
                    }
                    break;
                }
                KeyCode::Esc => break,
                KeyCode::Backspace => {
                    input.pop();
                    execute!(
                        stdout,
                        MoveTo(0, rows - 1),
                        Clear(ClearType::CurrentLine),
                        Print(format!("更新間隔(秒): {}", input))
                    )?;
                    stdout.flush()?;
                }
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    input.push(c);
                    execute!(stdout, Print(c))?;
                    stdout.flush()?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn user_filter_dialog(state: &mut TopState, stdout: &mut io::Stdout) -> io::Result<()> {
    let (_cols, rows) = terminal::size()?;

    execute!(
        stdout,
        MoveTo(0, rows - 1),
        Clear(ClearType::CurrentLine),
        Print("ユーザー名/パターン (空でフィルタ解除): ")
    )?;
    stdout.flush()?;

    let mut input = String::new();

    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind != crossterm::event::KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Enter => {
                    if input.is_empty() {
                        state.user_filter = None;
                        state.message = Some("ユーザーフィルタを解除しました".to_string());
                    } else {
                        state.user_filter = Some(input.clone());
                        state.message =
                            Some(format!("ユーザー/パターン '{}' でフィルタリング", input));
                    }
                    break;
                }
                KeyCode::Esc => break,
                KeyCode::Backspace => {
                    input.pop();
                    execute!(
                        stdout,
                        MoveTo(0, rows - 1),
                        Clear(ClearType::CurrentLine),
                        Print(format!("ユーザー名/パターン (空でフィルタ解除): {}", input))
                    )?;
                    stdout.flush()?;
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    execute!(stdout, Print(c))?;
                    stdout.flush()?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    if days > 0 {
        format!("{} days, {:02}:{:02}", days, hours, mins)
    } else {
        format!("{:02}:{:02}", hours, mins)
    }
}

fn format_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    let hundredths = (ms % 1000) / 10;

    format!("{}:{:02}.{:02}", mins, secs, hundredths)
}

fn format_kb(kb: u64) -> String {
    if kb >= 1048576 {
        format!("{:.1}G", kb as f64 / 1048576.0)
    } else if kb >= 1024 {
        format!("{:.1}M", kb as f64 / 1024.0)
    } else {
        format!("{}K", kb)
    }
}

fn format_mib(kb: u64) -> String {
    format!("{:.1}", kb as f64 / 1024.0)
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.0}", value))
        .unwrap_or_else(|| "-".to_string())
}

fn format_gpu_memory(percent: Option<f64>, memory_mb: Option<u64>) -> String {
    if let Some(percent) = percent {
        format!("{:.0}", percent)
    } else if let Some(memory_mb) = memory_mb {
        if memory_mb >= 1024 {
            format!("{:.1}G", memory_mb as f64 / 1024.0)
        } else {
            format!("{}M", memory_mb)
        }
    } else {
        "-".to_string()
    }
}

fn format_gpu_line(gpu: &GpuInfo) -> String {
    let utilization = gpu
        .utilization_percent
        .map(|value| format!("{:>5.1}%", value))
        .unwrap_or_else(|| "  N/A ".to_string());
    let memory = match (gpu.memory_used_mb, gpu.memory_total_mb) {
        (Some(used), Some(total)) => format!("{}/{} MiB", used, total),
        _ => "N/A".to_string(),
    };
    let temperature = gpu
        .temperature_c
        .map(|value| format!("{} C", value))
        .unwrap_or_else(|| "N/A".to_string());

    format!(
        "%Gpu: {}  util {}, mem {}, temp {}",
        gpu.name, utilization, memory, temperature
    )
}

fn vulkan_line_count(vulkan: &VulkanInfo) -> usize {
    1 + vulkan.devices.len()
}

fn format_vulkan_lines(vulkan: &VulkanInfo) -> Vec<String> {
    let mut lines = Vec::with_capacity(vulkan_line_count(vulkan));
    if vulkan.devices.is_empty() {
        lines.push(format!("Vulkan: API {}", vulkan.api_version));
        return lines;
    }

    lines.push(format!(
        "Vulkan: API {}, {} device(s)",
        vulkan.api_version,
        vulkan.devices.len()
    ));
    for device in &vulkan.devices {
        lines.push(format_vulkan_device_line(device));
    }
    lines
}

fn format_vulkan_device_line(device: &VulkanDeviceInfo) -> String {
    let name = device.name.as_deref().unwrap_or("unknown");
    let device_type = device.device_type.as_deref().unwrap_or("unknown");
    let api_version = device.api_version.as_deref().unwrap_or("N/A");
    let driver = match (device.driver_name.as_deref(), device.driver_info.as_deref()) {
        (Some(name), Some(info)) => format!("{} {}", name, info),
        (Some(name), None) => name.to_string(),
        (None, Some(info)) => info.to_string(),
        (None, None) => "N/A".to_string(),
    };

    format!(
        "%Vk{}: {}, {}, API {}, driver {}",
        device.index, name, device_type, api_version, driver
    )
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars().take(max_len).collect()
    }
}

impl VulkanVersion {
    fn from_raw(version: u32) -> Self {
        Self {
            major: version >> 22,
            minor: (version >> 12) & 0x3ff,
            patch: version & 0xfff,
        }
    }
}

impl std::fmt::Display for VulkanVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn matches_filter_pattern(pattern: &str, value: &str) -> bool {
    if has_glob_metachar(pattern) {
        matches_posix_glob_case_insensitive(pattern, value)
    } else {
        value.to_lowercase().contains(&pattern.to_lowercase())
    }
}

fn has_glob_metachar(pattern: &str) -> bool {
    let mut escaped = false;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '*' | '?' | '[' => return true,
            _ => {}
        }
    }
    false
}

fn matches_posix_glob_case_insensitive(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let value: Vec<char> = value.chars().flat_map(char::to_lowercase).collect();
    glob_match_recursive(&pattern, 0, &value, 0)
}

fn glob_match_recursive(pattern: &[char], pi: usize, value: &[char], vi: usize) -> bool {
    if pi == pattern.len() {
        return vi == value.len();
    }

    match pattern[pi] {
        '*' => {
            let mut next = pi + 1;
            while next < pattern.len() && pattern[next] == '*' {
                next += 1;
            }
            if next == pattern.len() {
                return true;
            }
            for idx in vi..=value.len() {
                if glob_match_recursive(pattern, next, value, idx) {
                    return true;
                }
            }
            false
        }
        '?' => vi < value.len() && glob_match_recursive(pattern, pi + 1, value, vi + 1),
        '[' => match parse_char_class(pattern, pi) {
            Some((char_class, next_pi)) => {
                vi < value.len()
                    && char_class.matches(value[vi])
                    && glob_match_recursive(pattern, next_pi, value, vi + 1)
            }
            None => {
                vi < value.len()
                    && value[vi] == '['
                    && glob_match_recursive(pattern, pi + 1, value, vi + 1)
            }
        },
        '\\' => {
            let next_char = pattern.get(pi + 1).copied().unwrap_or('\\');
            let advance = usize::from(pi + 1 < pattern.len()) + 1;
            vi < value.len()
                && value[vi] == next_char
                && glob_match_recursive(pattern, pi + advance, value, vi + 1)
        }
        literal => {
            vi < value.len()
                && value[vi] == literal
                && glob_match_recursive(pattern, pi + 1, value, vi + 1)
        }
    }
}

#[derive(Debug)]
struct CharClass {
    negated: bool,
    items: Vec<ClassItem>,
}

impl CharClass {
    fn matches(&self, c: char) -> bool {
        let matched = self.items.iter().any(|item| item.matches(c));
        if self.negated {
            !matched
        } else {
            matched
        }
    }
}

#[derive(Debug)]
enum ClassItem {
    Single(char),
    Range(char, char),
}

impl ClassItem {
    fn matches(&self, c: char) -> bool {
        match *self {
            ClassItem::Single(single) => c == single,
            ClassItem::Range(start, end) => start <= c && c <= end,
        }
    }
}

fn parse_char_class(pattern: &[char], start: usize) -> Option<(CharClass, usize)> {
    if pattern.get(start) != Some(&'[') {
        return None;
    }

    let mut i = start + 1;
    let mut negated = false;
    if matches!(pattern.get(i), Some('!') | Some('^')) {
        negated = true;
        i += 1;
    }

    let mut items = Vec::new();
    let mut first = true;
    while i < pattern.len() {
        let current = pattern[i];
        if current == ']' && !first {
            return Some((CharClass { negated, items }, i + 1));
        }

        let (first_char, after_first) = parse_class_char(pattern, i);
        if after_first < pattern.len()
            && pattern[after_first] == '-'
            && after_first + 1 < pattern.len()
            && pattern[after_first + 1] != ']'
        {
            let (last_char, after_last) = parse_class_char(pattern, after_first + 1);
            items.push(ClassItem::Range(first_char, last_char));
            i = after_last;
        } else {
            items.push(ClassItem::Single(first_char));
            i = after_first;
        }
        first = false;
    }

    None
}

fn parse_class_char(pattern: &[char], start: usize) -> (char, usize) {
    match pattern.get(start) {
        Some('\\') if start + 1 < pattern.len() => (pattern[start + 1], start + 2),
        Some(c) => (*c, start + 1),
        None => ('\\', start),
    }
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            uptime_secs: 0,
            total_procs: 0,
            running_procs: 0,
            sleeping_procs: 0,
            cpu_user: 0.0,
            cpu_system: 0.0,
            cpu_idle: 100.0,
            mem_total: 0,
            mem_used: 0,
            mem_free: 0,
            swap_total: 0,
            swap_used: 0,
            swap_free: 0,
            gpus: Vec::new(),
            vulkan: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_gpu_line, format_gpu_memory, format_optional_percent, format_vulkan_lines,
        has_glob_metachar, matches_filter_pattern, matches_posix_glob_case_insensitive,
        parse_nvidia_pmon_process_line, parse_nvidia_smi_gpu_line, parse_vulkaninfo_summary,
        GpuInfo, VulkanInfo, VulkanVersion,
    };

    #[test]
    fn plain_filter_remains_case_insensitive_substring() {
        assert!(matches_filter_pattern("adm", "Administrator"));
        assert!(matches_filter_pattern("ADMIN", "administrator"));
        assert!(!matches_filter_pattern("svc", "administrator"));
    }

    #[test]
    fn glob_filter_supports_posix_style_wildcards() {
        assert!(matches_posix_glob_case_insensitive("adm*", "Administrator"));
        assert!(matches_posix_glob_case_insensitive("a?m*", "AdMin"));
        assert!(matches_posix_glob_case_insensitive(
            "[!x]dmin*",
            "Administrator"
        ));
        assert!(!matches_posix_glob_case_insensitive(
            "[!a]*",
            "Administrator"
        ));
    }

    #[test]
    fn glob_filter_supports_bracket_ranges_and_escapes() {
        assert!(matches_posix_glob_case_insensitive("user[0-9]", "USER7"));
        assert!(matches_posix_glob_case_insensitive(r"file\[*", "file[abc"));
        assert!(!matches_posix_glob_case_insensitive(r"file\[*", "filex"));
    }

    #[test]
    fn glob_meta_detection_honors_escapes() {
        assert!(has_glob_metachar("adm*"));
        assert!(has_glob_metachar("user[0-9]"));
        assert!(!has_glob_metachar(r"user\*literal"));
    }

    #[test]
    fn nvidia_smi_gpu_line_is_parsed() {
        let gpu = parse_nvidia_smi_gpu_line("NVIDIA GeForce RTX 4070, 12, 12282, 2048, 54")
            .expect("GPU line should parse");

        assert_eq!(gpu.name, "NVIDIA GeForce RTX 4070");
        assert_eq!(gpu.utilization_percent, Some(12.0));
        assert_eq!(gpu.memory_total_mb, Some(12282));
        assert_eq!(gpu.memory_used_mb, Some(2048));
        assert_eq!(gpu.temperature_c, Some(54));
    }

    #[test]
    fn gpu_line_formats_missing_values_as_na() {
        let line = format_gpu_line(&GpuInfo {
            name: "GPU 0".to_string(),
            utilization_percent: None,
            memory_total_mb: None,
            memory_used_mb: None,
            temperature_c: None,
        });

        assert_eq!(line, "%Gpu: GPU 0  util   N/A , mem N/A, temp N/A");
    }

    #[test]
    fn nvidia_pmon_process_line_is_parsed() {
        let (pid, info) = parse_nvidia_pmon_process_line(
            "    0   29916     C    37    11     -     -  ollama.exe",
        )
        .expect("pmon process line should parse");

        assert_eq!(pid, 29916);
        assert_eq!(info.gpu_percent, Some(37.0));
        assert_eq!(info.gpu_mem_percent, Some(11.0));
    }

    #[test]
    fn gpu_process_formatters_use_dash_for_missing_values() {
        assert_eq!(format_optional_percent(None), "-");
        assert_eq!(format_optional_percent(Some(42.0)), "42");
        assert_eq!(format_gpu_memory(None, None), "-");
        assert_eq!(format_gpu_memory(None, Some(1536)), "1.5G");
        assert_eq!(format_gpu_memory(Some(8.0), Some(1536)), "8");
    }

    #[test]
    fn vulkan_version_is_decoded_from_raw_api_version() {
        let raw = (1 << 22) | (3 << 12) | 275;
        assert_eq!(
            VulkanVersion::from_raw(raw),
            VulkanVersion {
                major: 1,
                minor: 3,
                patch: 275
            }
        );
        assert_eq!(VulkanVersion::from_raw(raw).to_string(), "1.3.275");
    }

    #[test]
    fn vulkaninfo_summary_devices_are_parsed() {
        let devices = parse_vulkaninfo_summary(
            r#"Vulkan Instance Version: 1.4.341

Devices:
========
GPU0:
    apiVersion         = 1.4.329
    deviceType         = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU
    deviceName         = NVIDIA GeForce RTX 5070
    driverName         = NVIDIA
    driverInfo         = 595.79
GPU1:
    apiVersion         = 1.4.315
    deviceType         = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
    deviceName         = AMD Radeon(TM) Graphics
    driverName         = AMD proprietary driver
"#,
        );

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].index, 0);
        assert_eq!(devices[0].name.as_deref(), Some("NVIDIA GeForce RTX 5070"));
        assert_eq!(devices[0].device_type.as_deref(), Some("discrete gpu"));
        assert_eq!(devices[0].driver_info.as_deref(), Some("595.79"));
        assert_eq!(devices[1].index, 1);
        assert_eq!(devices[1].device_type.as_deref(), Some("integrated gpu"));
    }

    #[test]
    fn vulkan_lines_include_device_details_when_present() {
        let lines = format_vulkan_lines(&VulkanInfo {
            api_version: VulkanVersion {
                major: 1,
                minor: 4,
                patch: 341,
            },
            devices: parse_vulkaninfo_summary(
                r#"GPU0:
    apiVersion = 1.4.329
    deviceType = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU
    deviceName = NVIDIA GeForce RTX 5070
    driverName = NVIDIA
    driverInfo = 595.79
"#,
            ),
        });

        assert_eq!(lines[0], "Vulkan: API 1.4.341, 1 device(s)");
        assert_eq!(
            lines[1],
            "%Vk0: NVIDIA GeForce RTX 5070, discrete gpu, API 1.4.329, driver NVIDIA 595.79"
        );
    }
}
