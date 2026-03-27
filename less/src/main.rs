use std::env;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use encoding_rs::{EUC_JP, ISO_2022_JP, SHIFT_JIS, UTF_8};
use glob;

/// コマンドラインオプション
#[derive(Default, Clone)]
struct Options {
    number: bool,              // -N: 行番号表示
    chop_long: bool,           // -S: 長い行を折り返さない
    squeeze_blank: bool,       // -s: 連続空行を圧縮
    ignore_case: bool,         // -i: 検索で大文字小文字無視
    smart_case: bool,          // -I: スマートケース
    quit_if_one_screen: bool,  // -F: 1画面に収まれば即終了
    quit_at_eof: bool,         // -e: EOF到達で終了
    no_init: bool,             // -X: 終了時に画面をクリアしない
    follow_mode: bool,         // +F: tail -f モード
    prompt_mode: PromptMode,   // -m/-M: プロンプトの詳細度
    start_pattern: Option<String>,
    start_line: Option<usize>,
    tab_stop: usize,
    show_help: bool,
    show_version: bool,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum PromptMode {
    #[default]
    Short,
    Medium,
    Long,
}

struct Pager {
    lines: Vec<String>,
    display_lines: Vec<DisplayLine>,
    top_line: usize,
    left_col: usize,
    term_rows: u16,
    term_cols: u16,
    opts: Options,
    search_pattern: Option<String>,
    filename: String,
    filepath: Option<String>,
    message: Option<String>,
    eof_count: usize,
    filter_pattern: Option<String>,
    input_buffer: String,
}

#[derive(Clone)]
struct DisplayLine {
    original_idx: usize,
    text: String,
    is_continuation: bool,
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    
    let (opts, files) = match parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("less: {}", e);
            std::process::exit(1);
        }
    };

    if opts.show_help {
        print_help();
        return Ok(());
    }

    if opts.show_version {
        println!("less 1.0.0 (Rust for Windows)");
        return Ok(());
    }

    let (lines, filename, filepath) = if files.is_empty() {
        let lines = read_stdin()?;
        (lines, String::from("(standard input)"), None)
    } else {
        let lines = read_file(&files[0])?;
        (lines, files[0].clone(), Some(files[0].clone()))
    };

    let lines = if opts.squeeze_blank {
        squeeze_blank_lines(&lines)
    } else {
        lines
    };

    if opts.quit_if_one_screen {
        let (_, term_rows) = terminal::size().unwrap_or((80, 24));
        if lines.len() <= (term_rows - 1) as usize {
            for line in &lines {
                println!("{}", line);
            }
            return Ok(());
        }
    }

    let mut pager = Pager::new(lines, filename, filepath, opts.clone());
    
    if let Some(n) = opts.start_line {
        pager.goto_line(n);
    }
    
    if let Some(ref pattern) = opts.start_pattern {
        pager.search_pattern = Some(pattern.clone());
        pager.find_next(true);
    }

    if opts.follow_mode {
        pager.run_follow_mode()?;
    } else {
        pager.run()?;
    }

    Ok(())
}

