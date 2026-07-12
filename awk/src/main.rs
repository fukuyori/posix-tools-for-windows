/// AWK - A Rust implementation of POSIX AWK
mod ast;
mod builtins;
mod interpreter;
mod lexer;
mod parser;
mod regex_compat;
mod value;

use glob::{glob_with, MatchOptions};
use interpreter::Interpreter;
use parser::Parser;
use std::env;
use std::fs::File;
use std::io::{self, BufReader};
use std::process;

fn print_usage() {
    eprintln!("使用法: awk [-F fs] [-v var=value]... 'プログラム' [ファイル...]");
    eprintln!("        awk [-F fs] [-v var=value]... -f プログラムファイル [ファイル...]");
    eprintln!();
    eprintln!("オプション:");
    eprintln!("  -F fs         フィールド区切り文字を fs に設定");
    eprintln!("  -v var=value  変数 var に value を代入");
    eprintln!("  -f progfile   プログラムをファイルから読み込む");
    eprintln!("  --help        このヘルプメッセージを表示");
    eprintln!("  --version     バージョン情報を表示");
}

fn print_version() {
    eprintln!("awk-rs 1.0.0");
    eprintln!("Rust で実装された POSIX AWK");
}

struct Config {
    program: String,
    files: Vec<String>,
    awk_argv: Vec<String>,
    field_separator: Option<String>,
    variables: Vec<(String, String)>,
}

fn parse_args() -> Result<Config, String> {
    parse_args_from(env::args())
}

fn parse_args_from<I>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let mut config = Config {
        program: String::new(),
        files: Vec::new(),
        // POSIX: ARGV[0] is the name of the awk utility, not its full path
        awk_argv: vec!["awk".to_string()],
        field_separator: None,
        variables: Vec::new(),
    };

    let mut i = 1;
    let mut program_set = false;
    let mut options_done = false;

    while i < args.len() {
        let arg = &args[i];

        if !options_done && arg == "--" {
            options_done = true;
            i += 1;
            continue;
        }

        if options_done || !arg.starts_with('-') || arg == "-" {
            if !program_set {
                config.program = arg.clone();
                program_set = true;
            } else if parse_assignment_operand(arg).is_some() {
                // var=value operand: keep literally (no glob expansion)
                config.files.push(arg.clone());
            } else {
                let expanded = expand_path_argument(arg)
                    .map_err(|e| format!("入力ファイル '{}' を展開できません: {}", arg, e))?;
                config.files.extend(expanded);
            }
            i += 1;
            continue;
        }

        if arg == "--help" {
            print_usage();
            process::exit(0);
        } else if arg == "--version" {
            print_version();
            process::exit(0);
        } else if arg == "-F" {
            i += 1;
            if i >= args.len() {
                return Err("-F には引数が必要です".to_string());
            }
            config.field_separator = Some(process_escapes(&args[i]));
        } else if arg.starts_with("-F") {
            config.field_separator = Some(process_escapes(&arg[2..]));
        } else if arg == "-v" {
            i += 1;
            if i >= args.len() {
                return Err("-v には引数が必要です".to_string());
            }
            let assignment = &args[i];
            if let Some((name, value)) = parse_assignment_operand(assignment) {
                config.variables.push((name, value));
            } else {
                return Err(format!("無効な変数代入: {}", assignment));
            }
        } else if arg.starts_with("-v") {
            let assignment = &arg[2..];
            if let Some((name, value)) = parse_assignment_operand(assignment) {
                config.variables.push((name, value));
            } else {
                return Err(format!("無効な変数代入: {}", assignment));
            }
        } else if arg == "-f" {
            i += 1;
            if i >= args.len() {
                return Err("-f には引数が必要です".to_string());
            }
            let prog_files = expand_path_argument(&args[i])
                .map_err(|e| format!("プログラムファイル '{}' を展開できません: {}", args[i], e))?;
            let prog_file = match prog_files.as_slice() {
                [] => &args[i],
                [single] => single,
                _ => {
                    return Err(format!(
                        "プログラムファイル '{}' が複数に展開されました",
                        args[i]
                    ))
                }
            };
            let program = std::fs::read_to_string(prog_file).map_err(|e| {
                format!("プログラムファイル '{}' を読み込めません: {}", prog_file, e)
            })?;
            config.program.push_str(&program);
            config.program.push('\n');
            program_set = true;
        } else {
            return Err(format!("不明なオプション: {}", arg));
        }

        i += 1;
    }

    config.awk_argv.extend(config.files.iter().cloned());

    if !program_set {
        return Err("プログラムが指定されていません".to_string());
    }

    Ok(config)
}

