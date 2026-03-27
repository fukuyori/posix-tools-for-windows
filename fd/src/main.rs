// fd-rs - A simple, fast and user-friendly alternative to 'find'
// fd互換 Windows実装 v1.0.0

use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use glob;
use regex::Regex;

#[derive(Default)]
struct Options {
    hidden: bool,
    no_ignore: bool,
    no_ignore_vcs: bool,
    no_ignore_parent: bool,
    case_sensitive: bool,
    ignore_case: bool,
    glob: bool,
    fixed_strings: bool,
    full_path: bool,
    and_patterns: Vec<String>,
    absolute_path: bool,
    list_details: bool,
    follow: bool,
    null_separator: bool,
    color: ColorWhen,
    hyperlink: HyperlinkWhen,
    path_separator: Option<String>,
    strip_cwd_prefix: StripCwdWhen,
    format: Option<String>,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    exact_depth: Option<usize>,
    file_types: Vec<FileType>,
    extensions: Vec<String>,
    size_filters: Vec<SizeFilter>,
    changed_within: Option<String>,
    changed_before: Option<String>,
    exclude: Vec<String>,
    prune: bool,
    exec: Vec<Vec<String>>,
    exec_batch: Option<Vec<String>>,
    batch_size: usize,
    threads: Option<usize>,
    max_results: Option<usize>,
    quiet: bool,
    show_errors: bool,
    one_file_system: bool,
    base_directory: Option<PathBuf>,
    search_paths: Vec<PathBuf>,
    ignore_files: Vec<PathBuf>,
    show_help: bool,
    show_version: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ColorWhen {
    #[default]
    Auto,
    Always,
    Never,
}
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum HyperlinkWhen {
    Auto,
    Always,
    #[default]
    Never,
}
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum StripCwdWhen {
    #[default]
    Auto,
    Always,
    Never,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FileType {
    File,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    Executable,
    Empty,
    Socket,
    Pipe,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SizeFilter {
    Max(u64),
    Min(u64),
    Equals(u64),
}
#[derive(Clone, Debug)]
enum TimeFilter {
    Before(SystemTime),
    After(SystemTime),
}

#[derive(Clone, Debug)]
struct SearchMatcher {
    regex: Option<Regex>,
    match_path: bool,
}

impl SearchMatcher {
    fn matches(&self, file_name: &str, relative_path: &str) -> bool {
        match &self.regex {
            Some(regex) => {
                let candidate = if self.match_path {
                    relative_path
                } else {
                    file_name
                };
                regex.is_match(candidate)
            }
            None => true,
        }
    }
}

#[derive(Clone, Debug)]
struct IgnoreRule {
    base_path: PathBuf,
    pattern: String,
    negated: bool,
    directory_only: bool,
    anchored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Placeholder,
    Basename,
    Parent,
    NoExt,
    BasenameNoExt,
    Text(String),
}
#[derive(Clone, Debug)]
enum FormatTemplate {
    Tokens(Vec<Token>),
    Text(String),
}

impl FormatTemplate {
    fn parse(fmt: &str) -> Self {
        let mut tokens = Vec::new();
        let mut buf = String::new();
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                match chars.peek() {
                    Some(&'{') => {
                        chars.next();
                        buf.push('{');
                    }
                    Some(&'}') => {
                        chars.next();
                        if !buf.is_empty() {
                            tokens.push(Token::Text(std::mem::take(&mut buf)));
                        }
                        tokens.push(Token::Placeholder);
                    }
                    Some(&'/') => {
                        chars.next();
                        match chars.peek() {
                            Some(&'}') => {
                                chars.next();
                                if !buf.is_empty() {
                                    tokens.push(Token::Text(std::mem::take(&mut buf)));
                                }
                                tokens.push(Token::Basename);
                            }
                            Some(&'/') => {
                                chars.next();
                                if chars.peek() == Some(&'}') {
                                    chars.next();
                                    if !buf.is_empty() {
                                        tokens.push(Token::Text(std::mem::take(&mut buf)));
                                    }
                                    tokens.push(Token::Parent);
                                } else {
                                    buf.push_str("{//");
                                }
                            }
                            Some(&'.') => {
                                chars.next();
                                if chars.peek() == Some(&'}') {
                                    chars.next();
                                    if !buf.is_empty() {
                                        tokens.push(Token::Text(std::mem::take(&mut buf)));
                                    }
                                    tokens.push(Token::BasenameNoExt);
                                } else {
                                    buf.push_str("{/.");
                                }
                            }
                            _ => buf.push_str("{/"),
                        }
                    }
                    Some(&'.') => {
                        chars.next();
                        if chars.peek() == Some(&'}') {
                            chars.next();
                            if !buf.is_empty() {
                                tokens.push(Token::Text(std::mem::take(&mut buf)));
                            }
                            tokens.push(Token::NoExt);
                        } else {
                            buf.push_str("{.");
                        }
                    }
                    _ => buf.push('{'),
                }
            } else if c == '}' {
                if chars.peek() == Some(&'}') {
                    chars.next();
                }
                buf.push('}');
            } else {
                buf.push(c);
            }
        }
        if tokens.is_empty() {
            FormatTemplate::Text(buf)
        } else {
            if !buf.is_empty() {
                tokens.push(Token::Text(buf));
            }
            FormatTemplate::Tokens(tokens)
        }
    }
    fn generate(&self, path: &Path, path_separator: Option<&str>) -> OsString {
        match self {
            FormatTemplate::Text(t) => OsString::from(t),
            FormatTemplate::Tokens(tokens) => {
                let mut r = OsString::new();
                for token in tokens {
                    match token {
                        Token::Placeholder => {
                            r.push(replace_separator(path.as_os_str(), path_separator))
                        }
                        Token::Basename => r.push(replace_separator(
                            path.file_name().unwrap_or_default(),
                            path_separator,
                        )),
                        Token::Parent => r.push(replace_separator(
                            path.parent().map(|p| p.as_os_str()).unwrap_or_default(),
                            path_separator,
                        )),
                        Token::NoExt => r.push(replace_separator(
                            remove_extension(path).as_os_str(),
                            path_separator,
                        )),
                        Token::BasenameNoExt => r.push(replace_separator(
                            path.file_stem().unwrap_or_default(),
                            path_separator,
                        )),
                        Token::Text(t) => r.push(t),
                    }
                }
                r
            }
        }
    }
}

fn remove_extension(path: &Path) -> PathBuf {
    match path.extension() {
        Some(_) => {
            let s = path.file_stem().unwrap_or_default();
            match path.parent() {
                Some(p) if p != Path::new("") => p.join(s),
                _ => PathBuf::from(s),
            }
        }
        None => path.to_path_buf(),
    }
}

fn replace_separator<'a>(path: &'a OsStr, sep: Option<&str>) -> Cow<'a, OsStr> {
    match sep {
        None => Cow::Borrowed(path),
        Some(sep) => {
            let mut out = OsString::with_capacity(path.len());
            let mut components = Path::new(path).components().peekable();
            while let Some(comp) = components.next() {
                match comp {
                    Component::Prefix(prefix) => {
                        if let Prefix::UNC(server, share) = prefix.kind() {
                            out.push(sep);
                            out.push(sep);
                            out.push(server);
                            out.push(sep);
                            out.push(share);
                        } else {
                            out.push(comp.as_os_str());
                        }
                    }
                    Component::RootDir => out.push(sep),
                    _ => {
                        out.push(comp.as_os_str());
                        if components.peek().is_some() {
                            out.push(sep);
                        }
                    }
                }
            }
            Cow::Owned(out)
        }
    }
}

const KILO: u64 = 1000;
const MEGA: u64 = KILO * 1000;
const GIGA: u64 = MEGA * 1000;
const TERA: u64 = GIGA * 1000;
const KIBI: u64 = 1024;
const MEBI: u64 = KIBI * 1024;
const GIBI: u64 = MEBI * 1024;
const TEBI: u64 = GIBI * 1024;
static SIZE_REGEX: OnceLock<Regex> = OnceLock::new();