fn parse_args(args: &[String]) -> Result<(Options, Vec<String>), String> {
    let mut opts = Options { tab_stop: 8, ..Default::default() };
    let mut files = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            files.extend(args[i + 1..].iter().cloned());
            break;
        }

        if arg.starts_with('+') {
            let cmd = &arg[1..];
            if cmd.chars().all(|c| c.is_ascii_digit()) {
                opts.start_line = cmd.parse().ok();
            } else if cmd == "F" {
                opts.follow_mode = true;
            } else if cmd.starts_with('/') {
                opts.start_pattern = Some(cmd[1..].to_string());
            }
            i += 1;
            continue;
        }

        if arg.starts_with("--") {
            match arg.as_str() {
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                "--line-numbers" => opts.number = true,
                "--chop-long-lines" => opts.chop_long = true,
                "--squeeze-blank-lines" => opts.squeeze_blank = true,
                "--ignore-case" => opts.ignore_case = true,
                "--IGNORE-CASE" => opts.smart_case = true,
                "--quit-if-one-screen" => opts.quit_if_one_screen = true,
                "--quit-at-eof" => opts.quit_at_eof = true,
                "--no-init" => opts.no_init = true,
                s if s.starts_with("--tabs=") => {
                    opts.tab_stop = s.trim_start_matches("--tabs=").parse().unwrap_or(8);
                }
                s if s.starts_with("--pattern=") => {
                    opts.start_pattern = Some(s.trim_start_matches("--pattern=").to_string());
                }
                _ => return Err(format!("不明なオプション: '{}'", arg)),
            }
            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'N' => opts.number = true,
                    'S' => opts.chop_long = true,
                    's' => opts.squeeze_blank = true,
                    'i' => opts.ignore_case = true,
                    'I' => opts.smart_case = true,
                    'F' => opts.quit_if_one_screen = true,
                    'e' => opts.quit_at_eof = true,
                    'X' => opts.no_init = true,
                    'm' => opts.prompt_mode = PromptMode::Medium,
                    'M' => opts.prompt_mode = PromptMode::Long,
                    'n' => {}
                    'h' => opts.show_help = true,
                    'V' => opts.show_version = true,
                    'p' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.start_pattern = Some(rest);
                        } else if i + 1 < args.len() {
                            i += 1;
                            opts.start_pattern = Some(args[i].clone());
                        }
                        break;
                    }
                    'x' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if !rest.is_empty() {
                            opts.tab_stop = rest.parse().unwrap_or(8);
                        } else if i + 1 < args.len() {
                            i += 1;
                            opts.tab_stop = args[i].parse().unwrap_or(8);
                        }
                        break;
                    }
                    _ => return Err(format!("不明なオプション: '-{}'", chars[j])),
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        files.push(arg.clone());
        i += 1;
    }

    // glob展開
    let files = expand_globs(files);

    Ok((opts, files))
}

/// Windows向けglob展開（大文字小文字を区別しない）
fn expand_globs(raw_files: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    
    // Windowsでは大文字小文字を区別しない
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    
    for pattern in raw_files {
        // "-" は標準入力なのでそのまま
        if pattern == "-" {
            result.push(pattern);
            continue;
        }
        
        // ワイルドカード（* または ?）を含む場合はglob展開
        if pattern.contains('*') || pattern.contains('?') {
            match glob::glob_with(&pattern, options) {
                Ok(paths) => {
                    let mut matched = false;
                    for entry in paths {
                        if let Ok(path) = entry {
                            let path: PathBuf = path;
                            if path.is_file() {
                                result.push(path.to_string_lossy().to_string());
                                matched = true;
                            }
                        }
                    }
                    if !matched {
                        // マッチなしの場合は元のパターンをそのまま（エラー表示用）
                        result.push(pattern);
                    }
                }
                Err(_) => {
                    // glob解析エラーの場合も元のパターンをそのまま
                    result.push(pattern);
                }
            }
        } else {
            result.push(pattern);
        }
    }
    
    result
}

fn print_help() {
    println!(r#"使い方: less [オプション] [ファイル]...

テキストファイルをページ単位で表示します。
UTF-8, Shift_JIS, EUC-JP, ISO-2022-JP を自動判定します。

オプション:
  -N, --line-numbers         行番号を表示
  -S, --chop-long-lines      長い行を折り返さない（横スクロール可能）
  -s, --squeeze-blank-lines  連続する空行を1行に圧縮
  -i, --ignore-case          検索で大文字小文字を区別しない
  -I, --IGNORE-CASE          スマートケース（大文字含むパターンは区別）
  -F, --quit-if-one-screen   1画面に収まれば即終了
  -e, --quit-at-eof          ファイル末尾で終了
  -X, --no-init              終了時に画面をクリアしない
  -m                         プロンプトにパーセントを表示
  -M                         詳細なプロンプトを表示
  -x N, --tabs=N             タブ幅を設定（デフォルト: 8）
  -p pattern                 起動時に検索
  +N                         N行目から表示
  +/pattern                  起動時にpatternを検索
  +F                         tail -f モードで起動
  -h, --help                 このヘルプを表示
  -V, --version              バージョンを表示

キー操作:
  移動:
    j, ↓, Enter, e           1行下へ
    k, ↑, y                  1行上へ
    f, Space, PgDn           1ページ下へ
    b, PgUp                  1ページ上へ
    d                        半ページ下へ
    u                        半ページ上へ
    g, Home                  先頭へ
    G, End                   末尾へ
    Ng                       N行目へ（例: 100g）
    →, l                     右へスクロール（-S時）
    ←, h                     左へスクロール（-S時）

  検索:
    /pattern                 前方検索
    ?pattern                 後方検索
    n                        次の検索結果
    N                        前の検索結果
    &pattern                 パターンにマッチする行のみ表示

  その他:
    =                        ファイル情報を表示
    v                        エディタで開く
    F                        tail -f モード（Ctrl+Cで戻る）
    q, Q                     終了

環境変数:
  VISUAL, EDITOR      v キーで起動するエディタ"#);
}

fn squeeze_blank_lines(lines: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut prev_blank = false;
    for line in lines {
        let is_blank = line.trim().is_empty();
        if !(is_blank && prev_blank) {
            result.push(line.clone());
        }
        prev_blank = is_blank;
    }
    result
}

fn read_stdin() -> io::Result<Vec<String>> {
    let stdin = io::stdin();
    let mut buffer = Vec::new();
    stdin.lock().read_to_end(&mut buffer)?;
    let content = decode_to_utf8(&buffer);
    Ok(content.lines().map(String::from).collect())
}

fn read_file(path: &str) -> io::Result<Vec<String>> {
    let path = Path::new(path);
    if path.is_dir() {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "ディレクトリです"));
    }
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let content = decode_to_utf8(&buffer);
    Ok(content.lines().map(String::from).collect())
}

