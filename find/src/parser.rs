use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use glob::glob;
use glob::Pattern;
use regex::{Regex, RegexBuilder};

use crate::expression::*;
use crate::messages;
use crate::platform;

/// グローバルオプション
#[derive(Debug, Clone)]
pub struct GlobalOptions {
    pub follow_symlinks: FollowSymlinks,
    pub max_depth: Option<usize>,
    pub min_depth: Option<usize>,
    pub depth_first: bool,
    pub xdev: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FollowSymlinks {
    Never,       // -P (default)
    Commandline, // -H
    Always,      // -L
}

impl Default for GlobalOptions {
    fn default() -> Self {
        GlobalOptions {
            follow_symlinks: FollowSymlinks::Never,
            max_depth: None,
            min_depth: None,
            depth_first: false,
            xdev: false,
        }
    }
}

/// パース結果
pub struct ParseResult {
    pub paths: Vec<PathBuf>,
    pub expression: Option<Expression>,
    pub global_options: GlobalOptions,
}

/// パーサー
pub struct Parser {
    args: Vec<String>,
    pos: usize,
}

impl Parser {
    pub fn new(args: Vec<String>) -> Self {
        Parser { args, pos: 0 }
    }

    fn current(&self) -> Option<&str> {
        self.args.get(self.pos).map(|s| s.as_str())
    }

    fn advance(&mut self) -> Option<&str> {
        let current = self.args.get(self.pos).map(|s| s.as_str());
        if current.is_some() {
            self.pos += 1;
        }
        current
    }

    fn expect_arg(&mut self, opt: &str) -> Result<String, String> {
        self.advance();
        self.args
            .get(self.pos)
            .cloned()
            .ok_or_else(|| messages::err_missing_argument(opt))
    }

    #[allow(dead_code)]
    fn peek_next(&self) -> Option<&str> {
        self.args.get(self.pos + 1).map(|s| s.as_str())
    }

    pub fn parse(&mut self) -> Result<ParseResult, String> {
        let mut global_options = GlobalOptions::default();
        let mut paths = Vec::new();

        // Parse global options first
        while let Some(arg) = self.current() {
            match arg {
                "-H" => {
                    global_options.follow_symlinks = FollowSymlinks::Commandline;
                    self.advance();
                }
                "-L" => {
                    global_options.follow_symlinks = FollowSymlinks::Always;
                    self.advance();
                }
                "-P" => {
                    global_options.follow_symlinks = FollowSymlinks::Never;
                    self.advance();
                }
                "-D" => {
                    self.advance();
                    self.advance();
                }
                "-O" | "-Olevel" => {
                    self.advance();
                    if self
                        .current()
                        .map(|s| s.parse::<i32>().is_ok())
                        .unwrap_or(false)
                    {
                        self.advance();
                    }
                }
                _ => break,
            }
        }

        // Parse paths
        while let Some(arg) = self.current() {
            if arg.starts_with('-')
                || is_open_paren(arg)
                || is_not_operator(arg)
                || is_list_separator(arg)
            {
                break;
            }
            paths.extend(expand_start_path(arg));
            self.advance();
        }

        if paths.is_empty() {
            paths.push(PathBuf::from("."));
        }

        // Parse positional options
        while let Some(arg) = self.current() {
            match arg {
                "-maxdepth" => {
                    let n_str = self.expect_arg("-maxdepth")?;
                    let n: usize = n_str
                        .parse()
                        .map_err(|_| messages::err_invalid_argument("-maxdepth", &n_str))?;
                    global_options.max_depth = Some(n);
                    self.advance();
                }
                "-mindepth" => {
                    let n_str = self.expect_arg("-mindepth")?;
                    let n: usize = n_str
                        .parse()
                        .map_err(|_| messages::err_invalid_argument("-mindepth", &n_str))?;
                    global_options.min_depth = Some(n);
                    self.advance();
                }
                "-depth" | "-d" => {
                    global_options.depth_first = true;
                    self.advance();
                }
                "-xdev" | "-mount" => {
                    global_options.xdev = true;
                    self.advance();
                }
                "-noleaf"
                | "-ignore_readdir_race"
                | "-noignore_readdir_race"
                | "-warn"
                | "-nowarn" => {
                    self.advance();
                }
                "-regextype" => {
                    self.advance();
                    self.advance();
                }
                "-help" | "--help" => {
                    println!("{}", messages::HELP_MESSAGE);
                    std::process::exit(0);
                }
                "-version" | "--version" => {
                    println!("find 1.0.0");
                    std::process::exit(0);
                }
                _ => break,
            }
        }

        let expression = if self.current().is_some() {
            Some(self.parse_expression()?)
        } else {
            None
        };

        if let Some(arg) = self.current() {
            return if is_close_paren(arg) {
                Err(messages::err_unmatched_paren())
            } else {
                Err(messages::err_unknown_option(arg))
            };
        }

        if let Some(ref expr) = expression {
            if Self::contains_delete(expr) {
                global_options.depth_first = true;
            }
        }

        Ok(ParseResult {
            paths,
            expression,
            global_options,
        })
    }