impl SizeFilter {
    fn from_string(s: &str) -> Result<Self, String> {
        let pattern = SIZE_REGEX.get_or_init(|| {
            Regex::new(r"(?i)^([+-]?)(\d+)(b|[kmgt]i?b?)$").expect("SIZE_REGEX pattern is valid")
        });
        let captures = pattern
            .captures(s)
            .ok_or_else(|| format!("'{}' は有効なサイズ制約ではありません", s))?;
        let limit_kind = captures.get(1).map_or("", |m| m.as_str());
        let quantity: u64 = captures
            .get(2)
            .and_then(|v| v.as_str().parse().ok())
            .ok_or_else(|| format!("無効な数値: {}", s))?;
        let unit = captures.get(3).map_or("b", |m| m.as_str()).to_lowercase();
        let multiplier = match &unit[..] {
            v if v.starts_with("ki") => KIBI,
            v if v.starts_with('k') => KILO,
            v if v.starts_with("mi") => MEBI,
            v if v.starts_with('m') => MEGA,
            v if v.starts_with("gi") => GIBI,
            v if v.starts_with('g') => GIGA,
            v if v.starts_with("ti") => TEBI,
            v if v.starts_with('t') => TERA,
            "b" => 1,
            _ => return Err(format!("無効な単位: {}", s)),
        };
        match limit_kind {
            "+" => Ok(SizeFilter::Min(quantity * multiplier)),
            "-" => Ok(SizeFilter::Max(quantity * multiplier)),
            "" => Ok(SizeFilter::Equals(quantity * multiplier)),
            _ => Err(format!("無効: {}", s)),
        }
    }
    fn is_within(&self, size: u64) -> bool {
        match *self {
            SizeFilter::Max(l) => size <= l,
            SizeFilter::Min(l) => size >= l,
            SizeFilter::Equals(l) => size == l,
        }
    }
}

impl TimeFilter {
    fn from_str(s: &str) -> Option<SystemTime> {
        if let Some(ts) = s.strip_prefix('@') {
            if let Ok(secs) = ts.parse::<u64>() {
                return Some(UNIX_EPOCH + Duration::from_secs(secs));
            }
        }
        if let Some(dt) = parse_datetime(s) {
            return Some(dt);
        }
        if let Some(dur) = parse_duration(s) {
            return Some(SystemTime::now() - dur);
        }
        None
    }
    fn before(s: &str) -> Option<TimeFilter> {
        Self::from_str(s).map(TimeFilter::Before)
    }
    fn after(s: &str) -> Option<TimeFilter> {
        Self::from_str(s).map(TimeFilter::After)
    }
    fn applies_to(&self, t: &SystemTime) -> bool {
        match self {
            TimeFilter::Before(l) => t < l,
            TimeFilter::After(l) => t > l,
        }
    }
}

fn parse_datetime(s: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = s.split(|c| c == ' ' || c == 'T').collect();
    let dp: Vec<u32> = parts
        .first()?
        .split('-')
        .filter_map(|p| p.parse().ok())
        .collect();
    if dp.len() != 3 {
        return None;
    }
    let (hour, minute, second) = if parts.len() > 1 {
        let tp: Vec<u32> = parts[1].split(':').filter_map(|p| p.parse().ok()).collect();
        (
            tp.first().copied().unwrap_or(0),
            tp.get(1).copied().unwrap_or(0),
            tp.get(2).copied().unwrap_or(0),
        )
    } else {
        (0, 0, 0)
    };
    let days = days_from_date(dp[0], dp[1], dp[2])?;
    Some(
        UNIX_EPOCH
            + Duration::from_secs(
                days as u64 * 86400 + hour as u64 * 3600 + minute as u64 * 60 + second as u64,
            ),
    )
}

fn days_from_date(year: u32, month: u32, day: u32) -> Option<i64> {
    if month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }
    let (y, m, d) = (year as i64, month as i64, day as i64);
    let a = (14 - m) / 12;
    let y2 = y - a;
    let m2 = m + 12 * a - 3;
    Some(d + (153 * m2 + 2) / 5 + 365 * y2 + y2 / 4 - y2 / 100 + y2 / 400 + 1721119 - 2440588)
}

fn parse_duration(s: &str) -> Option<Duration> {
    static DURATION_REGEX: OnceLock<Regex> = OnceLock::new();
    let pattern = DURATION_REGEX.get_or_init(|| Regex::new(r"(?i)^(\d+)\s*(s|sec|second|seconds|m|min|minute|minutes|h|hour|hours|d|day|days|w|week|weeks|month|months|y|year|years)$").expect("DURATION_REGEX pattern is valid"));
    let c = pattern.captures(s)?;
    let num: u64 = c.get(1)?.as_str().parse().ok()?;
    let unit = c.get(2)?.as_str().to_lowercase();
    let secs = match &unit[..] {
        "s" | "sec" | "second" | "seconds" => num,
        "m" | "min" | "minute" | "minutes" => num * 60,
        "h" | "hour" | "hours" => num * 3600,
        "d" | "day" | "days" => num * 86400,
        "w" | "week" | "weeks" => num * 7 * 86400,
        "month" | "months" => num * 30 * 86400,
        "y" | "year" | "years" => num * 365 * 86400,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct FileId {
    volume_serial: u32,
    file_index: u64,
}
struct FileInfo {
    path: PathBuf,
    file_type: fs::FileType,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (opts, pattern) = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("fd: エラー: {}", e);
            std::process::exit(1);
        }
    };
    if opts.show_help {
        print_help();
        std::process::exit(0);
    }
    if opts.show_version {
        println!("fd 1.0.0 (fd互換 Rust Windows版)");
        std::process::exit(0);
    }
    if let Some(ref base_dir) = opts.base_directory {
        if let Err(e) = env::set_current_dir(base_dir) {
            eprintln!("fd: ディレクトリ変更エラー '{}': {}", base_dir.display(), e);
            std::process::exit(1);
        }
    }

    let search_paths = if opts.search_paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        opts.search_paths.clone()
    };
    let matcher = match build_matcher(&pattern, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("fd: 正規表現エラー: {}", e);
            std::process::exit(1);
        }
    };
    let and_matchers: Vec<SearchMatcher> = opts
        .and_patterns
        .iter()
        .map(|p| {
            build_matcher(&Some(p.clone()), &opts).unwrap_or(SearchMatcher {
                regex: None,
                match_path: false,
            })
        })
        .collect();
    let time_after = opts
        .changed_within
        .as_ref()
        .and_then(|s| TimeFilter::after(s));
    let time_before = opts
        .changed_before
        .as_ref()
        .and_then(|s| TimeFilter::before(s));
    let use_color = match opts.color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => is_tty(),
    };
    let use_hyperlink = match opts.hyperlink {
        HyperlinkWhen::Always => true,
        HyperlinkWhen::Never => false,
        HyperlinkWhen::Auto => use_color,
    };

    let mut results: Vec<FileInfo> = Vec::new();
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut found = false;
    let mut has_error = false;

    for sp in &search_paths {
        if !sp.exists() {
            eprintln!(
                "fd: '{}': そのようなファイルやディレクトリはありません",
                sp.display()
            );
            has_error = true;
            continue;
        }
        let root_abs = absolutize_path(sp);
        let ignore_rules = if !opts.no_ignore {
            load_root_ignore_rules(sp, &root_abs, &opts)
        } else {
            Vec::new()
        };
        let root_dev = if opts.one_file_system {
            get_device_id(sp)
        } else {
            None
        };
        search_directory(
            sp,
            &root_abs,
            sp,
            &matcher,
            &and_matchers,
            &opts,
            &ignore_rules,
            0,
            &mut results,
            root_dev,
            &mut found,
            &mut seen,
            &time_after,
            &time_before,
            &mut has_error,
        );
        if let Some(max) = opts.max_results {
            if results.len() >= max {
                results.truncate(max);
                break;
            }
        }
    }

    if opts.quiet {
        std::process::exit(if found { 0 } else { 1 });
    }
    if let Some(ref cmd) = opts.exec_batch {
        if !results.is_empty() {
            if !exec_batch_command(cmd, &results, &opts) {
                has_error = true;
            }
        }
        std::process::exit(if has_error { 1 } else { 0 });
    }
    if opts.list_details {
        exec_ls_details(&results, &opts);
        std::process::exit(if has_error { 1 } else { 0 });
    }

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    for info in &results {
        if !opts.exec.is_empty() {
            for cmd in &opts.exec {
                if !exec_command(cmd, &info.path, &opts) {
                    has_error = true;
                }
            }
        } else {
            print_result(&mut handle, info, &opts, use_color, use_hyperlink);
        }
    }
    std::process::exit(if has_error { 1 } else { 0 });
}