fn detect_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }
    if is_iso2022jp(bytes) {
        return ISO_2022_JP;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return UTF_8;
    }
    let sjis = calc_sjis_score(bytes);
    let eucjp = calc_eucjp_score(bytes);
    if sjis > eucjp { SHIFT_JIS } else if eucjp > sjis { EUC_JP } else { SHIFT_JIS }
}

fn is_iso2022jp(bytes: &[u8]) -> bool {
    for i in 0..bytes.len().saturating_sub(2) {
        if bytes[i] == 0x1B {
            if (bytes[i+1] == b'$' && (bytes[i+2] == b'B' || bytes[i+2] == b'@'))
                || (bytes[i+1] == b'(' && (bytes[i+2] == b'B' || bytes[i+2] == b'J')) {
                return true;
            }
        }
    }
    false
}

fn calc_sjis_score(bytes: &[u8]) -> i32 {
    let mut score = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            i += 1;
        } else if (0x81..=0x9F).contains(&b) || (0xE0..=0xEF).contains(&b) {
            if i + 1 < bytes.len() {
                let b2 = bytes[i + 1];
                if (0x40..=0x7E).contains(&b2) || (0x80..=0xFC).contains(&b2) {
                    score += 2; i += 2;
                } else { score -= 2; i += 1; }
            } else { i += 1; }
        } else if (0xA1..=0xDF).contains(&b) {
            if i + 1 < bytes.len() && (0xA1..=0xFE).contains(&bytes[i + 1]) {
                score -= 1;
            } else { score += 1; }
            i += 1;
        } else { score -= 1; i += 1; }
    }
    score
}

fn calc_eucjp_score(bytes: &[u8]) -> i32 {
    let mut score = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b <= 0x7F {
            i += 1;
        } else if (0xA1..=0xFE).contains(&b) {
            if i + 1 < bytes.len() && (0xA1..=0xFE).contains(&bytes[i + 1]) {
                score += 2; i += 2;
            } else { score -= 2; i += 1; }
        } else if b == 0x8E {
            if i + 1 < bytes.len() && (0xA1..=0xDF).contains(&bytes[i + 1]) {
                score += 1; i += 2;
            } else { score -= 1; i += 1; }
        } else { score -= 1; i += 1; }
    }
    score
}