    fn contains_delete(expr: &Expression) -> bool {
        match expr {
            Expression::Action(Action::Delete) => true,
            Expression::Not(e) => Self::contains_delete(e),
            Expression::And(a, b) | Expression::Or(a, b) | Expression::List(a, b) => {
                Self::contains_delete(a) || Self::contains_delete(b)
            }
            _ => false,
        }
    }

    fn parse_expression(&mut self) -> Result<Expression, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_and()?;

        while let Some(arg) = self.current() {
            if arg == "-o" || arg == "-or" {
                self.advance();
                let right = self.parse_and()?;
                left = Expression::Or(Box::new(left), Box::new(right));
            } else if is_list_separator(arg) {
                self.advance();
                let right = self.parse_and()?;
                left = Expression::List(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }

        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_unary()?;

        while let Some(arg) = self.current() {
            if arg == "-a" || arg == "-and" {
                self.advance();
                let right = self.parse_unary()?;
                left = Expression::And(Box::new(left), Box::new(right));
            } else if !is_or_or_list_or_close(arg) && !arg.is_empty() {
                // 次のトークンが新しい primary を開始するか確認する。
                // 先頭が '-' のオプション、または '!' / '(' のみを暗黙的 AND の対象とする。
                // それ以外（'+', ';' など exec の終端トークンが漏れてきた場合）は break。
                if !is_not_operator(arg) && !is_open_paren(arg) && !arg.starts_with('-') {
                    break;
                }
                // エラーは呼び出し元に伝播させる（飲み込まない）
                let right = self.parse_unary()?;
                left = Expression::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }

        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expression, String> {
        match self.current() {
            Some(arg) if is_not_operator(arg) || arg == "-not" => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expression::Not(Box::new(expr)))
            }
            Some(arg) if is_open_paren(arg) => {
                self.advance();
                let expr = self.parse_expression()?;
                if !self.current().map(is_close_paren).unwrap_or(false) {
                    return Err(messages::err_unmatched_paren());
                }
                self.advance();
                Ok(expr)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expression, String> {
        let arg = self
            .current()
            .ok_or_else(|| "式が必要です".to_string())?
            .to_string();
        let arg_ref = arg.as_str();

        match arg_ref {
            "-name" => {
                let pattern_str = self.expect_arg("-name")?;
                let pattern = Pattern::new(&normalize_glob_pattern(&pattern_str))
                    .map_err(|e| messages::err_invalid_argument("-name", &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Name {
                    pattern,
                    case_insensitive: false,
                }))
            }

            "-iname" => {
                let pattern_str = self.expect_arg("-iname")?;
                let pattern = Pattern::new(&normalize_glob_pattern(&pattern_str.to_lowercase()))
                    .map_err(|e| messages::err_invalid_argument("-iname", &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Name {
                    pattern,
                    case_insensitive: true,
                }))
            }

            "-path" | "-wholename" => {
                let pattern_str = self.expect_arg(&arg)?;
                let pattern = Pattern::new(&normalize_glob_pattern(&pattern_str))
                    .map_err(|e| messages::err_invalid_argument(&arg, &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Path {
                    pattern,
                    case_insensitive: false,
                }))
            }

            "-ipath" | "-iwholename" => {
                let pattern_str = self.expect_arg(&arg)?;
                let pattern = Pattern::new(&normalize_glob_pattern(&pattern_str.to_lowercase()))
                    .map_err(|e| messages::err_invalid_argument(&arg, &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Path {
                    pattern,
                    case_insensitive: true,
                }))
            }

            "-regex" => {
                let regex_str = self.expect_arg("-regex")?;
                let regex = Regex::new(&regex_str)
                    .map_err(|e| messages::err_invalid_regex(&regex_str, &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Regex {
                    regex,
                    case_insensitive: false,
                }))
            }

            "-iregex" => {
                let regex_str = self.expect_arg("-iregex")?;
                let regex = RegexBuilder::new(&regex_str)
                    .case_insensitive(true)
                    .build()
                    .map_err(|e| messages::err_invalid_regex(&regex_str, &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Regex {
                    regex,
                    case_insensitive: true,
                }))
            }

            "-type" => {
                let type_str = self.expect_arg("-type")?;
                if type_str.len() != 1 {
                    return Err(messages::err_invalid_type(&type_str));
                }
                let ft = FileType::from_char(type_str.chars().next().unwrap())
                    .ok_or_else(|| messages::err_invalid_type(&type_str))?;
                self.advance();
                Ok(Expression::Test(Test::Type(ft)))
            }

            "-xtype" => {
                let type_str = self.expect_arg("-xtype")?;
                if type_str.len() != 1 {
                    return Err(messages::err_invalid_type(&type_str));
                }
                let ft = FileType::from_char(type_str.chars().next().unwrap())
                    .ok_or_else(|| messages::err_invalid_type(&type_str))?;
                self.advance();
                Ok(Expression::Test(Test::Xtype(ft)))
            }

            "-size" => {
                let size_str = self.expect_arg("-size")?;
                let size_comp = SizeComparison::parse(&size_str)
                    .ok_or_else(|| messages::err_invalid_size(&size_str))?;
                self.advance();
                Ok(Expression::Test(Test::Size(size_comp)))
            }

            "-empty" => {
                self.advance();
                Ok(Expression::Test(Test::Empty))
            }

            "-atime" => {
                let n_str = self.expect_arg("-atime")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Time {
                    time_type: TimeType::Access,
                    comparison: comp,
                    minutes: false,
                }))
            }

            "-ctime" => {
                let n_str = self.expect_arg("-ctime")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Time {
                    time_type: TimeType::Change,
                    comparison: comp,
                    minutes: false,
                }))
            }

            "-mtime" => {
                let n_str = self.expect_arg("-mtime")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Time {
                    time_type: TimeType::Modify,
                    comparison: comp,
                    minutes: false,
                }))
            }