fn parse_args(args: &[String]) -> Result<(Options, Option<String>), String> {
    let mut opts = Options::default();
    let mut pattern: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut end_of_opts = false;
    let mut i = 1;
    let mut unrestricted = 0u8;

    while i < args.len() {
        let arg = &args[i];
        if end_of_opts {
            positional.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            end_of_opts = true;
            i += 1;
            continue;
        }

        if arg.starts_with("--") {
            match arg.as_str() {
                "--hidden" => opts.hidden = true,
                "--no-hidden" => opts.hidden = false,
                "--no-ignore" => opts.no_ignore = true,
                "--ignore" => opts.no_ignore = false,
                "--no-ignore-vcs" => opts.no_ignore_vcs = true,
                "--no-ignore-parent" => opts.no_ignore_parent = true,
                "--unrestricted" => {
                    unrestricted += 1;
                    if unrestricted >= 1 {
                        opts.no_ignore = true;
                    }
                    if unrestricted >= 2 {
                        opts.hidden = true;
                    }
                }
                "--case-sensitive" => opts.case_sensitive = true,
                "--ignore-case" => opts.ignore_case = true,
                "--glob" => opts.glob = true,
                "--regex" => opts.glob = false,
                "--fixed-strings" | "--literal" => opts.fixed_strings = true,
                "--full-path" => opts.full_path = true,
                "--absolute-path" => opts.absolute_path = true,
                "--relative-path" => opts.absolute_path = false,
                "--list-details" => opts.list_details = true,
                "--follow" | "--dereference" => opts.follow = true,
                "--no-follow" => opts.follow = false,
                "--print0" => opts.null_separator = true,
                "--prune" => opts.prune = true,
                "--quiet" | "--has-results" => opts.quiet = true,
                "--show-errors" => opts.show_errors = true,
                "--one-file-system" | "--mount" | "--xdev" => opts.one_file_system = true,
                "--help" => opts.show_help = true,
                "--version" => opts.show_version = true,
                "--color=auto" => opts.color = ColorWhen::Auto,
                "--color=always" => opts.color = ColorWhen::Always,
                "--color=never" => opts.color = ColorWhen::Never,
                "--hyperlink" | "--hyperlink=auto" => opts.hyperlink = HyperlinkWhen::Auto,
                "--hyperlink=always" => opts.hyperlink = HyperlinkWhen::Always,
                "--hyperlink=never" => opts.hyperlink = HyperlinkWhen::Never,
                "--strip-cwd-prefix" | "--strip-cwd-prefix=always" => {
                    opts.strip_cwd_prefix = StripCwdWhen::Always
                }
                "--strip-cwd-prefix=auto" => opts.strip_cwd_prefix = StripCwdWhen::Auto,
                "--strip-cwd-prefix=never" => opts.strip_cwd_prefix = StripCwdWhen::Never,
                s if s.starts_with("--type=") => {
                    parse_file_types(s.trim_start_matches("--type="), &mut opts.file_types)?
                }
                s if s.starts_with("--extension=") => opts
                    .extensions
                    .push(s.trim_start_matches("--extension=").to_lowercase()),
                s if s.starts_with("--exclude=") => opts
                    .exclude
                    .push(s.trim_start_matches("--exclude=").to_string()),
                s if s.starts_with("--size=") => opts
                    .size_filters
                    .push(SizeFilter::from_string(s.trim_start_matches("--size="))?),
                s if s.starts_with("--max-depth=") => {
                    opts.max_depth = Some(
                        s.trim_start_matches("--max-depth=")
                            .parse()
                            .map_err(|_| "無効な深さ")?,
                    )
                }
                s if s.starts_with("--min-depth=") => {
                    opts.min_depth = Some(
                        s.trim_start_matches("--min-depth=")
                            .parse()
                            .map_err(|_| "無効な深さ")?,
                    )
                }
                s if s.starts_with("--exact-depth=") => {
                    opts.exact_depth = Some(
                        s.trim_start_matches("--exact-depth=")
                            .parse()
                            .map_err(|_| "無効な深さ")?,
                    )
                }
                s if s.starts_with("--max-results=") => {
                    opts.max_results = Some(
                        s.trim_start_matches("--max-results=")
                            .parse()
                            .map_err(|_| "無効な数")?,
                    )
                }
                s if s.starts_with("--changed-within=") => {
                    opts.changed_within =
                        Some(s.trim_start_matches("--changed-within=").to_string())
                }
                s if s.starts_with("--changed-before=") => {
                    opts.changed_before =
                        Some(s.trim_start_matches("--changed-before=").to_string())
                }
                s if s.starts_with("--format=") => {
                    opts.format = Some(s.trim_start_matches("--format=").to_string())
                }
                s if s.starts_with("--path-separator=") => {
                    opts.path_separator =
                        Some(s.trim_start_matches("--path-separator=").to_string())
                }
                s if s.starts_with("--batch-size=") => {
                    opts.batch_size = s
                        .trim_start_matches("--batch-size=")
                        .parse()
                        .map_err(|_| "無効")?
                }
                s if s.starts_with("--threads=") => {
                    opts.threads = Some(
                        s.trim_start_matches("--threads=")
                            .parse()
                            .map_err(|_| "無効")?,
                    )
                }
                s if s.starts_with("--base-directory=") => {
                    opts.base_directory =
                        Some(PathBuf::from(s.trim_start_matches("--base-directory=")))
                }
                s if s.starts_with("--search-path=") => opts
                    .search_paths
                    .push(PathBuf::from(s.trim_start_matches("--search-path="))),
                s if s.starts_with("--ignore-file=") => opts
                    .ignore_files
                    .push(PathBuf::from(s.trim_start_matches("--ignore-file="))),
                s if s.starts_with("--and=") => opts
                    .and_patterns
                    .push(s.trim_start_matches("--and=").to_string()),
                "--type" => {
                    parse_file_types(
                        args.get(i + 1).ok_or("--type には引数が必要")?,
                        &mut opts.file_types,
                    )?;
                    i += 1;
                }
                "--extension" => {
                    opts.extensions.push(
                        args.get(i + 1)
                            .ok_or("--extension には引数が必要")?
                            .to_lowercase(),
                    );
                    i += 1;
                }
                "--exclude" => {
                    opts.exclude
                        .push(args.get(i + 1).ok_or("--exclude には引数が必要")?.clone());
                    i += 1;
                }
                "--size" => {
                    opts.size_filters.push(SizeFilter::from_string(
                        args.get(i + 1).ok_or("--size には引数が必要")?,
                    )?);
                    i += 1;
                }
                "--max-depth" | "--maxdepth" => {
                    opts.max_depth = Some(
                        args.get(i + 1)
                            .ok_or("引数が必要")?
                            .parse()
                            .map_err(|_| "無効")?,
                    );
                    i += 1;
                }
                "--min-depth" | "--mindepth" => {
                    opts.min_depth = Some(
                        args.get(i + 1)
                            .ok_or("引数が必要")?
                            .parse()
                            .map_err(|_| "無効")?,
                    );
                    i += 1;
                }
                "--exact-depth" => {
                    opts.exact_depth = Some(
                        args.get(i + 1)
                            .ok_or("引数が必要")?
                            .parse()
                            .map_err(|_| "無効")?,
                    );
                    i += 1;
                }
                "--max-results" => {
                    opts.max_results = Some(
                        args.get(i + 1)
                            .ok_or("引数が必要")?
                            .parse()
                            .map_err(|_| "無効")?,
                    );
                    i += 1;
                }
                "--changed-within" | "--change-newer-than" | "--newer" | "--changed-after" => {
                    opts.changed_within = Some(args.get(i + 1).ok_or("引数が必要")?.clone());
                    i += 1;
                }
                "--changed-before" | "--change-older-than" | "--older" => {
                    opts.changed_before = Some(args.get(i + 1).ok_or("引数が必要")?.clone());
                    i += 1;
                }
                "--format" => {
                    opts.format = Some(args.get(i + 1).ok_or("引数が必要")?.clone());
                    i += 1;
                }
                "--path-separator" => {
                    opts.path_separator = Some(args.get(i + 1).ok_or("引数が必要")?.clone());
                    i += 1;
                }
                "--color" => {
                    opts.color = match args.get(i + 1).map(|s| s.as_str()) {
                        Some("always") => ColorWhen::Always,
                        Some("never") => ColorWhen::Never,
                        _ => ColorWhen::Auto,
                    };
                    i += 1;
                }
                "--batch-size" => {
                    opts.batch_size = args
                        .get(i + 1)
                        .ok_or("引数が必要")?
                        .parse()
                        .map_err(|_| "無効")?;
                    i += 1;
                }
                "--threads" => {
                    opts.threads = Some(
                        args.get(i + 1)
                            .ok_or("引数が必要")?
                            .parse()
                            .map_err(|_| "無効")?,
                    );
                    i += 1;
                }
                "--base-directory" => {
                    opts.base_directory = Some(PathBuf::from(args.get(i + 1).ok_or("引数が必要")?));
                    i += 1;
                }
                "--search-path" => {
                    opts.search_paths
                        .push(PathBuf::from(args.get(i + 1).ok_or("引数が必要")?));
                    i += 1;
                }
                "--ignore-file" => {
                    opts.ignore_files
                        .push(PathBuf::from(args.get(i + 1).ok_or("引数が必要")?));
                    i += 1;
                }
                "--and" => {
                    opts.and_patterns
                        .push(args.get(i + 1).ok_or("引数が必要")?.clone());
                    i += 1;
                }
                "--exec" => {
                    let c = collect_exec_args(&args[i + 1..]);
                    opts.exec.push(c.0.clone());
                    i += c.1;
                }
                "--exec-batch" => {
                    let c = collect_exec_args(&args[i + 1..]);
                    opts.exec_batch = Some(c.0);
                    i += c.1;
                }
                _ => return Err(format!("認識できないオプション: {}", arg)),
            }
            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'H' => opts.hidden = true,
                    'I' => opts.no_ignore = true,
                    'u' => {
                        unrestricted += 1;
                        if unrestricted >= 1 {
                            opts.no_ignore = true;
                        }
                        if unrestricted >= 2 {
                            opts.hidden = true;
                        }
                    }
                    's' => opts.case_sensitive = true,
                    'i' => opts.ignore_case = true,
                    'g' => opts.glob = true,
                    'F' => opts.fixed_strings = true,
                    'p' => opts.full_path = true,
                    'a' => opts.absolute_path = true,
                    'l' => opts.list_details = true,
                    'L' => opts.follow = true,
                    '0' => opts.null_separator = true,
                    'q' => opts.quiet = true,
                    '1' => opts.max_results = Some(1),
                    'h' => opts.show_help = true,
                    'V' => opts.show_version = true,
                    't' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            parse_file_types(&r, &mut opts.file_types)?;
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            parse_file_types(v, &mut opts.file_types)?;
                            i += 1;
                            break;
                        }
                    }
                    'e' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            opts.extensions.push(r.to_lowercase());
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            opts.extensions.push(v.to_lowercase());
                            i += 1;
                            break;
                        }
                    }
                    'E' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            opts.exclude.push(r);
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            opts.exclude.push(v.clone());
                            i += 1;
                            break;
                        }
                    }
                    'S' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            opts.size_filters.push(SizeFilter::from_string(&r)?);
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            opts.size_filters.push(SizeFilter::from_string(v)?);
                            i += 1;
                            break;
                        }
                    }
                    'd' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            opts.max_depth = Some(r.parse().map_err(|_| "無効")?);
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            opts.max_depth = Some(v.parse().map_err(|_| "無効")?);
                            i += 1;
                            break;
                        }
                    }
                    'j' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            opts.threads = Some(r.parse().map_err(|_| "無効")?);
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            opts.threads = Some(v.parse().map_err(|_| "無効")?);
                            i += 1;
                            break;
                        }
                    }
                    'c' => {
                        if let Some(v) = args.get(i + 1) {
                            opts.color = match v.as_str() {
                                "always" => ColorWhen::Always,
                                "never" => ColorWhen::Never,
                                _ => ColorWhen::Auto,
                            };
                            i += 1;
                            break;
                        }
                    }
                    'C' => {
                        let r: String = chars[j + 1..].iter().collect();
                        if !r.is_empty() {
                            opts.base_directory = Some(PathBuf::from(r));
                            break;
                        } else if let Some(v) = args.get(i + 1) {
                            opts.base_directory = Some(PathBuf::from(v));
                            i += 1;
                            break;
                        }
                    }
                    'x' => {
                        let c = collect_exec_args(&args[i + 1..]);
                        opts.exec.push(c.0.clone());
                        i += c.1;
                        break;
                    }
                    'X' => {
                        let c = collect_exec_args(&args[i + 1..]);
                        opts.exec_batch = Some(c.0);
                        i += c.1;
                        break;
                    }
                    _ => return Err(format!("無効なオプション: -{}", chars[j])),
                }
                j += 1;
            }
            i += 1;
            continue;
        }
        positional.push(arg.clone());
        i += 1;
    }

    if !positional.is_empty() {
        pattern = Some(positional[0].clone());
        for p in positional.into_iter().skip(1) {
            opts.search_paths.push(PathBuf::from(p));
        }
    }
    // search_paths の glob展開
    opts.search_paths = expand_globs_paths(opts.search_paths);
    if let Some(d) = opts.exact_depth {
        opts.max_depth = Some(d);
        opts.min_depth = Some(d);
    }
    Ok((opts, pattern))
}