fn decode_to_utf8(bytes: &[u8]) -> String {
    let encoding = detect_encoding(bytes);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

impl Pager {
    fn new(lines: Vec<String>, filename: String, filepath: Option<String>, opts: Options) -> Self {
        let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));
        let mut pager = Pager {
            lines, display_lines: Vec::new(), top_line: 0, left_col: 0,
            term_rows, term_cols, opts, search_pattern: None,
            filename, filepath, message: None, eof_count: 0,
            filter_pattern: None, input_buffer: String::new(),
        };
        pager.rebuild_display_lines();
        pager
    }

    fn rebuild_display_lines(&mut self) {
        self.display_lines.clear();
        let lines: Vec<(usize, &String)> = if let Some(ref pat) = self.filter_pattern {
            let p = pat.to_lowercase();
            self.lines.iter().enumerate().filter(|(_, l)| l.to_lowercase().contains(&p)).collect()
        } else {
            self.lines.iter().enumerate().collect()
        };
        let width = self.get_content_width();
        for (idx, line) in lines {
            if self.opts.chop_long {
                self.display_lines.push(DisplayLine { original_idx: idx, text: line.clone(), is_continuation: false });
            } else {
                let expanded = expand_tabs(line, self.opts.tab_stop);
                for (i, part) in wrap_line(&expanded, width).into_iter().enumerate() {
                    self.display_lines.push(DisplayLine { original_idx: idx, text: part, is_continuation: i > 0 });
                }
            }
        }
    }

    fn get_content_width(&self) -> usize {
        let cols = self.term_cols as usize;
        let width = if self.opts.number {
            cols.saturating_sub(format!("{}", self.lines.len()).len() + 1)
        } else { cols };
        // 行末の余裕を1文字確保（折り返し時の表示崩れ防止）
        width.saturating_sub(1)
    }

    fn run(&mut self) -> io::Result<()> {
        let alt = !self.opts.no_init;
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        if alt { execute!(stdout, EnterAlternateScreen, Hide)?; }
        else { execute!(stdout, Hide)?; }

        self.draw(&mut stdout)?;
        loop {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != crossterm::event::KeyEventKind::Press { continue; }
                    match self.handle_key(key, &mut stdout)? {
                        Action::Continue => {}
                        Action::Quit => break,
                        Action::OpenEditor => { self.open_editor(&mut stdout)?; }
                        Action::FollowMode => { self.run_follow_mode()?; }
                    }
                    self.draw(&mut stdout)?;
                }
                Event::Resize(cols, rows) => {
                    // ターミナルサイズ変更イベントを処理
                    self.term_cols = cols;
                    self.term_rows = rows;
                    self.rebuild_display_lines();
                    execute!(stdout, Clear(ClearType::All))?;
                    self.draw(&mut stdout)?;
                }
                _ => {}
            }
        }
        if alt { execute!(stdout, Show, LeaveAlternateScreen)?; }
        else { execute!(stdout, Show)?; }
        terminal::disable_raw_mode()?;
        Ok(())
    }

    fn run_follow_mode(&mut self) -> io::Result<()> {
        let fp = match &self.filepath {
            Some(p) => p.clone(),
            None => { self.message = Some("Cannot follow standard input".to_string()); return Ok(()); }
        };
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        self.message = Some("Waiting for data... (interrupt to abort)".to_string());
        self.goto_end();
        self.draw(&mut stdout)?;

        let mut file = File::open(&fp)?;
        file.seek(SeekFrom::End(0))?;
        loop {
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind != crossterm::event::KeyEventKind::Press { continue; }
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                            KeyCode::Char('q') | KeyCode::Char('Q') => break,
                            _ => {}
                        }
                    }
                    Event::Resize(cols, rows) => {
                        // ターミナルサイズ変更イベントを処理
                        self.term_cols = cols;
                        self.term_rows = rows;
                        self.rebuild_display_lines();
                        execute!(stdout, Clear(ClearType::All))?;
                        self.draw(&mut stdout)?;
                    }
                    _ => {}
                }
            }
            let mut new_data = Vec::new();
            file.read_to_end(&mut new_data)?;
            if !new_data.is_empty() {
                let content = decode_to_utf8(&new_data);
                for line in content.lines() { self.lines.push(line.to_string()); }
                self.rebuild_display_lines();
                self.goto_end();
                self.message = Some("Waiting for data... (interrupt to abort)".to_string());
                self.draw(&mut stdout)?;
            }
        }
        self.message = None;
        execute!(stdout, Show, LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    fn draw(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        execute!(stdout, MoveTo(0, 0))?;
        let content_rows = (self.term_rows - 1) as usize;
        let num_width = if self.opts.number { format!("{}", self.lines.len()).len() } else { 0 };

        for row in 0..content_rows {
            execute!(stdout, MoveTo(0, row as u16), Clear(ClearType::CurrentLine))?;
            let idx = self.top_line + row;
            if idx < self.display_lines.len() {
                let dl = &self.display_lines[idx];
                if self.opts.number {
                    if dl.is_continuation {
                        execute!(stdout, SetForegroundColor(Color::DarkYellow), Print(format!("{:>w$} ", "", w = num_width)), ResetColor)?;
                    } else {
                        execute!(stdout, SetForegroundColor(Color::DarkYellow), Print(format!("{:>w$} ", dl.original_idx + 1, w = num_width)), ResetColor)?;
                    }
                }
                let text = if self.opts.chop_long {
                    let chars: Vec<char> = dl.text.chars().collect();
                    let start = self.left_col.min(chars.len());
                    truncate_to_width(&chars[start..].iter().collect::<String>(), self.term_cols as usize - num_width - 1)
                } else { dl.text.clone() };

                if let Some(ref pat) = self.search_pattern {
                    self.print_highlighted(stdout, &text, pat)?;
                } else {
                    execute!(stdout, Print(&text))?;
                }
            } else {
                execute!(stdout, SetForegroundColor(Color::Blue), Print("~"), ResetColor)?;
            }
        }
        self.draw_status(stdout)?;
        stdout.flush()?;
        Ok(())
    }

    fn print_highlighted(&self, stdout: &mut io::Stdout, line: &str, pattern: &str) -> io::Result<()> {
        let (sline, spat) = if self.opts.ignore_case || self.opts.smart_case {
            let has_upper = pattern.chars().any(|c| c.is_uppercase());
            if self.opts.smart_case && has_upper { (line.to_string(), pattern.to_string()) }
            else { (line.to_lowercase(), pattern.to_lowercase()) }
        } else { (line.to_string(), pattern.to_string()) };

        let mut last = 0;
        let chars: Vec<char> = line.chars().collect();
        for (start, _) in sline.match_indices(&spat) {
            if start > last {
                execute!(stdout, Print(chars[last..start].iter().collect::<String>()))?;
            }
            let end = (start + pattern.len()).min(chars.len());
            execute!(stdout, SetBackgroundColor(Color::Yellow), SetForegroundColor(Color::Black),
                Print(chars[start..end].iter().collect::<String>()), ResetColor)?;
            last = end;
        }
        if last < chars.len() {
            execute!(stdout, Print(chars[last..].iter().collect::<String>()))?;
        }
        Ok(())
    }

    fn draw_status(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        let row = self.term_rows - 1;
        execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        let status = self.message.clone().unwrap_or_else(|| self.build_prompt());
        execute!(stdout, Print("\x1b[7m"), Print(&status), Print("\x1b[0m"))?;
        Ok(())
    }

    fn build_prompt(&self) -> String {
        let rows = (self.term_rows - 1) as usize;
        let at_end = self.top_line + rows >= self.display_lines.len();
        let at_start = self.top_line == 0;
        match self.opts.prompt_mode {
            PromptMode::Short => {
                if self.display_lines.is_empty() || at_end { "(END)".into() }
                else if at_start { format!("{}:", self.filename) }
                else { ":".into() }
            }
            PromptMode::Medium => {
                if at_end { "(END)".into() }
                else { format!("{}%", ((self.top_line + rows) * 100) / self.display_lines.len().max(1)) }
            }
            PromptMode::Long => {
                let start = self.top_line + 1;
                let end = (self.top_line + rows).min(self.display_lines.len());
                let total = self.display_lines.len();
                if at_end { format!("{} lines {}-{}/{} (END)", self.filename, start, end, total) }
                else { format!("{} lines {}-{}/{} {}%", self.filename, start, end, total, (end * 100) / total.max(1)) }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent, stdout: &mut io::Stdout) -> io::Result<Action> {
        self.message = None;
        let rows = (self.term_rows - 1) as usize;
        let page = rows.saturating_sub(1);
        let half = page / 2;
        let max_top = self.display_lines.len().saturating_sub(rows);

        if let KeyCode::Char(c) = key.code {
            if c.is_ascii_digit() {
                self.input_buffer.push(c);
                self.message = Some(format!(":{}", self.input_buffer));
                return Ok(Action::Continue);
            }
        }
        let count: Option<usize> = if !self.input_buffer.is_empty() {
            let n = self.input_buffer.parse().ok();
            self.input_buffer.clear();
            n
        } else { None };

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(Action::Quit),
            KeyCode::Esc => {
                if self.filter_pattern.is_some() {
                    self.filter_pattern = None;
                    self.rebuild_display_lines();
                } else { return Ok(Action::Quit); }
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(Action::Quit),

            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter | KeyCode::Char('e') => {
                self.top_line = (self.top_line + count.unwrap_or(1)).min(max_top);
                self.check_eof();
            }
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('y') => {
                self.top_line = self.top_line.saturating_sub(count.unwrap_or(1));
                self.eof_count = 0;
            }
            KeyCode::Char('f') | KeyCode::Char(' ') | KeyCode::PageDown => {
                self.top_line = (self.top_line + page * count.unwrap_or(1)).min(max_top);
                self.check_eof();
            }
            KeyCode::Char('b') | KeyCode::PageUp => {
                self.top_line = self.top_line.saturating_sub(page * count.unwrap_or(1));
                self.eof_count = 0;
            }
            KeyCode::Char('d') => {
                self.top_line = (self.top_line + count.unwrap_or(half)).min(max_top);
                self.check_eof();
            }
            KeyCode::Char('u') => {
                self.top_line = self.top_line.saturating_sub(count.unwrap_or(half));
                self.eof_count = 0;
            }
            KeyCode::Char('g') | KeyCode::Char('<') | KeyCode::Home => {
                if let Some(n) = count { self.goto_line(n); }
                else { self.top_line = 0; }
                self.eof_count = 0;
            }
            KeyCode::Char('G') | KeyCode::Char('>') | KeyCode::End => {
                if let Some(n) = count { self.goto_line(n); }
                else { self.goto_end(); }
            }
            KeyCode::Right | KeyCode::Char('l') if self.opts.chop_long => {
                self.left_col += count.unwrap_or(8);
            }
            KeyCode::Left | KeyCode::Char('h') if self.opts.chop_long => {
                self.left_col = self.left_col.saturating_sub(count.unwrap_or(8));
            }
            KeyCode::Char('/') => { self.search_forward(stdout)?; }
            KeyCode::Char('?') => { self.search_backward(stdout)?; }
            KeyCode::Char('n') => { self.find_next(true); }
            KeyCode::Char('N') => { self.find_next(false); }
            KeyCode::Char('&') => { self.set_filter(stdout)?; }
            KeyCode::Char('=') => { self.show_file_info(); }
            KeyCode::Char('v') => { return Ok(Action::OpenEditor); }
            KeyCode::Char('F') => { return Ok(Action::FollowMode); }
            _ => {}
        }
        Ok(Action::Continue)
    }

    fn check_eof(&mut self) {
        let rows = (self.term_rows - 1) as usize;
        if self.top_line >= self.display_lines.len().saturating_sub(rows) {
            self.eof_count += 1;
        }
    }

    fn goto_line(&mut self, n: usize) {
        let target = n.saturating_sub(1);
        for (i, dl) in self.display_lines.iter().enumerate() {
            if dl.original_idx >= target && !dl.is_continuation {
                self.top_line = i;
                return;
            }
        }
        self.goto_end();
    }

    fn goto_end(&mut self) {
        let rows = (self.term_rows - 1) as usize;
        self.top_line = self.display_lines.len().saturating_sub(rows);
    }

    fn search_forward(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if let Some(pat) = self.prompt_input(stdout, "/")? {
            if !pat.is_empty() { self.search_pattern = Some(pat); self.find_next(true); }
        }
        Ok(())
    }

    fn search_backward(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if let Some(pat) = self.prompt_input(stdout, "?")? {
            if !pat.is_empty() { self.search_pattern = Some(pat); self.find_next(false); }
        }
        Ok(())
    }

    fn set_filter(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if let Some(pat) = self.prompt_input(stdout, "&")? {
            self.filter_pattern = if pat.is_empty() { None } else { Some(pat) };
            self.top_line = 0;
            self.rebuild_display_lines();
        }
        Ok(())
    }

    fn prompt_input(&mut self, stdout: &mut io::Stdout, prompt: &str) -> io::Result<Option<String>> {
        let row = self.term_rows - 1;
        execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine), Print("\x1b[7m"), Print(prompt), Print("\x1b[0m"), Show)?;
        stdout.flush()?;
        let mut input = String::new();
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind != crossterm::event::KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Enter => { execute!(stdout, Hide)?; return Ok(Some(input)); }
                    KeyCode::Esc => { execute!(stdout, Hide)?; return Ok(None); }
                    KeyCode::Backspace => {
                        input.pop();
                        execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine), Print("\x1b[7m"), Print(prompt), Print("\x1b[0m"), Print(&input))?;
                        stdout.flush()?;
                    }
                    KeyCode::Char(c) => { input.push(c); execute!(stdout, Print(c))?; stdout.flush()?; }
                    _ => {}
                }
            }
        }
    }

    fn find_next(&mut self, forward: bool) {
        let pattern = match &self.search_pattern {
            Some(p) => p.clone(),
            None => { self.message = Some("No previous search pattern".into()); return; }
        };
        let (spat, lower) = if self.opts.smart_case {
            if pattern.chars().any(|c| c.is_uppercase()) { (pattern.clone(), false) }
            else { (pattern.to_lowercase(), true) }
        } else if self.opts.ignore_case { (pattern.to_lowercase(), true) }
        else { (pattern.clone(), false) };

        let rows = (self.term_rows - 1) as usize;
        if forward {
            for i in (self.top_line + 1)..self.display_lines.len() {
                let line = if lower { self.display_lines[i].text.to_lowercase() } else { self.display_lines[i].text.clone() };
                if line.contains(&spat) {
                    self.top_line = i.min(self.display_lines.len().saturating_sub(rows));
                    return;
                }
            }
        } else {
            for i in (0..self.top_line).rev() {
                let line = if lower { self.display_lines[i].text.to_lowercase() } else { self.display_lines[i].text.clone() };
                if line.contains(&spat) { self.top_line = i; return; }
            }
        }
        self.message = Some("Pattern not found".into());
    }

    fn show_file_info(&mut self) {
        let rows = (self.term_rows - 1) as usize;
        let start = self.top_line + 1;
        let end = (self.top_line + rows).min(self.display_lines.len());
        let bytes: usize = self.lines.iter().map(|l| l.len() + 1).sum();
        self.message = Some(format!("{} lines {}-{} / {} lines ({} bytes)", self.filename, start, end, self.lines.len(), bytes));
    }

    fn open_editor(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        let fp = match &self.filepath {
            Some(p) => p.clone(),
            None => { self.message = Some("Cannot edit standard input".into()); return Ok(()); }
        };
        let line_num = if self.top_line < self.display_lines.len() {
            self.display_lines[self.top_line].original_idx + 1
        } else { 1 };

        let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "notepad".into());
        execute!(stdout, Show, LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        launch_editor(&editor, &fp, line_num)?;

        if let Ok(lines) = read_file(&fp) {
            self.lines = if self.opts.squeeze_blank { squeeze_blank_lines(&lines) } else { lines };
            self.rebuild_display_lines();
        }
        terminal::enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(())
    }
}