            "-amin" => {
                let n_str = self.expect_arg("-amin")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Time {
                    time_type: TimeType::Access,
                    comparison: comp,
                    minutes: true,
                }))
            }

            "-cmin" => {
                let n_str = self.expect_arg("-cmin")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Time {
                    time_type: TimeType::Change,
                    comparison: comp,
                    minutes: true,
                }))
            }

            "-mmin" => {
                let n_str = self.expect_arg("-mmin")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Time {
                    time_type: TimeType::Modify,
                    comparison: comp,
                    minutes: true,
                }))
            }

            "-newer" => {
                let file = self.expect_arg("-newer")?;
                let meta = fs::metadata(&file).map_err(|_| messages::err_file_not_found(&file))?;
                let mtime = meta
                    .modified()
                    .map_err(|_| messages::err_file_not_found(&file))?;
                self.advance();
                Ok(Expression::Test(Test::Newer {
                    reference_time: mtime,
                }))
            }

            s if s.starts_with("-newer") && s.len() >= 8 => {
                let arg_clone = arg.clone();
                let suffix = &arg_clone[6..];
                let chars: Vec<char> = suffix.chars().collect();
                if chars.len() >= 2 {
                    let x = match chars[0] {
                        'a' => TimeType::Access,
                        'c' => TimeType::Change,
                        'm' => TimeType::Modify,
                        _ => return Err(messages::err_unknown_option(&arg)),
                    };
                    let (y, reference) = if chars[1] == 't' {
                        let time_str = self.expect_arg(&arg)?;
                        let time = parse_datetime(&time_str)
                            .ok_or_else(|| messages::err_invalid_argument(&arg, &time_str))?;
                        self.advance();
                        (TimeType::Modify, time)
                    } else {
                        let y = match chars[1] {
                            'a' => TimeType::Access,
                            'c' => TimeType::Change,
                            'm' => TimeType::Modify,
                            _ => return Err(messages::err_unknown_option(&arg)),
                        };
                        let file = self.expect_arg(&arg)?;
                        let meta =
                            fs::metadata(&file).map_err(|_| messages::err_file_not_found(&file))?;
                        let reference = match y {
                            TimeType::Access => meta.accessed().unwrap_or(UNIX_EPOCH),
                            TimeType::Change => platform::get_ctime(&meta),
                            TimeType::Modify => meta.modified().unwrap_or(UNIX_EPOCH),
                        };
                        self.advance();
                        (y, reference)
                    };
                    Ok(Expression::Test(Test::NewerXY { x, y, reference }))
                } else {
                    Err(messages::err_unknown_option(&arg))
                }
            }

            "-user" => {
                let user_str = self.expect_arg("-user")?;
                let uid = if let Ok(id) = user_str.parse::<u32>() {
                    id
                } else {
                    platform::get_user_by_name(&user_str)
                        .ok_or_else(|| messages::err_user_not_found(&user_str))?
                };
                self.advance();
                Ok(Expression::Test(Test::User(uid)))
            }

            "-group" => {
                let group_str = self.expect_arg("-group")?;
                let gid = if let Ok(id) = group_str.parse::<u32>() {
                    id
                } else {
                    platform::get_group_by_name(&group_str)
                        .ok_or_else(|| messages::err_group_not_found(&group_str))?
                };
                self.advance();
                Ok(Expression::Test(Test::Group(gid)))
            }

            "-uid" => {
                let n_str = self.expect_arg("-uid")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_argument("-uid", &n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Uid(comp)))
            }

            "-gid" => {
                let n_str = self.expect_arg("-gid")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_argument("-gid", &n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Gid(comp)))
            }

            "-nouser" => {
                self.advance();
                Ok(Expression::Test(Test::NoUser))
            }

            "-nogroup" => {
                self.advance();
                Ok(Expression::Test(Test::NoGroup))
            }

            "-perm" => {
                let perm_str = self.expect_arg("-perm")?;
                let perm = PermMode::parse(&perm_str)
                    .ok_or_else(|| messages::err_invalid_perm(&perm_str))?;
                self.advance();
                Ok(Expression::Test(Test::Perm(perm)))
            }

            "-readable" => {
                self.advance();
                Ok(Expression::Test(Test::Readable))
            }

            "-writable" => {
                self.advance();
                Ok(Expression::Test(Test::Writable))
            }

            "-executable" => {
                self.advance();
                Ok(Expression::Test(Test::Executable))
            }

            "-links" => {
                let n_str = self.expect_arg("-links")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_argument("-links", &n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Links(comp)))
            }

            "-inum" => {
                let n_str = self.expect_arg("-inum")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_argument("-inum", &n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Inum(comp)))
            }

            "-samefile" => {
                let file = self.expect_arg("-samefile")?;
                let meta = fs::metadata(&file).map_err(|_| messages::err_file_not_found(&file))?;
                self.advance();
                Ok(Expression::Test(Test::Samefile {
                    dev: platform::get_dev(&meta),
                    ino: platform::get_ino(&meta),
                }))
            }

            "-true" => {
                self.advance();
                Ok(Expression::Test(Test::True))
            }

            "-false" => {
                self.advance();
                Ok(Expression::Test(Test::False))
            }

            "-print" => {
                self.advance();
                Ok(Expression::Action(Action::Print))
            }

            "-print0" => {
                self.advance();
                Ok(Expression::Action(Action::Print0))
            }

            "-fprint" => {
                let file = self.expect_arg("-fprint")?;
                self.advance();
                Ok(Expression::Action(Action::FPrint(file)))
            }

            "-fprint0" => {
                let file = self.expect_arg("-fprint0")?;
                self.advance();
                Ok(Expression::Action(Action::FPrint0(file)))
            }

            "-printf" => {
                let format = self.expect_arg("-printf")?;
                self.advance();
                Ok(Expression::Action(Action::Printf(format)))
            }

            "-ls" => {
                self.advance();
                Ok(Expression::Action(Action::Ls))
            }

            "-fls" => {
                let file = self.expect_arg("-fls")?;
                self.advance();
                Ok(Expression::Action(Action::FLs(file)))
            }

            "-exec" | "-execdir" => {
                let in_dir = arg_ref == "-execdir";
                let (command, exec_type) = self.parse_exec_command(&arg)?;
                Ok(Expression::Action(Action::Exec {
                    command,
                    exec_type,
                    in_dir,
                }))
            }

            "-ok" | "-okdir" => {
                let in_dir = arg_ref == "-okdir";
                let (command, exec_type) = self.parse_exec_command(&arg)?;
                if exec_type == ExecType::Batch {
                    return Err(messages::err_ok_no_batch(&arg));
                }
                Ok(Expression::Action(Action::Ok { command, in_dir }))
            }

            "-delete" => {
                self.advance();
                Ok(Expression::Action(Action::Delete))
            }

            "-prune" => {
                self.advance();
                Ok(Expression::Action(Action::Prune))
            }

            "-quit" => {
                self.advance();
                Ok(Expression::Action(Action::Quit))
            }

            _ => Err(messages::err_unknown_option(&arg)),
        }
    }

    fn parse_exec_command(&mut self, opt: &str) -> Result<(Vec<String>, ExecType), String> {
        self.advance();

        let mut command = Vec::new();
        let mut exec_type = ExecType::Each;

        loop {
            match self.current() {
                Some(arg) if is_exec_terminator(arg) => {
                    self.advance();
                    break;
                }
                Some("+") => {
                    if command.last().map(String::as_str) == Some("{}") {
                        exec_type = ExecType::Batch;
                        self.advance();
                        break;
                    } else if command
                        .last()
                        .map(|s: &String| s.contains("{}"))
                        .unwrap_or(false)
                    {
                        // GNU find と同様に {} が単独トークンでない場合は構文エラー
                        let bad = command.last().unwrap().clone();
                        return Err(messages::err_exec_batch_partial_placeholder(&bad));
                    } else {
                        command.push("+".to_string());
                        self.advance();
                    }
                }
                Some(arg) => {
                    command.push(arg.to_string());
                    self.advance();
                }
                None => {
                    return Err(messages::err_missing_exec_terminator());
                }
            }
        }

        if command.is_empty() {
            return Err(messages::err_missing_argument(opt));
        }

        Ok((command, exec_type))
    }
}