/// Windows向けglob展開（パス用、大文字小文字を区別しない）
fn expand_globs_paths(raw_paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();

    // Windowsでは大文字小文字を区別しない
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };

    for path in raw_paths {
        let pattern = path.to_string_lossy();
        // ワイルドカード（* または ?）を含む場合はglob展開
        if pattern.contains('*') || pattern.contains('?') {
            match glob::glob_with(&pattern, options) {
                Ok(paths) => {
                    let mut matched = false;
                    for entry in paths {
                        if let Ok(p) = entry {
                            result.push(p);
                            matched = true;
                        }
                    }
                    if !matched {
                        // マッチなしの場合は元のパターンをそのまま（エラー表示用）
                        result.push(path);
                    }
                }
                Err(_) => {
                    // glob解析エラーの場合も元のパターンをそのまま
                    result.push(path);
                }
            }
        } else {
            result.push(path);
        }
    }

    result
}

fn collect_exec_args(args: &[String]) -> (Vec<String>, usize) {
    let mut cmd = Vec::new();
    let mut count = 0;
    for arg in args {
        // `;` で明示終了
        if arg == ";" {
            count += 1;
            break;
        }
        // 次のオプション (`-` で始まる) が来たら exec 引数の終端とみなす
        // (fd の実際の動作に合わせる。ただし `--` 以降はパス扱いのため除外)
        if !cmd.is_empty() && arg.starts_with('-') {
            // このオプション自体は消費しない（count を増やさない）
            break;
        }
        count += 1;
        cmd.push(arg.clone());
    }
    (cmd, count)
}