/// Process escape sequences in -v/-F/command-line assignment values
/// (POSIX: these undergo the same processing as string literals).
fn process_escapes(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('/') => out.push('/'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some(d @ '0'..='7') => {
                let mut code = d as u32 - '0' as u32;
                for _ in 0..2 {
                    match chars.peek() {
                        Some(&e @ '0'..='7') => {
                            code = code * 8 + (e as u32 - '0' as u32);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Check whether a command-line operand is a `var=value` assignment.
/// Returns (name, value) with escape processing applied to the value.
fn parse_assignment_operand(arg: &str) -> Option<(String, String)> {
    let eq = arg.find('=')?;
    let name = &arg[..eq];
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((name.to_string(), process_escapes(&arg[eq + 1..])))
}

fn expand_path_argument(arg: &str) -> Result<Vec<String>, glob::PatternError> {
    if arg == "-" || !contains_glob_meta(arg) {
        return Ok(vec![arg.to_string()]);
    }

    let options = MatchOptions {
        case_sensitive: !cfg!(windows),
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };

    let mut matches = glob_with(arg, options)?
        .filter_map(Result::ok)
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    if matches.is_empty() {
        Ok(vec![arg.to_string()])
    } else {
        matches.sort_by_key(|path| path.to_ascii_lowercase());
        Ok(matches)
    }
}

fn contains_glob_meta(arg: &str) -> bool {
    arg.contains('*') || arg.contains('?') || arg.contains('[')
}

fn run(config: Config) -> Result<i32, String> {
    let mut parser = Parser::new(&config.program).map_err(|e| format!("構文エラー: {}", e))?;
    let program = parser.parse().map_err(|e| format!("構文エラー: {}", e))?;

    let output: Box<dyn std::io::Write> = Box::new(io::stdout());
    let mut interp = Interpreter::new(&program, output);
    interp.set_argv(&config.awk_argv);
    interp.set_environ(env::vars());

    if let Some(fs) = &config.field_separator {
        interp.set_var("FS", fs);
    }

    for (name, value) in &config.variables {
        interp.set_var(name, value);
    }

    let begin_exited = interp
        .run_begin_rules()
        .map_err(|e| format!("実行エラー: {}", e))?
        .is_some();

    if !begin_exited {
        let operands = interp.input_file_args();
        // File operands, excluding var=value assignments
        let has_real_file = operands.iter().any(|o| parse_assignment_operand(o).is_none());

        if !has_real_file {
            // Apply any assignment operands, then read stdin
            for operand in &operands {
                if let Some((name, value)) = parse_assignment_operand(operand) {
                    interp.set_var(&name, &value);
                }
            }
            let uses_stdin = program_reads_input(&program);
            if uses_stdin {
                let stdin = io::stdin();
                let reader = BufReader::new(stdin.lock());
                interp
                    .run(reader, "-")
                    .map_err(|e| format!("実行エラー: {}", e))?;
            }
        } else {
            for operand in &operands {
                if interp.has_exited() {
                    break;
                }
                if let Some((name, value)) = parse_assignment_operand(operand) {
                    // Assignment operand: takes effect at this point
                    // in the file sequence
                    interp.set_var(&name, &value);
                } else if operand == "-" {
                    let stdin = io::stdin();
                    let reader = BufReader::new(stdin.lock());
                    interp
                        .run(reader, "-")
                        .map_err(|e| format!("実行エラー: {}", e))?;
                } else {
                    let file = File::open(operand)
                        .map_err(|e| format!("'{}' を開けません: {}", operand, e))?;
                    let reader = BufReader::new(file);
                    interp
                        .run(reader, operand)
                        .map_err(|e| format!("実行エラー: {}", e))?;
                }
            }
        }
    }

    interp
        .run_end_rules()
        .map_err(|e| format!("実行エラー: {}", e))?;

    // Wait for pipe children and flush files before exiting
    interp.close_all_outputs();

    Ok(interp.exit_code())
}

/// A program consisting only of BEGIN rules never reads input;
/// anything else (main rules or END rules) consumes stdin.
fn program_reads_input(program: &ast::Program) -> bool {
    program
        .rules
        .iter()
        .any(|r| !matches!(r.pattern, Some(ast::Pattern::Begin)))
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("awk: {}", e);
            eprintln!("詳しくは 'awk --help' を参照してください。");
            process::exit(2);
        }
    };

    match run(config) {
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("awk: {}", e);
            process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Cursor;
    use std::rc::Rc;

    struct TestWriter(Rc<RefCell<Vec<u8>>>);

    impl std::io::Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn run_awk(program: &str, input: &str) -> String {
        run_awk_result(program, input).unwrap()
    }

    fn run_awk_result(program: &str, input: &str) -> Result<String, String> {
        let mut parser = Parser::new(program).unwrap();
        let prog = parser.parse().unwrap();

        let output = Rc::new(RefCell::new(Vec::new()));
        let writer = TestWriter(Rc::clone(&output));

        let run_result = {
            let mut interp = Interpreter::new(&prog, Box::new(writer));
            let reader = BufReader::new(Cursor::new(input));
            interp.run(reader, "test").map_err(|e| e.to_string())?;
            interp.run_end_rules().map_err(|e| e.to_string())
        };

        run_result?;

        let bytes = output.borrow().clone();
        Ok(String::from_utf8(bytes).unwrap())
    }

    fn run_awk_with_setup<F>(program: &str, input: &str, setup: F) -> String
    where
        F: FnOnce(&mut Interpreter<'_>),
    {
        let mut parser = Parser::new(program).unwrap();
        let prog = parser.parse().unwrap();

        let output = Rc::new(RefCell::new(Vec::new()));
        let writer = TestWriter(Rc::clone(&output));

        {
            let mut interp = Interpreter::new(&prog, Box::new(writer));
            setup(&mut interp);
            let reader = BufReader::new(Cursor::new(input));
            interp.run(reader, "test").unwrap();
            interp.run_end_rules().unwrap();
        }

        let bytes = output.borrow().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn test_expand_path_argument_without_meta_keeps_literal() {
        let expanded = expand_path_argument("plain.txt").unwrap();
        assert_eq!(expanded, vec!["plain.txt"]);
    }

    #[test]
    fn test_expand_path_argument_without_match_keeps_literal() {
        let expanded = expand_path_argument("definitely-no-match-*.txt").unwrap();
        assert_eq!(expanded, vec!["definitely-no-match-*.txt"]);
    }

    #[test]
    fn test_parse_args_supports_windows_style_glob_expansion() {
        let dir = std::env::temp_dir().join(format!(
            "awk-rs-glob-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let upper = dir.join("Alpha.TXT");
        let lower = dir.join("beta.txt");
        std::fs::write(&upper, "a").unwrap();
        std::fs::write(&lower, "b").unwrap();

        let pattern = dir.join("*.txt").to_string_lossy().into_owned();
        let config =
            parse_args_from(vec!["awk".to_string(), "{ print }".to_string(), pattern]).unwrap();

        assert_eq!(config.files.len(), 2);
        assert!(config.files.iter().any(|f| f.ends_with("Alpha.TXT")));
        assert!(config.files.iter().any(|f| f.ends_with("beta.txt")));

        std::fs::remove_file(upper).unwrap();
        std::fs::remove_file(lower).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn test_parse_args_supports_multiple_f_options() {
        let dir = std::env::temp_dir().join(format!(
            "awk-rs-prog-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let prog1 = dir.join("one.awk");
        let prog2 = dir.join("two.awk");
        std::fs::write(&prog1, "BEGIN { print \"one\" }\n").unwrap();
        std::fs::write(&prog2, "BEGIN { print \"two\" }\n").unwrap();

        let config = parse_args_from(vec![
            "awk".to_string(),
            "-f".to_string(),
            prog1.to_string_lossy().into_owned(),
            "-f".to_string(),
            prog2.to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert!(config.program.contains("print \"one\""));
        assert!(config.program.contains("print \"two\""));

        std::fs::remove_file(prog1).unwrap();
        std::fs::remove_file(prog2).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn test_rs_empty_paragraph_mode() {
        let result = run_awk_with_setup("{ print NR, $0 }", "a\nb\n\nc\n\n", |interp| {
            interp.set_var("RS", "");
        });
        assert_eq!(result, "1 a\nb\n2 c\n");
    }

    #[test]
    fn test_getline_from_current_input_updates_counters() {
        let result = run_awk(
            "{ print $0; getline nextline; print NR, nextline }",
            "a\nb\nc\n",
        );
        assert_eq!(result, "a\n2 b\nc\n3 b\n");
    }

    #[test]
    fn test_split_populates_array() {
        let result = run_awk(
            "BEGIN { n = split(\"a:b:c\", parts, \":\"); print n, parts[1], parts[2], parts[3] }",
            "",
        );
        assert_eq!(result, "3 a b c\n");
    }

    #[test]
    fn test_sub_and_gsub_update_target() {
        let result = run_awk(
            "BEGIN { s = \"foo foo\"; print sub(\"foo\", \"bar\", s), s; print gsub(\"o\", \"0\", s), s }",
            "",
        );
        assert_eq!(result, "1 bar foo\n2 bar f00\n");
    }

    #[test]
    fn test_sub_with_regex_literal_pattern() {
        // sub/gsub の第1引数が `/.../` 形式の regex リテラルの場合、
        // パターンソースをそのまま正規表現として使うこと（POSIX `$0` への暗黙マッチ評価を抑止）。
        let result = run_awk(
            "{ r = sub(/abc/, \"X\"); print \"ret:\", r, \"line:\", $0 }",
            "abcdef\n",
        );
        assert_eq!(result, "ret: 1 line: Xdef\n");
    }

    #[test]
    fn test_gsub_with_regex_literal_pattern() {
        let result = run_awk(
            "BEGIN { s = \"a-b-c\"; n = gsub(/-/, \"_\", s); print n, s }",
            "",
        );
        assert_eq!(result, "2 a_b_c\n");
    }

    #[test]
    fn test_match_with_regex_literal_pattern() {
        let result = run_awk(
            "BEGIN { print match(\"hello world\", /o w/), RSTART, RLENGTH }",
            "",
        );
        assert_eq!(result, "5 5 3\n");
    }

    #[test]
    fn test_split_with_regex_literal_separator() {
        let result = run_awk(
            "BEGIN { n = split(\"1,2;3,4\", a, /[,;]/); for (i=1;i<=n;i++) print a[i] }",
            "",
        );
        assert_eq!(result, "1\n2\n3\n4\n");
    }

    #[test]
    fn test_sub_replacement_honors_escaped_ampersand() {
        let result = run_awk(
            "BEGIN { s = \"abc\"; sub(\"b\", \"\\\\&-&\", s); print s }",
            "",
        );
        assert_eq!(result, "a&-bc\n");
    }

    #[test]
    fn test_nextfile_skips_rest_of_current_file() {
        let mut parser = Parser::new("{ print FILENAME, FNR, $0; nextfile }").unwrap();
        let prog = parser.parse().unwrap();
        let output = Rc::new(RefCell::new(Vec::new()));
        let writer = TestWriter(Rc::clone(&output));

        {
            let mut interp = Interpreter::new(&prog, Box::new(writer));
            interp
                .run(BufReader::new(Cursor::new("a1\na2\n")), "first")
                .unwrap();
            interp
                .run(BufReader::new(Cursor::new("b1\nb2\n")), "second")
                .unwrap();
            interp.run_end_rules().unwrap();
        }

        let bytes = output.borrow().clone();
        let result = String::from_utf8(bytes).unwrap();
        assert_eq!(result, "first 1 a1\nsecond 1 b1\n");
    }

    #[test]
    fn test_fflush_returns_success() {
        let result = run_awk("BEGIN { print fflush() }", "");
        assert_eq!(result, "0\n");
    }

    #[test]
    fn test_system_returns_exit_status() {
        let result = run_awk("BEGIN { print system(\"exit 7\") }", "");
        assert_eq!(result, "7\n");
    }

    #[test]
    fn test_match_prefers_posix_leftmost_longest_for_top_level_alternation() {
        let result = run_awk(
            "BEGIN { print match(\"ab\", \"a|ab\"), RSTART, RLENGTH }",
            "",
        );
        assert_eq!(result, "1 1 2\n");
    }

    #[test]
    fn test_regex_operator_prefers_posix_leftmost_longest_for_top_level_alternation() {
        let result = run_awk("BEGIN { print \"ab\" ~ /a|ab/ }", "");
        assert_eq!(result, "1\n");
    }

    #[test]
    fn test_nested_alternation_prefers_posix_leftmost_longest() {
        let result = run_awk(
            "BEGIN { print match(\"abc\", \"(a|ab)c\"), RSTART, RLENGTH }",
            "",
        );
        assert_eq!(result, "1 1 3\n");
    }

    #[test]
    fn test_posix_character_class_uses_c_locale_behavior() {
        let result = run_awk("BEGIN { print match(\"123ABC\", \"[[:alpha:]]+\") }", "");
        assert_eq!(result, "4\n");
    }

    #[test]
    fn test_split_with_regex_field_separator() {
        let result = run_awk(
            "BEGIN { n = split(\"abc123def\", parts, \"[[:digit:]]+\"); print n, parts[1], parts[2] }",
            "",
        );
        assert_eq!(result, "2 abc def\n");
    }

    #[test]
    fn test_fs_regex_uses_posix_engine() {
        let result = run_awk_with_setup("{ print NF, $1, $2 }", "abc123def\n", |interp| {
            interp.set_var("FS", "[[:digit:]]+");
        });
        assert_eq!(result, "2 abc def\n");
    }

    #[test]
    fn test_gsub_empty_match_progresses_safely() {
        let result = run_awk("BEGIN { s = \"ab\"; print gsub(\"b*\", \"X\", s), s }", "");
        assert_eq!(result, "3 XaXX\n");
    }

    #[test]
    fn test_counted_repetition_works_in_runtime_match() {
        let result = run_awk(
            "BEGIN { print match(\"aaab\", \"a{2,3}b\"), RSTART, RLENGTH }",
            "",
        );
        assert_eq!(result, "1 1 4\n");
    }

    #[test]
    fn test_runtime_regex_treats_mid_pattern_caret_as_literal() {
        let result = run_awk("BEGIN { print \"a^b\" ~ /a^b/ }", "");
        assert_eq!(result, "1\n");
    }

    #[test]
    fn test_runtime_regex_rejects_collating_symbols() {
        let err = run_awk_result("BEGIN { print match(\"ch\", \"[[.ch.]]\") }", "").unwrap_err();
        assert!(err.contains("collating symbols"));
    }

    #[test]
    fn test_runtime_regex_rejects_invalid_repeat_range() {
        let err = run_awk_result("BEGIN { print match(\"aaa\", \"a{3,2}\") }", "").unwrap_err();
        assert!(err.contains("invalid repeat range"));
    }

    #[test]
    #[ignore = "stress test"]
    fn stress_test_nested_alternation_on_long_input() {
        let input = format!("{}b", "a".repeat(400));
        let program = format!(
            "BEGIN {{ print match(\"{}\", \"(a|aa|aaa|aaaa|aaaaa)*b\"), RSTART, RLENGTH }}",
            input
        );
        let result = run_awk(&program, "");
        assert_eq!(result, format!("1 1 {}\n", input.len()));
    }

    #[test]
    #[ignore = "stress test"]
    fn stress_test_large_gsub_with_empty_match_progress() {
        let source = "ab".repeat(200);
        let program = format!(
            "BEGIN {{ s = \"{}\"; print gsub(\"b*\", \"X\", s), length(s) }}",
            source
        );
        let result = run_awk(&program, "");
        assert!(result.starts_with("401 "));
    }

    #[test]
    fn test_argc_argv_and_environ_are_available() {
        let result = run_awk_with_setup(
            "BEGIN { print ARGC, ARGV[0], ARGV[1], ENVIRON[\"AWK_RS_TEST\"] }",
            "",
            |interp| {
                let argv = vec!["awk".to_string(), "input.txt".to_string()];
                interp.set_argv(&argv);
                interp.set_environ(vec![("AWK_RS_TEST".to_string(), "ok".to_string())]);
            },
        );
        assert_eq!(result, "2 awk input.txt ok\n");
    }

    #[test]
    fn test_begin_can_rewrite_input_file_list() {
        let mut parser = Parser::new("BEGIN { ARGC = 2; ARGV[1] = \"second.txt\" }").unwrap();
        let prog = parser.parse().unwrap();
        let output = Rc::new(RefCell::new(Vec::new()));
        let writer = TestWriter(Rc::clone(&output));
        let mut interp = Interpreter::new(&prog, Box::new(writer));
        interp.set_argv(&["awk".to_string(), "first.txt".to_string()]);

        interp.run_begin_rules().unwrap();

        assert_eq!(interp.input_file_args(), vec!["second.txt"]);
    }

    #[test]
    fn test_print_all() {
        let result = run_awk("{ print }", "hello\nworld\n");
        assert_eq!(result, "hello\nworld\n");
    }

    #[test]
    fn test_print_field() {
        let result = run_awk("{ print $1 }", "hello world\nfoo bar\n");
        assert_eq!(result, "hello\nfoo\n");
    }

    #[test]
    fn test_begin_end() {
        let result = run_awk("BEGIN { print \"start\" } END { print \"end\" }", "");
        assert_eq!(result, "start\nend\n");
    }

    #[test]
    fn test_pattern() {
        let result = run_awk("/hello/ { print }", "hello\nworld\nhello again\n");
        assert_eq!(result, "hello\nhello again\n");
    }

    #[test]
    fn test_nr() {
        let result = run_awk("{ print NR, $0 }", "a\nb\nc\n");
        assert_eq!(result, "1 a\n2 b\n3 c\n");
    }

    #[test]
    fn test_sum() {
        let result = run_awk("{ sum += $1 } END { print sum }", "1\n2\n3\n4\n5\n");
        assert_eq!(result, "15\n");
    }

    #[test]
    fn test_if() {
        let result = run_awk("{ if ($1 > 2) print $1 }", "1\n2\n3\n4\n5\n");
        assert_eq!(result, "3\n4\n5\n");
    }

    #[test]
    fn test_for_loop() {
        let result = run_awk("BEGIN { for (i = 1; i <= 3; i++) print i }", "");
        assert_eq!(result, "1\n2\n3\n");
    }

    #[test]
    fn test_array() {
        let result = run_awk(
            "{ a[$1] = $2 } END { for (k in a) print k, a[k] }",
            "x 1\ny 2\n",
        );
        assert!(result.contains("x 1"));
        assert!(result.contains("y 2"));
    }

    #[test]
    fn test_printf() {
        let result = run_awk("BEGIN { printf \"%s=%d\\n\", \"x\", 42 }", "");
        assert_eq!(result, "x=42\n");
    }

    #[test]
    fn test_function() {
        let result = run_awk(
            "function double(x) { return x * 2 } BEGIN { print double(21) }",
            "",
        );
        assert_eq!(result, "42\n");
    }

    #[test]
    fn test_length() {
        let result = run_awk("{ print length($0) }", "hello\nhi\n");
        assert_eq!(result, "5\n2\n");
    }

    #[test]
    fn test_substr() {
        let result = run_awk("BEGIN { print substr(\"hello\", 2, 3) }", "");
        assert_eq!(result, "ell\n");
    }

    #[test]
    fn test_toupper_tolower() {
        let result = run_awk("BEGIN { print toupper(\"hello\"), tolower(\"WORLD\") }", "");
        assert_eq!(result, "HELLO world\n");
    }

    #[test]
    fn test_math() {
        let result = run_awk("BEGIN { print int(3.7), sqrt(16) }", "");
        assert_eq!(result, "3 4\n");
    }

    #[test]
    fn test_comparison() {
        let result = run_awk("$1 > 5 { print }", "3\n7\n2\n9\n");
        assert_eq!(result, "7\n9\n");
    }

    #[test]
    fn test_regex_match() {
        let result = run_awk(
            "$0 ~ /^[0-9]+$/ { print \"number:\", $0 }",
            "123\nabc\n456\n",
        );
        assert_eq!(result, "number: 123\nnumber: 456\n");
    }

    #[test]
    fn test_increment() {
        let result = run_awk("BEGIN { x = 5; print x++, ++x }", "");
        assert_eq!(result, "5 7\n");
    }

    #[test]
    fn test_string_concat() {
        let result = run_awk("BEGIN { a = \"hello\"; b = \"world\"; print a b }", "");
        assert_eq!(result, "helloworld\n");
    }

    #[test]
    fn test_ors_is_honored() {
        let result = run_awk("BEGIN{ORS=\"|\"}{print}", "a\nb\n");
        assert_eq!(result, "a|b|");
    }

    #[test]
    fn test_if_semicolon_before_else() {
        let result = run_awk("BEGIN{if (0) print \"a\"; else print \"b\"}", "");
        assert_eq!(result, "b\n");
    }

    #[test]
    fn test_exit_in_begin_still_runs_end() {
        let result = run_awk("BEGIN{exit 0} END{print \"end\"}", "");
        assert_eq!(result, "end\n");
    }

    #[test]
    fn test_exit_in_main_runs_end_once() {
        let result = run_awk("NR==1{exit} {print \"no\"} END{print \"end\"}", "a\nb\n");
        assert_eq!(result, "end\n");
    }

    #[test]
    fn test_array_passed_by_reference() {
        let result = run_awk(
            "function f(arr) {arr[\"k\"]=\"v\"} BEGIN{f(a); print a[\"k\"]}",
            "",
        );
        assert_eq!(result, "v\n");
    }

    #[test]
    fn test_local_array_parameter_supports_recursion() {
        let result = run_awk(
            "function f(n,  tmp) { split(\"x y\", tmp); if (n > 0) f(n - 1); return tmp[1] } \
             BEGIN { print f(2) }",
            "",
        );
        assert_eq!(result, "x\n");
    }

    #[test]
    fn test_space_before_paren_is_concatenation() {
        let result = run_awk(
            "function f(a, b) {return a (b==\"\" ? \"-\" : b)} BEGIN{print f(\"x\")}",
            "",
        );
        assert_eq!(result, "x-\n");
    }

    #[test]
    fn test_length_of_array_and_bare_length() {
        let result = run_awk("BEGIN{a[1]=1;a[2]=2;print length(a)}", "");
        assert_eq!(result, "2\n");
        let result = run_awk("{print length}", "hello\n");
        assert_eq!(result, "5\n");
    }

    #[test]
    fn test_delete_whole_array() {
        let result = run_awk("BEGIN{a[1]=1; a[2]=2; delete a; print length(a)}", "");
        assert_eq!(result, "0\n");
    }

    #[test]
    fn test_multidim_in_operator() {
        let result = run_awk("BEGIN{a[1,2]=3; print ((1,2) in a), ((9,9) in a)}", "");
        assert_eq!(result, "1 0\n");
    }

    #[test]
    fn test_unary_minus_binds_looser_than_power() {
        let result = run_awk("BEGIN{print -2^2, 2^-1}", "");
        assert_eq!(result, "-4 0.5\n");
    }

    #[test]
    fn test_unary_plus_coerces_to_number() {
        let result = run_awk("BEGIN{x=\"3abc\"; print +x}", "");
        assert_eq!(result, "3\n");
    }

    #[test]
    fn test_print_assignment_expression() {
        let result = run_awk("BEGIN{print x = 5; print x}", "");
        assert_eq!(result, "5\n5\n");
    }

    #[test]
    fn test_printf_parenthesized_args() {
        let result = run_awk("BEGIN{printf(\"%d-%s\\n\", 7, \"y\")}", "");
        assert_eq!(result, "7-y\n");
    }

    #[test]
    fn test_getline_from_missing_file_returns_minus_one() {
        let result = run_awk(
            "BEGIN{r = (getline x < \"definitely_missing_file_xyz\"); print r}",
            "",
        );
        assert_eq!(result, "-1\n");
    }

    #[test]
    fn test_getline_var_from_file_does_not_touch_nr() {
        let dir = std::env::temp_dir().join(format!(
            "awk-rs-getline-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("data.txt");
        std::fs::write(&data, "zzz\n").unwrap();
        let path = data.to_string_lossy().replace('\\', "/");
        let prog = format!("{{getline line < \"{}\"; print NR, line}}", path);
        let result = run_awk(&prog, "a\n");
        assert_eq!(result, "1 zzz\n");
        std::fs::remove_file(data).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn test_convfmt_applies_to_concatenation() {
        let result = run_awk("BEGIN{CONVFMT=\"%.2g\"; x=3.14159; print x \"\"}", "");
        assert_eq!(result, "3.1\n");
    }

    #[test]
    fn test_ofmt_not_applied_to_integers() {
        let result = run_awk("BEGIN{OFMT=\"%.2f\"; print 3.14159, 42}", "");
        assert_eq!(result, "3.14 42\n");
    }

    #[test]
    fn test_division_by_zero_is_error() {
        let err = run_awk_result("BEGIN{print 1/0}", "").unwrap_err();
        assert!(err.contains("division by zero"));
    }

    #[test]
    fn test_negative_field_assignment_is_error() {
        let err = run_awk_result("{$(-1)=\"x\"}", "a\n").unwrap_err();
        assert!(err.contains("field -1"));
    }

    #[test]
    fn test_multichar_rs_is_regex() {
        let result = run_awk_with_setup("{print NR, $0}", "a1b22c", |interp| {
            interp.set_var("RS", "[0-9]+");
        });
        assert_eq!(result, "1 a\n2 b\n3 c\n");
    }

    #[test]
    fn test_paragraph_mode_newline_is_field_separator() {
        let result = run_awk_with_setup("{print $2}", "x:1\ny:2\n\n", |interp| {
            interp.set_var("RS", "");
            interp.set_var("FS", ":");
        });
        assert_eq!(result, "1\n");
    }

    #[test]
    fn test_numeric_string_comparison() {
        let result = run_awk("{print ($1 == 10)}", "10.0\n");
        assert_eq!(result, "1\n");
        let result = run_awk("BEGIN{print (\"10\" == \"10.0\")}", "");
        assert_eq!(result, "0\n");
    }

    #[test]
    fn test_uninitialized_compares_equal_to_zero_and_empty() {
        let result = run_awk("BEGIN{print (x == 0), (x == \"\")}", "");
        assert_eq!(result, "1 1\n");
    }

    #[test]
    fn test_substr_clamps_out_of_range_start() {
        let result = run_awk(
            "BEGIN{print substr(\"hello\", 0, 2) \"|\" substr(\"hello\", -1, 3) \"|\" substr(\"hello\", 10)}",
            "",
        );
        assert_eq!(result, "h|h|\n");
    }

    #[test]
    fn test_exit_inside_function() {
        let result = run_awk(
            "function die() { exit 3 } NR==1 { die(); print \"no\" } END { print \"end\" }",
            "a\nb\n",
        );
        assert_eq!(result, "end\n");
    }

    #[test]
    fn test_func_keyword_alias() {
        let result = run_awk("func f() {return 9} BEGIN{print f()}", "");
        assert_eq!(result, "9\n");
    }

    #[test]
    fn test_getline_into_array_element() {
        let result = run_awk("NR==1{getline a[\"x\"]; print a[\"x\"]}", "one\ntwo\n");
        assert_eq!(result, "two\n");
    }

    #[test]
    fn test_ternary() {
        let result = run_awk(
            "{ print ($1 > 0 ? \"positive\" : \"non-positive\") }",
            "5\n-3\n0\n",
        );
        assert_eq!(result, "positive\nnon-positive\nnon-positive\n");
    }
}