fn expand_start_path(arg: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        if contains_glob_metachar(arg) {
            let mut expanded = Vec::new();
            if let Ok(entries) = glob(&normalize_glob_pattern(arg)) {
                for entry in entries.flatten() {
                    expanded.push(entry);
                }
            }
            if !expanded.is_empty() {
                return expanded;
            }
        }
    }

    vec![PathBuf::from(arg)]
}

fn normalize_glob_pattern(pattern: &str) -> String {
    #[cfg(windows)]
    {
        pattern.replace('\\', "/")
    }

    #[cfg(not(windows))]
    {
        pattern.to_string()
    }
}

#[cfg(windows)]
fn contains_glob_metachar(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn is_open_paren(arg: &str) -> bool {
    arg == "(" || arg == "\\("
}

fn is_close_paren(arg: &str) -> bool {
    arg == ")" || arg == "\\)"
}

fn is_not_operator(arg: &str) -> bool {
    arg == "!" || arg == "\\!"
}

fn is_list_separator(arg: &str) -> bool {
    arg == ","
}

fn is_or_or_list_or_close(arg: &str) -> bool {
    matches!(arg, "-o" | "-or") || is_list_separator(arg) || is_close_paren(arg)
}

fn is_exec_terminator(arg: &str) -> bool {
    arg == ";" || arg == "\\;"
}

fn parse_datetime(s: &str) -> Option<SystemTime> {
    use chrono::{Local, NaiveDateTime, TimeZone};

    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%Y/%m/%d",
    ];

    for fmt in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            if let Some(local_dt) = Local.from_local_datetime(&dt).single() {
                let timestamp = local_dt.timestamp();
                return Some(UNIX_EPOCH + Duration::from_secs(timestamp as u64));
            }
        }
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date.and_hms_opt(0, 0, 0)?;
        if let Some(local_dt) = Local.from_local_datetime(&dt).single() {
            let timestamp = local_dt.timestamp();
            return Some(UNIX_EPOCH + Duration::from_secs(timestamp as u64));
        }
    }

    None
}