fn parse_file_types(s: &str, types: &mut Vec<FileType>) -> Result<(), String> {
    for part in s.split(',') {
        let ft = match part.trim() {
            "f" | "file" => FileType::File,
            "d" | "dir" | "directory" => FileType::Directory,
            "l" | "symlink" => FileType::Symlink,
            "x" | "executable" => FileType::Executable,
            "e" | "empty" => FileType::Empty,
            "s" | "socket" => FileType::Socket,
            "p" | "pipe" => FileType::Pipe,
            "b" | "block-device" => FileType::BlockDevice,
            "c" | "char-device" => FileType::CharDevice,
            _ => return Err(format!("無効なファイルタイプ: {}", part)),
        };
        if !types.contains(&ft) {
            types.push(ft);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn search_directory(
    dir: &Path,
    root_abs: &Path,
    root: &Path,
    matcher: &SearchMatcher,
    and_matchers: &[SearchMatcher],
    opts: &Options,
    inherited_ignore_rules: &[IgnoreRule],
    depth: usize,
    results: &mut Vec<FileInfo>,
    root_device: Option<u64>,
    found: &mut bool,
    seen: &mut HashSet<FileId>,
    time_after: &Option<TimeFilter>,
    time_before: &Option<TimeFilter>,
    has_error: &mut bool,
) {
    let dir_abs = if dir == root {
        root_abs.to_path_buf()
    } else {
        root_abs.join(dir.strip_prefix(root).unwrap_or(dir))
    };
    let ignore_rules = extend_ignore_rules(inherited_ignore_rules, dir, &dir_abs, opts);
    let effective_max = opts.max_depth.or(opts.exact_depth);
    let effective_min = opts.min_depth.or(opts.exact_depth).unwrap_or(0);
    if let Some(max) = opts.max_results {
        if results.len() >= max {
            return;
        }
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            if opts.show_errors {
                eprintln!("fd: '{}' を読み込めません: {}", dir.display(), e);
            }
            *has_error = true;
            return;
        }
    };
    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                if opts.show_errors {
                    eprintln!(
                        "fd: '{}' 内のエントリを読み取れません: {}",
                        dir.display(),
                        e
                    );
                }
                *has_error = true;
                continue;
            }
        };
        let path = entry.path();
        let file_name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        let metadata = if opts.follow {
            fs::metadata(&path)
        } else {
            fs::symlink_metadata(&path)
        };
        let metadata = match metadata {
            Ok(m) => m,
            Err(e) => {
                if opts.show_errors {
                    eprintln!("fd: '{}': {}", path.display(), e);
                }
                continue;
            }
        };
        let file_type = metadata.file_type();
        let is_dir = file_type.is_dir();
        let is_symlink = file_type.is_symlink();
        let entry_depth = depth + 1;
        let relative_path = path.strip_prefix(root).unwrap_or(&path);
        let normalized_relative_path = normalize_for_matching(relative_path);

        if let Some(max) = effective_max {
            if entry_depth > max {
                continue;
            }
        }

        if !opts.hidden && is_hidden(&path, &file_name) {
            continue;
        }
        if !opts.no_ignore && is_ignored(&path, &file_name, is_dir, &ignore_rules) {
            continue;
        }
        if is_excluded(&file_name, &normalized_relative_path, is_dir, &opts.exclude) {
            continue;
        }

        if opts.one_file_system {
            if let Some(rd) = root_device {
                if let Some(cd) = get_device_id(&path) {
                    if rd != cd {
                        continue;
                    }
                }
            }
        }
        if !is_dir && !is_symlink {
            if let Some(fid) = get_file_id(&path) {
                if !seen.insert(fid) {
                    continue;
                }
            }
        }

        let is_executable = is_executable_file(&path, &file_name);
        let is_empty = if is_dir {
            fs::read_dir(&path)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        } else {
            metadata.len() == 0
        };
        let type_match = if opts.file_types.is_empty() {
            true
        } else {
            opts.file_types
                .iter()
                .any(|ft| matches_file_type(*ft, is_dir, is_symlink, is_executable, is_empty))
        };
        if !type_match {
            if is_dir {
                search_directory(
                    &path,
                    root_abs,
                    root,
                    matcher,
                    and_matchers,
                    opts,
                    &ignore_rules,
                    entry_depth,
                    results,
                    root_device,
                    found,
                    seen,
                    time_after,
                    time_before,
                    has_error,
                );
            }
            continue;
        }

        if !opts.extensions.is_empty() {
            if is_dir {
                search_directory(
                    &path,
                    root_abs,
                    root,
                    matcher,
                    and_matchers,
                    opts,
                    &ignore_rules,
                    entry_depth,
                    results,
                    root_device,
                    found,
                    seen,
                    time_after,
                    time_before,
                    has_error,
                );
                continue;
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !opts.extensions.iter().any(|e| e == &ext) {
                continue;
            }
        }
        if !opts.size_filters.is_empty() && !is_dir {
            let size = metadata.len();
            if !opts.size_filters.iter().all(|sf| sf.is_within(size)) {
                continue;
            }
        }
        if let Some(ref tf) = time_after {
            if let Ok(mt) = metadata.modified() {
                if !tf.applies_to(&mt) {
                    continue;
                }
            }
        }
        if let Some(ref tf) = time_before {
            if let Ok(mt) = metadata.modified() {
                if !tf.applies_to(&mt) {
                    continue;
                }
            }
        }

        let matches = matcher.matches(&file_name, &normalized_relative_path);
        let and_matches = and_matchers
            .iter()
            .all(|m| m.matches(&file_name, &normalized_relative_path));

        if matches && and_matches {
            *found = true;
            if entry_depth >= effective_min {
                if let Some(max) = opts.max_results {
                    if results.len() >= max {
                        return;
                    }
                }
                let display_path = if opts.absolute_path {
                    fs::canonicalize(&path).unwrap_or_else(|_| path.clone())
                } else if should_strip_cwd(&opts.strip_cwd_prefix, opts) {
                    path.strip_prefix("./").unwrap_or(&path).to_path_buf()
                } else {
                    path.clone()
                };
                results.push(FileInfo {
                    path: normalize_path(&display_path),
                    file_type,
                });
            }
            if opts.prune && is_dir {
                continue;
            }
        }
        if is_dir {
            search_directory(
                &path,
                root_abs,
                root,
                matcher,
                and_matchers,
                opts,
                &ignore_rules,
                entry_depth,
                results,
                root_device,
                found,
                seen,
                time_after,
                time_before,
                has_error,
            );
        }
    }
}

fn should_strip_cwd(mode: &StripCwdWhen, opts: &Options) -> bool {
    match mode {
        StripCwdWhen::Always => true,
        StripCwdWhen::Never => false,
        StripCwdWhen::Auto => {
            !opts.exec.is_empty() || opts.exec_batch.is_some() || opts.null_separator
        }
    }
}

fn matches_file_type(
    ft: FileType,
    is_dir: bool,
    is_symlink: bool,
    is_executable: bool,
    is_empty: bool,
) -> bool {
    match ft {
        FileType::File => !is_dir && !is_symlink,
        FileType::Directory => is_dir,
        FileType::Symlink => is_symlink,
        FileType::Executable => is_executable && !is_dir,
        FileType::Empty => is_empty,
        _ => false,
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

fn build_matcher(pattern: &Option<String>, opts: &Options) -> Result<SearchMatcher, String> {
    let pattern = match pattern {
        Some(p) if !p.is_empty() => p,
        _ => {
            return Ok(SearchMatcher {
                regex: None,
                match_path: false,
            })
        }
    };
    let match_path = opts.full_path || (opts.glob && has_path_separator(pattern));
    let normalized_pattern = if match_path {
        normalize_pattern(pattern)
    } else {
        pattern.clone()
    };
    let pat = if opts.fixed_strings {
        regex::escape(&normalized_pattern)
    } else if opts.glob {
        format!("^{}$", glob_to_regex(&normalized_pattern))
    } else {
        normalized_pattern.clone()
    };
    let ci = if opts.case_sensitive {
        false
    } else if opts.ignore_case {
        true
    } else {
        !pattern.chars().any(|c| c.is_uppercase())
    };
    let regex = regex::RegexBuilder::new(&pat)
        .case_insensitive(ci)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(SearchMatcher {
        regex: Some(regex),
        match_path,
    })
}

fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::new();
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    regex.push_str(".*");
                } else {
                    regex.push_str("[^/\\\\]*");
                }
            }
            '?' => regex.push_str("[^/\\\\]"),
            '[' => {
                regex.push('[');
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == ']' {
                        regex.push(']');
                        break;
                    }
                    if c == '!' && regex.ends_with('[') {
                        regex.push('^');
                    } else {
                        regex.push(c);
                    }
                }
            }
            '{' => {
                regex.push_str("(?:");
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '}' {
                        regex.push(')');
                        break;
                    }
                    if c == ',' {
                        regex.push('|');
                    } else {
                        if ".+()^$|\\".contains(c) {
                            regex.push('\\');
                        }
                        regex.push(c);
                    }
                }
            }
            '.' | '+' | '(' | ')' | '^' | '$' | '|' | '\\' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }
    regex
}

fn has_path_separator(pattern: &str) -> bool {
    pattern.contains('/') || pattern.contains('\\')
}

fn normalize_pattern(pattern: &str) -> String {
    pattern.replace('\\', "/")
}

fn normalize_for_matching(path: &Path) -> String {
    normalize_pattern(&path.to_string_lossy())
}

