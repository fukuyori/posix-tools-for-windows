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

/// -regextype で選択される正規表現の方言
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegexSyntax {
    /// POSIX 拡張正規表現（デフォルト。GNU find の posix-extended 相当）
    Extended,
    /// POSIX 基本正規表現（\( \) \{ \} が特殊、( ) { } + ? | はリテラル）
    Basic,
    /// GNU Emacs 風（\( \) \| が特殊、( ) | { } はリテラル、+ ? は特殊）
    Emacs,
}

/// パーサー
pub struct Parser {
    args: Vec<String>,
    pos: usize,
    options: GlobalOptions,
    /// -daystart が出現したか（それ以降の時間テストに適用される）
    daystart: bool,
    /// -regextype で指定された方言（それ以降の -regex/-iregex に適用される）
    regex_syntax: RegexSyntax,
}

impl Parser {
    pub fn new(args: Vec<String>) -> Self {
        Parser {
            args,
            pos: 0,
            options: GlobalOptions::default(),
            daystart: false,
            regex_syntax: RegexSyntax::Extended,
        }
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
        let mut paths = Vec::new();

        // Parse global options first
        while let Some(arg) = self.current() {
            match arg {
                "-H" => {
                    self.options.follow_symlinks = FollowSymlinks::Commandline;
                    self.advance();
                }
                "-L" => {
                    self.options.follow_symlinks = FollowSymlinks::Always;
                    self.advance();
                }
                "-P" => {
                    self.options.follow_symlinks = FollowSymlinks::Never;
                    self.advance();
                }
                "-D" => {
                    self.advance();
                    self.advance();
                }
                // -O3 のような結合形式
                s if s.starts_with("-O") && s[2..].chars().all(|c| c.is_ascii_digit()) => {
                    let bare = s == "-O";
                    self.advance();
                    // "-O" 単独の場合は次のトークンがレベル数値のことがある
                    if bare
                        && self
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
            if (arg.starts_with('-') && arg.len() > 1)
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

        // 位置オプション（-maxdepth, -daystart, -regextype など）は
        // parse_primary() 内で式の一部（常に真）として処理される。
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
                self.options.depth_first = true;
            }
        }

        Ok(ParseResult {
            paths,
            expression,
            global_options: self.options.clone(),
        })
    }

    /// 位置オプション／グローバルオプションを式の途中でも受け付ける（GNU 互換）。
    /// オプションとして消費した場合は Some(Ok(()))、
    /// オプションだが引数が不正な場合は Some(Err)、
    /// オプションでない場合は None を返す。
    fn try_parse_option(&mut self, arg: &str) -> Option<Result<(), String>> {
        match arg {
            "-maxdepth" => Some(self.parse_depth_option("-maxdepth", true)),
            "-mindepth" => Some(self.parse_depth_option("-mindepth", false)),
            "-depth" | "-d" => {
                self.options.depth_first = true;
                self.advance();
                Some(Ok(()))
            }
            "-xdev" | "-mount" => {
                self.options.xdev = true;
                self.advance();
                Some(Ok(()))
            }
            "-follow" => {
                self.options.follow_symlinks = FollowSymlinks::Always;
                self.advance();
                Some(Ok(()))
            }
            "-daystart" => {
                self.daystart = true;
                self.advance();
                Some(Ok(()))
            }
            "-noleaf" | "-ignore_readdir_race" | "-noignore_readdir_race" | "-warn"
            | "-nowarn" => {
                self.advance();
                Some(Ok(()))
            }
            "-regextype" => {
                let name = match self.expect_arg("-regextype") {
                    Ok(n) => n,
                    Err(e) => return Some(Err(e)),
                };
                match parse_regextype(&name) {
                    Some(syntax) => {
                        self.regex_syntax = syntax;
                        self.advance();
                        Some(Ok(()))
                    }
                    None => Some(Err(messages::err_invalid_regextype(&name))),
                }
            }
            _ => None,
        }
    }

    fn parse_depth_option(&mut self, opt: &str, is_max: bool) -> Result<(), String> {
        let n_str = self.expect_arg(opt)?;
        let n: usize = n_str
            .parse()
            .map_err(|_| messages::err_invalid_argument(opt, &n_str))?;
        if is_max {
            self.options.max_depth = Some(n);
        } else {
            self.options.min_depth = Some(n);
        }
        self.advance();
        Ok(())
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

        // 位置オプション／グローバルオプションは常に真の式として扱う（GNU 互換）
        if let Some(result) = self.try_parse_option(arg_ref) {
            result?;
            return Ok(Expression::Test(Test::True));
        }

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

            "-lname" => {
                let pattern_str = self.expect_arg("-lname")?;
                let pattern = Pattern::new(&normalize_glob_pattern(&pattern_str))
                    .map_err(|e| messages::err_invalid_argument("-lname", &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Lname {
                    pattern,
                    case_insensitive: false,
                }))
            }

            "-ilname" => {
                let pattern_str = self.expect_arg("-ilname")?;
                let pattern = Pattern::new(&normalize_glob_pattern(&pattern_str.to_lowercase()))
                    .map_err(|e| messages::err_invalid_argument("-ilname", &e.to_string()))?;
                self.advance();
                Ok(Expression::Test(Test::Lname {
                    pattern,
                    case_insensitive: true,
                }))
            }

            "-regex" => {
                let regex_str = self.expect_arg("-regex")?;
                let regex = self.build_path_regex(&regex_str, false)?;
                self.advance();
                Ok(Expression::Test(Test::Regex {
                    regex,
                    case_insensitive: false,
                }))
            }

            "-iregex" => {
                let regex_str = self.expect_arg("-iregex")?;
                let regex = self.build_path_regex(&regex_str, true)?;
                self.advance();
                Ok(Expression::Test(Test::Regex {
                    regex,
                    case_insensitive: true,
                }))
            }

            "-type" => self.parse_type_test("-type", false),

            "-xtype" => self.parse_type_test("-xtype", true),

            "-fstype" => {
                let fstype = self.expect_arg("-fstype")?;
                self.advance();
                Ok(Expression::Test(Test::Fstype(fstype)))
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

            "-atime" => self.parse_time_test("-atime", TimeType::Access, false),
            "-ctime" => self.parse_time_test("-ctime", TimeType::Change, false),
            "-mtime" => self.parse_time_test("-mtime", TimeType::Modify, false),
            "-amin" => self.parse_time_test("-amin", TimeType::Access, true),
            "-cmin" => self.parse_time_test("-cmin", TimeType::Change, true),
            "-mmin" => self.parse_time_test("-mmin", TimeType::Modify, true),

            "-used" => {
                let n_str = self.expect_arg("-used")?;
                let comp = NumericComparison::parse(&n_str)
                    .ok_or_else(|| messages::err_invalid_time(&n_str))?;
                self.advance();
                Ok(Expression::Test(Test::Used(comp)))
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

            "-anewer" | "-cnewer" => {
                let x = if arg_ref == "-anewer" {
                    TimeType::Access
                } else {
                    TimeType::Change
                };
                let file = self.expect_arg(&arg)?;
                let meta = fs::metadata(&file).map_err(|_| messages::err_file_not_found(&file))?;
                let mtime = meta
                    .modified()
                    .map_err(|_| messages::err_file_not_found(&file))?;
                self.advance();
                Ok(Expression::Test(Test::NewerXY {
                    x,
                    y: TimeType::Modify,
                    reference: mtime,
                }))
            }

            s if s.starts_with("-newer") && s.len() >= 8 => {
                let arg_clone = arg.clone();
                let suffix = &arg_clone[6..];
                let chars: Vec<char> = suffix.chars().collect();
                if chars.len() == 2 {
                    let x = parse_time_type_char(chars[0])
                        .ok_or_else(|| messages::err_unknown_option(&arg))?;
                    let (y, reference) = if chars[1] == 't' {
                        let time_str = self.expect_arg(&arg)?;
                        let time = parse_datetime(&time_str)
                            .ok_or_else(|| messages::err_invalid_argument(&arg, &time_str))?;
                        self.advance();
                        (TimeType::Modify, time)
                    } else {
                        let y = parse_time_type_char(chars[1])
                            .ok_or_else(|| messages::err_unknown_option(&arg))?;
                        let file = self.expect_arg(&arg)?;
                        let meta =
                            fs::metadata(&file).map_err(|_| messages::err_file_not_found(&file))?;
                        let reference = match y {
                            TimeType::Access => meta.accessed().unwrap_or(UNIX_EPOCH),
                            TimeType::Change => platform::get_ctime(&meta),
                            TimeType::Modify => meta.modified().unwrap_or(UNIX_EPOCH),
                            TimeType::Birth => platform::get_btime(&meta)
                                .ok_or_else(|| messages::err_birth_time_unsupported(&file))?,
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
                fs::metadata(&file).map_err(|_| messages::err_file_not_found(&file))?;
                let (dev, ino, _) = platform::get_file_ids(std::path::Path::new(&file), true)
                    .ok_or_else(|| messages::err_file_not_found(&file))?;
                self.advance();
                Ok(Expression::Test(Test::Samefile { dev, ino }))
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

            "-fprintf" => {
                let file = self.expect_arg("-fprintf")?;
                self.advance();
                let format = self
                    .args
                    .get(self.pos)
                    .cloned()
                    .ok_or_else(|| messages::err_missing_argument("-fprintf"))?;
                self.advance();
                Ok(Expression::Action(Action::FPrintf(file, format)))
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

    /// -atime / -mmin などの時間テストを組み立てる。
    /// それまでに -daystart が指定されていれば daystart フラグを立てる。
    fn parse_time_test(
        &mut self,
        opt: &str,
        time_type: TimeType,
        minutes: bool,
    ) -> Result<Expression, String> {
        let n_str = self.expect_arg(opt)?;
        let comp =
            NumericComparison::parse(&n_str).ok_or_else(|| messages::err_invalid_time(&n_str))?;
        self.advance();
        Ok(Expression::Test(Test::Time {
            time_type,
            comparison: comp,
            minutes,
            daystart: self.daystart,
        }))
    }

    /// -type / -xtype。GNU 4.10 互換で `f,d` のようなカンマ区切りを OR として展開する。
    fn parse_type_test(&mut self, opt: &str, xtype: bool) -> Result<Expression, String> {
        let type_str = self.expect_arg(opt)?;
        self.advance();

        let mut types = Vec::new();
        for part in type_str.split(',') {
            let mut chars = part.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(messages::err_invalid_type(&type_str));
            };
            let ft = FileType::from_char(c).ok_or_else(|| messages::err_invalid_type(&type_str))?;
            types.push(ft);
        }

        let make_test = |ft| {
            Expression::Test(if xtype {
                Test::Xtype(ft)
            } else {
                Test::Type(ft)
            })
        };

        let mut iter = types.into_iter();
        let first = iter
            .next()
            .ok_or_else(|| messages::err_invalid_type(&type_str))?;
        let mut expr = make_test(first);
        for ft in iter {
            expr = Expression::Or(Box::new(expr), Box::new(make_test(ft)));
        }
        Ok(expr)
    }

    /// -regex / -iregex 用の正規表現を組み立てる。
    /// GNU find と同様、パス全体にマッチするよう暗黙にアンカーする。
    fn build_path_regex(&self, pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
        let converted = convert_regex_syntax(pattern, self.regex_syntax);
        let anchored = format!("^(?:{})$", converted);
        RegexBuilder::new(&anchored)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|e| messages::err_invalid_regex(pattern, &e.to_string()))
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

fn parse_time_type_char(c: char) -> Option<TimeType> {
    match c {
        'a' => Some(TimeType::Access),
        'c' => Some(TimeType::Change),
        'm' => Some(TimeType::Modify),
        'B' => Some(TimeType::Birth),
        _ => None,
    }
}

/// -regextype の名前を方言に対応付ける
fn parse_regextype(name: &str) -> Option<RegexSyntax> {
    match name {
        "emacs" | "findutils-default" => Some(RegexSyntax::Emacs),
        "posix-basic" | "posix-minimal-basic" | "ed" | "sed" | "grep" => Some(RegexSyntax::Basic),
        "posix-extended" | "posix-egrep" | "egrep" | "posix-awk" | "awk" | "gnu-awk" => {
            Some(RegexSyntax::Extended)
        }
        _ => None,
    }
}

/// BRE / Emacs 方言を rust の regex クレート（ERE 相当）の構文へ変換する。
///
/// * Basic: `\( \) \{ \} \| \+ \?` が特殊 → `( ) { } | + ?` へ昇格。
///          裸の `( ) { } | + ?` はリテラル → エスケープする。
/// * Emacs: `\( \) \|` が特殊 → 昇格。裸の `( ) |` と `{ }` はリテラル。
///          `+ ?` は Emacs でも特殊なのでそのまま。
/// * Extended: 無変換。
pub(crate) fn convert_regex_syntax(pattern: &str, syntax: RegexSyntax) -> String {
    if syntax == RegexSyntax::Extended {
        return pattern.to_string();
    }
    let basic = syntax == RegexSyntax::Basic;

    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n) => {
                    let promote = match n {
                        '(' | ')' | '|' => true,
                        '{' | '}' | '+' | '?' => basic,
                        _ => false,
                    };
                    if promote {
                        out.push(n);
                    } else {
                        out.push('\\');
                        out.push(n);
                    }
                }
                None => out.push('\\'),
            },
            '(' | ')' | '|' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '+' | '?' if basic => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn parse_datetime(s: &str) -> Option<SystemTime> {
    use chrono::{Local, NaiveDateTime, TimeZone};

    // "@エポック秒" 形式（GNU 互換）
    if let Some(epoch_str) = s.strip_prefix('@') {
        if let Ok(secs) = epoch_str.parse::<i64>() {
            return if secs >= 0 {
                Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
            } else {
                UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
            };
        }
    }

    // RFC 3339 / ISO 8601（タイムゾーン付き）
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let ts = dt.timestamp();
        if ts >= 0 {
            return Some(UNIX_EPOCH + Duration::from_secs(ts as u64));
        }
    }

    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
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
