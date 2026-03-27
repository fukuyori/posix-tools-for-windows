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
    eprintln!("awk-rs 0.1.0");
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
    let argv0 = args.first().cloned().unwrap_or_else(|| "awk".to_string());
    let mut config = Config {
        program: String::new(),
        files: Vec::new(),
        awk_argv: vec![argv0],
        field_separator: None,
        variables: Vec::new(),
    };

    let mut i = 1;
    let mut program_set = false;

    while i < args.len() {
        let arg = &args[i];

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
            config.field_separator = Some(args[i].clone());
        } else if arg.starts_with("-F") {
            config.field_separator = Some(arg[2..].to_string());
        } else if arg == "-v" {
            i += 1;
            if i >= args.len() {
                return Err("-v には引数が必要です".to_string());
            }
            let assignment = &args[i];
            if let Some(eq_pos) = assignment.find('=') {
                let name = assignment[..eq_pos].to_string();
                let value = assignment[eq_pos + 1..].to_string();
                config.variables.push((name, value));
            } else {
                return Err(format!("無効な変数代入: {}", assignment));
            }
        } else if arg.starts_with("-v") {
            let assignment = &arg[2..];
            if let Some(eq_pos) = assignment.find('=') {
                let name = assignment[..eq_pos].to_string();
                let value = assignment[eq_pos + 1..].to_string();
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
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("不明なオプション: {}", arg));
        } else if !program_set {
            config.program = arg.clone();
            program_set = true;
        } else {
            let expanded = expand_path_argument(arg)
                .map_err(|e| format!("入力ファイル '{}' を展開できません: {}", arg, e))?;
            config.files.extend(expanded);
        }

        i += 1;
    }

    config.awk_argv.extend(config.files.iter().cloned());

    if !program_set {
        return Err("プログラムが指定されていません".to_string());
    }

    Ok(config)
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

    if let Some(code) = interp
        .run_begin_rules()
        .map_err(|e| format!("実行エラー: {}", e))?
    {
        return Ok(code);
    }

    let files = interp.input_file_args();
    if files.is_empty() {
        let stdin = io::stdin();
        let reader = BufReader::new(stdin.lock());
        interp
            .run(reader, "-")
            .map_err(|e| format!("実行エラー: {}", e))?;
    } else {
        for filename in &files {
            if filename == "-" {
                let stdin = io::stdin();
                let reader = BufReader::new(stdin.lock());
                interp
                    .run(reader, "-")
                    .map_err(|e| format!("実行エラー: {}", e))?;
            } else {
                let file = File::open(filename)
                    .map_err(|e| format!("'{}' を開けません: {}", filename, e))?;
                let reader = BufReader::new(file);
                interp
                    .run(reader, filename)
                    .map_err(|e| format!("実行エラー: {}", e))?;
            }
        }
    }

    interp
        .run_end_rules()
        .map_err(|e| format!("実行エラー: {}", e))?;

    Ok(interp.exit_code())
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
    fn test_ternary() {
        let result = run_awk(
            "{ print ($1 > 0 ? \"positive\" : \"non-positive\") }",
            "5\n-3\n0\n",
        );
        assert_eq!(result, "positive\nnon-positive\nnon-positive\n");
    }
}