fn matches_path_pattern(name: &str, relative_path: &str, is_dir: bool, pattern: &str) -> bool {
    let directory_only = pattern.ends_with('/') || pattern.ends_with('\\');
    if directory_only && !is_dir {
        return false;
    }

    let normalized_pattern = normalize_pattern(pattern);
    let trimmed = normalized_pattern
        .trim_start_matches('/')
        .trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }

    let case_insensitive = cfg!(windows);
    let target = if has_path_separator(trimmed) {
        relative_path
    } else {
        name
    };

    if !(trimmed.contains('*')
        || trimmed.contains('?')
        || trimmed.contains('[')
        || trimmed.contains('{'))
    {
        if has_path_separator(trimmed) {
            return path_equals(target, trimmed, case_insensitive)
                || (directory_only && path_starts_with(relative_path, trimmed, case_insensitive));
        }
        return text_equals(target, trimmed, case_insensitive);
    }

    let re_pat = glob_to_regex(trimmed);
    regex::RegexBuilder::new(&format!("^{}$", re_pat))
        .case_insensitive(case_insensitive)
        .build()
        .map(|re| re.is_match(target))
        .unwrap_or(false)
}

fn ignore_rule_matches(rule: &IgnoreRule, path: &Path, name: &str, is_dir: bool) -> bool {
    if rule.directory_only && !is_dir {
        return false;
    }

    let Ok(path_under_base) = path.strip_prefix(&rule.base_path) else {
        return false;
    };
    let relative_path = normalize_for_matching(path_under_base);
    let candidate = if rule.anchored || has_path_separator(&rule.pattern) {
        &relative_path
    } else {
        name
    };

    if !(rule.pattern.contains('*')
        || rule.pattern.contains('?')
        || rule.pattern.contains('[')
        || rule.pattern.contains('{'))
    {
        if rule.anchored || has_path_separator(&rule.pattern) {
            return path_equals(candidate, &rule.pattern, cfg!(windows))
                || (rule.directory_only
                    && path_starts_with(&relative_path, &rule.pattern, cfg!(windows)));
        }
        return text_equals(candidate, &rule.pattern, cfg!(windows));
    }

    let re_pat = glob_to_regex(&rule.pattern);
    regex::RegexBuilder::new(&format!("^{}$", re_pat))
        .case_insensitive(cfg!(windows))
        .build()
        .map(|re| re.is_match(candidate))
        .unwrap_or(false)
}

fn text_equals(left: &str, right: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn path_equals(left: &str, right: &str, case_insensitive: bool) -> bool {
    text_equals(
        left.trim_matches('/'),
        right.trim_matches('/'),
        case_insensitive,
    )
}

fn path_starts_with(path: &str, prefix: &str, case_insensitive: bool) -> bool {
    let normalized_path = path.trim_matches('/');
    let normalized_prefix = prefix.trim_matches('/');
    if normalized_path.len() < normalized_prefix.len() {
        return false;
    }

    let Some(head) = normalized_path.get(..normalized_prefix.len()) else {
        return false;
    };
    if !text_equals(head, normalized_prefix, case_insensitive) {
        return false;
    }

    normalized_path.len() == normalized_prefix.len()
        || normalized_path.as_bytes().get(normalized_prefix.len()) == Some(&b'/')
}

fn absolutize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn default_ignore_rules(root_abs: &Path) -> Vec<IgnoreRule> {
    [
        ".git",
        ".hg",
        ".svn",
        ".bzr",
        "CVS",
        "node_modules",
        "target",
        "__pycache__",
        ".venv",
        "venv",
    ]
    .into_iter()
    .map(|pattern| IgnoreRule {
        base_path: root_abs.to_path_buf(),
        pattern: pattern.to_string(),
        negated: false,
        directory_only: false,
        anchored: false,
    })
    .collect()
}

fn load_root_ignore_rules(root: &Path, root_abs: &Path, opts: &Options) -> Vec<IgnoreRule> {
    let mut rules = default_ignore_rules(root_abs);

    if !opts.no_ignore_parent {
        let start = if root.is_dir() {
            root_abs.to_path_buf()
        } else {
            root_abs
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root_abs.to_path_buf())
        };
        rules.extend(load_parent_ignore_rules(&start, opts));
    }

    rules.extend(load_ignore_file_arguments(root_abs, opts));
    rules
}

fn load_parent_ignore_rules(start_dir: &Path, opts: &Options) -> Vec<IgnoreRule> {
    let mut dirs = Vec::new();
    let mut current = start_dir.parent();
    while let Some(dir) = current {
        dirs.push(dir.to_path_buf());
        current = dir.parent();
    }
    dirs.reverse();

    let mut rules = Vec::new();
    for dir in dirs {
        rules.extend(load_ignore_files_for_dir(&dir, &dir, opts));
    }
    rules
}

fn load_ignore_file_arguments(root_abs: &Path, opts: &Options) -> Vec<IgnoreRule> {
    let mut rules = Vec::new();
    for file in &opts.ignore_files {
        let path = absolutize_path(file);
        if let Ok(content) = fs::read_to_string(&path) {
            rules.extend(parse_ignore_file(&content, root_abs));
        }
    }
    rules
}

fn extend_ignore_rules(
    inherited_rules: &[IgnoreRule],
    dir: &Path,
    dir_abs: &Path,
    opts: &Options,
) -> Vec<IgnoreRule> {
    let mut rules = inherited_rules.to_vec();
    rules.extend(load_ignore_files_for_dir(dir, dir_abs, opts));
    rules
}

fn load_ignore_files_for_dir(dir: &Path, dir_abs: &Path, opts: &Options) -> Vec<IgnoreRule> {
    let mut rules = Vec::new();
    for name in [".gitignore", ".fdignore", ".ignore"] {
        if opts.no_ignore_vcs && name == ".gitignore" {
            continue;
        }
        let path = dir.join(name);
        if let Ok(content) = fs::read_to_string(&path) {
            rules.extend(parse_ignore_file(&content, dir_abs));
        }
    }
    rules
}

fn parse_ignore_file(content: &str, base_path: &Path) -> Vec<IgnoreRule> {
    let mut rules = Vec::new();
    for raw_line in content.lines() {
        if let Some(rule) = parse_ignore_rule(raw_line, base_path) {
            rules.push(rule);
        }
    }
    rules
}

fn parse_ignore_rule(raw_line: &str, base_path: &Path) -> Option<IgnoreRule> {
    let line = raw_line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut pattern = line.to_string();
    if pattern.starts_with(r"\#") || pattern.starts_with(r"\!") {
        pattern.remove(0);
    }

    let negated = pattern.starts_with('!');
    if negated {
        pattern.remove(0);
    }
    if pattern.is_empty() {
        return None;
    }

    let directory_only = pattern.ends_with('/') || pattern.ends_with('\\');
    let pattern = normalize_pattern(pattern.trim_end_matches(['/', '\\']));
    if pattern.is_empty() {
        return None;
    }

    let anchored = pattern.starts_with('/');
    let pattern = pattern.trim_start_matches('/').to_string();
    if pattern.is_empty() {
        return None;
    }

    Some(IgnoreRule {
        base_path: base_path.to_path_buf(),
        pattern,
        negated,
        directory_only,
        anchored,
    })
}

fn is_hidden(path: &Path, name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(m) = path.symlink_metadata() {
            return m.file_attributes() & 0x2 != 0;
        }
    }
    false
}

fn is_ignored(path: &Path, name: &str, is_dir: bool, rules: &[IgnoreRule]) -> bool {
    let mut ignored = false;
    for rule in rules {
        if ignore_rule_matches(rule, path, name, is_dir) {
            ignored = !rule.negated;
        }
    }
    ignored
}

fn is_excluded(name: &str, relative_path: &str, is_dir: bool, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| matches_path_pattern(name, relative_path, is_dir, pattern))
}

fn is_executable_file(_path: &Path, name: &str) -> bool {
    let lower = name.to_lowercase();
    #[cfg(windows)]
    {
        lower.ends_with(".exe")
            || lower.ends_with(".bat")
            || lower.ends_with(".cmd")
            || lower.ends_with(".com")
            || lower.ends_with(".ps1")
            || lower.ends_with(".msi")
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = _path.metadata() {
            m.permissions().mode() & 0o111 != 0
        } else {
            false
        }
    }
}