fn expand_tabs(s: &str, tab: usize) -> String {
    let mut result = String::new();
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let spaces = tab - (col % tab);
            result.extend(std::iter::repeat(' ').take(spaces));
            col += spaces;
        } else {
            result.push(c);
            col += char_width(c);
        }
    }
    result
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 { return vec![line.to_string()]; }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut cw = 0;
    for c in line.chars() {
        let w = char_width(c);
        // 全角文字が行末にはみ出す場合、先に改行
        if cw + w > width {
            result.push(current);
            current = String::new();
            cw = 0;
        }
        current.push(c);
        cw += w;
    }
    if !current.is_empty() || result.is_empty() { result.push(current); }
    result
}

fn truncate_to_width(s: &str, width: usize) -> String {
    let mut result = String::new();
    let mut cw = 0;
    for c in s.chars() {
        let w = char_width(c);
        if cw + w > width { break; }
        result.push(c);
        cw += w;
    }
    result
}

fn char_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(0)
}

fn launch_editor(editor: &str, filepath: &str, line: usize) -> io::Result<()> {
    let e = editor.to_lowercase();
    let mut cmd = if e.contains("code") {
        let mut c = Command::new(editor); c.arg("-g").arg(format!("{}:{}", filepath, line)); c
    } else if e.contains("vim") || e.contains("nvim") || e.contains("vi") || e.contains("emacs") || e.contains("nano") {
        let mut c = Command::new(editor); c.arg(format!("+{}", line)).arg(filepath); c
    } else if e.contains("notepad++") {
        let mut c = Command::new(editor); c.arg(format!("-n{}", line)).arg(filepath); c
    } else if e.contains("subl") || e.contains("sublime") {
        let mut c = Command::new(editor); c.arg(format!("{}:{}", filepath, line)); c
    } else {
        let mut c = Command::new(editor); c.arg(filepath); c
    };
    cmd.status()?;
    Ok(())
}

enum Action { Continue, Quit, OpenEditor, FollowMode }