#[cfg(windows)]
fn get_device_id(path: &Path) -> Option<u64> {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = File::open(path).ok()?;
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(
            windows::Win32::Foundation::HANDLE(file.as_raw_handle()),
            &mut info,
        )
        .is_ok()
        {
            Some(info.dwVolumeSerialNumber as u64)
        } else {
            None
        }
    }
}
#[cfg(not(windows))]
fn get_device_id(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(windows)]
fn get_file_id(path: &Path) -> Option<FileId> {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = File::open(path).ok()?;
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(
            windows::Win32::Foundation::HANDLE(file.as_raw_handle()),
            &mut info,
        )
        .is_ok()
        {
            Some(FileId {
                volume_serial: info.dwVolumeSerialNumber,
                file_index: ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64),
            })
        } else {
            None
        }
    }
}
#[cfg(not(windows))]
fn get_file_id(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).ok().map(|m| FileId {
        volume_serial: m.dev() as u32,
        file_index: m.ino(),
    })
}

fn print_result<W: Write>(
    w: &mut W,
    info: &FileInfo,
    opts: &Options,
    use_color: bool,
    use_hyperlink: bool,
) {
    let mut path_str = info.path.to_string_lossy().to_string();
    if let Some(ref sep) = opts.path_separator {
        path_str = path_str.replace(['/', '\\'], sep.as_str());
    }
    if let Some(ref fmt) = opts.format {
        let template = FormatTemplate::parse(fmt);
        let output = template.generate(&info.path, opts.path_separator.as_deref());
        if opts.null_separator {
            let _ = write!(w, "{}\0", output.to_string_lossy());
        } else {
            let _ = writeln!(w, "{}", output.to_string_lossy());
        }
        return;
    }
    let output = if use_hyperlink {
        format_hyperlink(&path_str)
    } else if use_color {
        colorize_path(&path_str, info)
    } else {
        path_str
    };
    if opts.null_separator {
        let _ = write!(w, "{}\0", output);
    } else {
        let _ = writeln!(w, "{}", output);
    }
}

fn colorize_path(path: &str, info: &FileInfo) -> String {
    if info.file_type.is_dir() {
        format!("\x1b[1;34m{}\x1b[0m", path)
    } else if info.file_type.is_symlink() {
        format!("\x1b[1;36m{}\x1b[0m", path)
    } else if is_executable_file(Path::new(path), path) {
        format!("\x1b[1;32m{}\x1b[0m", path)
    } else {
        path.to_string()
    }
}

fn format_hyperlink(path: &str) -> String {
    let abs = fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());
    let url = format!("file://{}", abs.replace('\\', "/"));
    format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, path)
}

fn is_tty() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::System::Console::{GetConsoleMode, CONSOLE_MODE};
        let h = io::stdout().as_raw_handle();
        let mut m = CONSOLE_MODE::default();
        unsafe { GetConsoleMode(windows::Win32::Foundation::HANDLE(h), &mut m).is_ok() }
    }
    #[cfg(not(windows))]
    {
        unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
    }
}

/// Windows コマンドライン用に引数を安全にクォートする。
/// cmd.exe の解釈ルール：
///   スペース・タブ・`"`・`&`・`|`・`;`・`^` を含む場合は `"..."` で囲む。
///   内部の `"` は `\"` にエスケープする。
fn quote_arg_for_cmd(s: &str) -> String {
    // クォートが必要な文字
    let needs_quote = s.is_empty()
        || s.chars()
            .any(|c| matches!(c, ' ' | '\t' | '"' | '&' | '|' | ';' | '^' | '<' | '>'));
    if !needs_quote {
        return s.to_string();
    }
    // `"` を `\"` にエスケープして全体を `"..."` で囲む
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

/// sh/bash 用に引数をシングルクォートでクォートする。
#[allow(dead_code)]
fn quote_arg_for_sh(s: &str) -> String {
    // シングルクォート内の `'` は `'\''` に置換
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// exec_command/exec_batch_command が使う Windows コマンドライン長の上限（安全マージン込み）
const WINDOWS_CMDLINE_LIMIT: usize = 30_000;

/// --exec / -x : マッチした各ファイルに対して 1 回ずつコマンドを実行する。
///
/// 修正点:
///   - プレースホルダなし時は path をコマンドの argv として直接渡す（shell 経由不要）
///   - プレースホルダあり時は argv を個別に渡す（join(" ") せず Command::arg() を使用）
///   - exit code を返して呼び出し元で has_error に反映できるようにする
fn exec_command(cmd_parts: &[String], path: &Path, opts: &Options) -> bool {
    if cmd_parts.is_empty() {
        return true;
    }

    let has_ph = cmd_parts.iter().any(|p| {
        p.contains("{}")
            || p.contains("{/}")
            || p.contains("{//}")
            || p.contains("{.}")
            || p.contains("{/.}")
    });

    if has_ph {
        // プレースホルダを展開して argv の配列を作る
        let expanded_argv: Vec<std::ffi::OsString> = cmd_parts
            .iter()
            .map(|p| FormatTemplate::parse(p).generate(path, opts.path_separator.as_deref()))
            .collect();

        if expanded_argv.is_empty() {
            return true;
        }
        let status = std::process::Command::new(&expanded_argv[0])
            .args(&expanded_argv[1..])
            .status();
        return status.map(|s| s.success()).unwrap_or(false);
    }

    // プレースホルダなし: path を末尾引数として直接渡す
    let status = std::process::Command::new(&cmd_parts[0])
        .args(&cmd_parts[1..])
        .arg(path)
        .status();
    status.map(|s| s.success()).unwrap_or(false)
}

/// --exec-batch / -X : 全マッチファイルに対して 1 回（または複数バッチに分割して）コマンドを実行する。
///
/// 修正点:
///   - プレースホルダあり時: {} の位置に全パスを展開して 1 回実行（以前は各ファイルでコマンドを
///     繰り返す誤ったロジックだった）
///   - プレースホルダなし時: パスを末尾に追加
///   - Windows コマンドライン長（32,767文字）を超えたらバッチに分割して複数回実行
///   - path に特殊文字が含まれても正しくクォートする
fn exec_batch_command(cmd_parts: &[String], results: &[FileInfo], opts: &Options) -> bool {
    if results.is_empty() || cmd_parts.is_empty() {
        return true;
    }

    let has_ph = cmd_parts.iter().any(|p| {
        p.contains("{}")
            || p.contains("{/}")
            || p.contains("{//}")
            || p.contains("{.}")
            || p.contains("{/.}")
    });

    if has_ph {
        // プレースホルダあり:
        //   cmd_parts の各トークンを走査し、{} 等を含むトークンを「全パスのリスト」に展開。
        //   Windows コマンドライン長を超える場合はバッチ分割する。

        // まずプレースホルダ位置と固定部分を分類
        let ph_index = cmd_parts
            .iter()
            .position(|p| p == "{}" || p == "{/}" || p == "{//}" || p == "{.}" || p == "{/.}");

        if let Some(idx) = ph_index {
            // 単純プレースホルダ（トークン全体が {} 等）の場合:
            //   cmd[..idx] + [paths...] + cmd[idx+1..] の形に展開
            let before: Vec<&String> = cmd_parts[..idx].iter().collect();
            let after: Vec<&String> = cmd_parts[idx + 1..].iter().collect();

            // コマンド固定部分の文字数（分割判定に使用）
            let fixed_len: usize = before
                .iter()
                .chain(after.iter())
                .map(|s| s.len() + 1)
                .sum::<usize>()
                + cmd_parts[0].len();

            // バッチ分割しながら実行
            let mut batch_paths: Vec<std::ffi::OsString> = Vec::new();
            let mut batch_len = fixed_len;
            let mut all_ok = true;

            let flush = |paths: &Vec<std::ffi::OsString>, all_ok: &mut bool| {
                if paths.is_empty() {
                    return;
                }
                let mut cmd = std::process::Command::new(&cmd_parts[0]);
                cmd.args(&cmd_parts[1..idx]);
                cmd.args(paths);
                for a in &after {
                    cmd.arg(a);
                }
                if let Ok(s) = cmd.status() {
                    if !s.success() {
                        *all_ok = false;
                    }
                } else {
                    *all_ok = false;
                }
            };

            for info in results {
                let path_os = info.path.as_os_str();
                let path_len = path_os.len() + 1;
                if !batch_paths.is_empty() && batch_len + path_len >= WINDOWS_CMDLINE_LIMIT {
                    flush(&batch_paths, &mut all_ok);
                    batch_paths.clear();
                    batch_len = fixed_len;
                }
                batch_paths.push(path_os.to_os_string());
                batch_len += path_len;
            }
            flush(&batch_paths, &mut all_ok);
            return all_ok;
        }

        // 複合プレースホルダ（"prefix-{}" 等）の場合:
        //   各ファイルごとに展開した引数を全て連結して 1 コマンドに渡す（fd の実挙動に準拠）
        let mut argv: Vec<std::ffi::OsString> = Vec::new();
        argv.push(std::ffi::OsString::from(&cmd_parts[0]));
        for part in &cmd_parts[1..] {
            for info in results {
                argv.push(
                    FormatTemplate::parse(part)
                        .generate(&info.path, opts.path_separator.as_deref()),
                );
            }
        }
        let status = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status();
        return status.map(|s| s.success()).unwrap_or(false);
    }

    // プレースホルダなし: 全パスを末尾引数として渡す。コマンドライン長を超えたらバッチ分割。
    let fixed_len: usize = cmd_parts.iter().map(|s| s.len() + 1).sum();
    let mut batch_paths: Vec<&std::path::Path> = Vec::new();
    let mut batch_len = fixed_len;
    let mut all_ok = true;

    let flush_plain = |paths: &Vec<&std::path::Path>, all_ok: &mut bool| {
        if paths.is_empty() {
            return;
        }
        let mut cmd = std::process::Command::new(&cmd_parts[0]);
        cmd.args(&cmd_parts[1..]).args(paths);
        if let Ok(s) = cmd.status() {
            if !s.success() {
                *all_ok = false;
            }
        } else {
            *all_ok = false;
        }
    };

    for info in results {
        let path_len = info.path.as_os_str().len() + 1;
        if !batch_paths.is_empty() && batch_len + path_len >= WINDOWS_CMDLINE_LIMIT {
            flush_plain(&batch_paths, &mut all_ok);
            batch_paths.clear();
            batch_len = fixed_len;
        }
        batch_paths.push(&info.path);
        batch_len += path_len;
    }
    flush_plain(&batch_paths, &mut all_ok);
    all_ok
}

fn exec_ls_details(results: &[FileInfo], _opts: &Options) {
    if results.is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        // dir コマンドは内部コマンドのため cmd /C 経由が必要。
        // ただしパスはクォートして渡す。
        let quoted: Vec<String> = results
            .iter()
            .map(|i| quote_arg_for_cmd(&i.path.to_string_lossy()))
            .collect();
        let _ = std::process::Command::new("cmd")
            .args(["/C", &format!("dir {}", quoted.join(" "))])
            .status();
    }
    #[cfg(not(windows))]
    {
        let paths: Vec<&std::path::Path> = results.iter().map(|i| i.path.as_path()).collect();
        let _ = std::process::Command::new("ls")
            .args(["-lhd", "--color=always"])
            .args(&paths)
            .status();
    }
}

fn print_help() {
    println!(
        r#"fd 1.0.0 (fd互換 Rust Windows版) - ファイルシステムエントリを検索するプログラム

使い方: fd [オプション] [パターン] [パス...]

引数:
  [パターン]  検索パターン (正規表現、--glob でglobパターン)
  [パス]...   検索対象ディレクトリ (省略時: カレントディレクトリ)

オプション:
  -H, --hidden                 隠しファイル・ディレクトリを検索
  -I, --no-ignore              .(git|fd)ignore ファイルを無視
  -u, --unrestricted           -I と -H の省略形 (複数指定可)
  -s, --case-sensitive         大文字小文字を区別 (デフォルト: スマートケース)
  -i, --ignore-case            大文字小文字を無視
  -g, --glob                   glob ベースの検索 (デフォルト: 正規表現)
  -F, --fixed-strings          リテラル文字列として扱う
  -a, --absolute-path          絶対パスで表示
  -l, --list-details           詳細表示 (ls -l 形式)
  -L, --follow                 シンボリックリンクを辿る
  -p, --full-path              フルパスで検索
  -d, --max-depth <depth>      最大検索深さ
      --min-depth <depth>      最小検索深さ
      --exact-depth <depth>    指定深さのみ
  -E, --exclude <pattern>      除外パターン
      --prune                  マッチしたディレクトリを探索しない
  -t, --type <filetype>        タイプでフィルタ (f,d,l,x,e,s,p,b,c)
  -e, --extension <ext>        拡張子でフィルタ
  -S, --size <size>            サイズでフィルタ (+100Ki, -1Mi, 50b)
      --changed-within <date>  変更時刻 (より新しい)
      --changed-before <date>  変更時刻 (より古い)
      --format <fmt>           出力フォーマット
  -x, --exec <cmd>...          各結果にコマンド実行
  -X, --exec-batch <cmd>...    全結果にコマンド実行
  -c, --color <when>           色付け (auto/always/never)
      --hyperlink[=<when>]     ハイパーリンク追加
  -0, --print0                 null区切りで出力
  -1                           最初の結果のみ
      --max-results <count>    結果数制限
  -q, --quiet                  出力なし (終了コードのみ)
      --show-errors            エラー表示
  -j, --threads <num>          スレッド数
      --and <pattern>          追加パターン
  -C, --base-directory <path>  作業ディレクトリ変更
      --path-separator <sep>   パス区切り文字
      --one-file-system        同一FS内のみ
  -h, --help                   ヘルプ表示
  -V, --version                バージョン表示

プレースホルダ: {{}} パス, {{/}} ベースネーム, {{//}} 親, {{.}} 拡張子なし, {{/.}} ベースネーム拡張子なし

例:
  fd                    全ファイル
  fd pattern            パターンにマッチ
  fd -e txt             .txt ファイル
  fd -t d               ディレクトリのみ
  fd -H -I pattern      隠しファイル含む
  fd -S +1Mi            1MiB以上
  fd -g '*.txt'         globパターン"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(base: &str, pattern: &str) -> IgnoreRule {
        parse_ignore_rule(pattern, Path::new(base)).unwrap()
    }

    #[test]
    fn glob_with_separator_matches_relative_path() {
        let opts = Options {
            glob: true,
            ..Options::default()
        };
        let matcher = build_matcher(&Some(r"src\*.rs".to_string()), &opts).unwrap();

        assert!(matcher.matches("main.rs", "src/main.rs"));
        assert!(!matcher.matches("main.rs", "nested/src/main.rs"));
    }

    #[test]
    fn exclude_patterns_accept_windows_separators() {
        assert!(is_excluded(
            "main.rs",
            "src/main.rs",
            false,
            &[r"src\*.rs".to_string()],
        ));
        assert!(!is_excluded(
            "main.rs",
            "src/main.rs",
            false,
            &[r"tests\*.rs".to_string()],
        ));
    }

    #[test]
    fn directory_only_patterns_match_subtrees() {
        let rules = vec![rule("repo", r"build\target\")];
        assert!(is_ignored(
            Path::new("repo/build/target"),
            "target",
            true,
            &rules,
        ));
        assert!(!is_ignored(
            Path::new("repo/build/target/artifact.bin"),
            "artifact.bin",
            false,
            &rules,
        ));
    }

    #[test]
    fn negated_ignore_rule_re_includes_path() {
        let rules = vec![rule("repo", "*.log"), rule("repo", "!keep.log")];
        assert!(!is_ignored(
            Path::new("repo/keep.log"),
            "keep.log",
            false,
            &rules
        ));
        assert!(is_ignored(
            Path::new("repo/error.log"),
            "error.log",
            false,
            &rules
        ));
    }

    #[test]
    fn anchored_ignore_rule_is_relative_to_its_directory() {
        let rules = vec![rule("repo/src", "/generated/*.rs")];
        assert!(is_ignored(
            Path::new("repo/src/generated/schema.rs"),
            "schema.rs",
            false,
            &rules,
        ));
        assert!(!is_ignored(
            Path::new("repo/generated/schema.rs"),
            "schema.rs",
            false,
            &rules,
        ));
    }
}
