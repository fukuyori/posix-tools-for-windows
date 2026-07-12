mod action;
mod expression;
mod messages;
mod parser;
mod platform;
mod walker;

use std::env;

use parser::Parser;
use walker::Walker;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-help") {
        println!("{}", messages::HELP_MESSAGE);
        return;
    }

    if args.iter().any(|a| a == "--version" || a == "-version") {
        println!("find 1.1.0");
        return;
    }

    // Windows: Job Object を作成し自プロセスを登録する。
    // これにより Ctrl-C 等で find が終了した際、-exec で起動した子プロセスも
    // OS によって自動的に kill される（孤児プロセス防止）。
    // 失敗しても動作は続ける（デバッガ下など既存 Job に属している場合は非致命的）。
    let _job = platform::job::JobObject::create_and_assign_self().ok();

    let mut parser = Parser::new(args);
    let parse_result = match parser.parse() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{}", e);
            eprintln!("詳しくは 'find --help' を参照してください。");
            std::process::exit(1);
        }
    };

    let mut walker = Walker::new(parse_result.global_options, parse_result.expression);
    walker.walk(&parse_result.paths);

    std::process::exit(walker.exit_code);
}

#[cfg(test)]
mod tests {
    use crate::expression::*;
    use crate::parser::Parser;
    use crate::parser::{FollowSymlinks, GlobalOptions};
    use crate::walker::{should_follow_metadata, Walker};

    #[test]
    fn test_parse_name() {
        let args = vec!["-name".to_string(), "*.txt".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_parse_type() {
        let args = vec!["-type".to_string(), "f".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_parse_complex_expression() {
        let args = vec![
            "(".to_string(),
            "-name".to_string(),
            "*.txt".to_string(),
            "-o".to_string(),
            "-name".to_string(),
            "*.rs".to_string(),
            ")".to_string(),
            "-type".to_string(),
            "f".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_numeric_comparison() {
        assert_eq!(
            NumericComparison::parse("+5"),
            Some(NumericComparison::GreaterThan(5))
        );
        assert_eq!(
            NumericComparison::parse("-3"),
            Some(NumericComparison::LessThan(3))
        );
        assert_eq!(
            NumericComparison::parse("10"),
            Some(NumericComparison::Exactly(10))
        );
    }

    #[test]
    fn test_size_comparison() {
        let size = SizeComparison::parse("+1M").unwrap();
        assert!(matches!(size.comparison, NumericComparison::GreaterThan(1)));
        assert_eq!(size.unit, SizeUnit::Mebi);

        let size = SizeComparison::parse("100c").unwrap();
        assert!(matches!(size.comparison, NumericComparison::Exactly(100)));
        assert_eq!(size.unit, SizeUnit::Bytes);
    }

    #[test]
    fn test_perm_mode() {
        let perm = PermMode::parse("644").unwrap();
        assert!(matches!(perm, PermMode::Exact(0o644)));

        let perm = PermMode::parse("-755").unwrap();
        assert!(matches!(perm, PermMode::All(0o755)));
    }

    #[test]
    fn test_perm_mode_symbolic_exact_respects_operator_semantics() {
        assert_eq!(PermMode::parse("u=r"), Some(PermMode::Exact(0o400)));
        assert_eq!(PermMode::parse("u+w"), Some(PermMode::Exact(0o200)));
        assert_eq!(PermMode::parse("u-w"), Some(PermMode::Exact(0)));
        assert_eq!(PermMode::parse("u=rw,g=u"), Some(PermMode::Exact(0o660)));
        assert_eq!(PermMode::parse("u=X"), Some(PermMode::Exact(0)));
        assert_eq!(PermMode::parse("u+x,g+X"), Some(PermMode::Exact(0o110)));
    }

    #[test]
    fn test_file_type() {
        assert_eq!(FileType::from_char('f'), Some(FileType::RegularFile));
        assert_eq!(FileType::from_char('d'), Some(FileType::Directory));
        assert_eq!(FileType::from_char('l'), Some(FileType::SymbolicLink));
        assert_eq!(FileType::from_char('x'), None);
    }

    #[test]
    fn test_parse_rejects_trailing_tokens() {
        let args = vec![
            "-name".to_string(),
            "*.txt".to_string(),
            "unexpected".to_string(),
        ];
        let mut parser = Parser::new(args);
        assert!(parser.parse().is_err());
    }

    #[test]
    fn test_parse_rejects_unmatched_closing_paren() {
        let args = vec!["-name".to_string(), "*.txt".to_string(), ")".to_string()];
        let mut parser = Parser::new(args);
        assert!(parser.parse().is_err());
    }

    #[test]
    fn test_default_print_remains_enabled_for_prune_only() {
        let args = vec!["-prune".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        let walker = Walker::new(result.global_options, result.expression);
        assert!(walker.use_default_print);
    }

    #[test]
    fn test_default_print_disabled_for_explicit_print_action() {
        let args = vec!["-print".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        let walker = Walker::new(result.global_options, result.expression);
        assert!(!walker.use_default_print);
    }

    #[test]
    fn test_parse_accepts_escaped_parentheses_and_not() {
        let args = vec![
            "\\(".to_string(),
            "\\!".to_string(),
            "-name".to_string(),
            "*.txt".to_string(),
            "\\)".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_parse_accepts_escaped_exec_terminator() {
        let args = vec![
            "-exec".to_string(),
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "{}".to_string(),
            "\\;".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_default_print_disabled_for_quit_action() {
        let args = vec!["-quit".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        let walker = Walker::new(result.global_options, result.expression);
        assert!(!walker.use_default_print);
    }

    #[test]
    fn test_parse_multi_type() {
        let args = vec!["-type".to_string(), "f,d".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        match result.expression.unwrap() {
            Expression::Or(a, b) => {
                assert!(matches!(*a, Expression::Test(Test::Type(FileType::RegularFile))));
                assert!(matches!(*b, Expression::Test(Test::Type(FileType::Directory))));
            }
            other => panic!("expected Or expression, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_rejects_invalid_multi_type() {
        let args = vec!["-type".to_string(), "f,".to_string()];
        let mut parser = Parser::new(args);
        assert!(parser.parse().is_err());
    }

    #[test]
    fn test_parse_options_inside_expression() {
        // GNU find と同様、-maxdepth などがテストの後に来ても受理する
        let args = vec![
            "-name".to_string(),
            "*.txt".to_string(),
            "-maxdepth".to_string(),
            "2".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert_eq!(result.global_options.max_depth, Some(2));
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_parse_daystart_applies_to_following_time_tests() {
        let args = vec![
            "-mtime".to_string(),
            "0".to_string(),
            "-daystart".to_string(),
            "-atime".to_string(),
            "0".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();

        fn collect_daystarts(expr: &Expression, out: &mut Vec<bool>) {
            match expr {
                Expression::Test(Test::Time { daystart, .. }) => out.push(*daystart),
                Expression::Not(e) => collect_daystarts(e, out),
                Expression::And(a, b) | Expression::Or(a, b) | Expression::List(a, b) => {
                    collect_daystarts(a, out);
                    collect_daystarts(b, out);
                }
                _ => {}
            }
        }

        let mut daystarts = Vec::new();
        collect_daystarts(result.expression.as_ref().unwrap(), &mut daystarts);
        assert_eq!(daystarts, vec![false, true]);
    }

    #[test]
    fn test_parse_used_and_lname() {
        let args = vec![
            "-used".to_string(),
            "+2".to_string(),
            "-lname".to_string(),
            "*target*".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
    }

    #[test]
    fn test_parse_fprintf() {
        let args = vec![
            "-fprintf".to_string(),
            "out.txt".to_string(),
            "%p\\n".to_string(),
        ];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        assert!(result.expression.is_some());
        // -fprintf はアクションなのでデフォルト print は無効
        let walker = Walker::new(result.global_options, result.expression);
        assert!(!walker.use_default_print);
    }

    #[test]
    fn test_convert_regex_basic_syntax() {
        use crate::parser::{convert_regex_syntax, RegexSyntax};

        // BRE: \( \) がグループ、裸の ( ) + ? はリテラル
        assert_eq!(
            convert_regex_syntax(r"\(ab\)\+", RegexSyntax::Basic),
            "(ab)+"
        );
        assert_eq!(convert_regex_syntax("a+b?", RegexSyntax::Basic), r"a\+b\?");
        assert_eq!(convert_regex_syntax("(x)", RegexSyntax::Basic), r"\(x\)");
        // Extended は無変換
        assert_eq!(
            convert_regex_syntax("(a|b)+", RegexSyntax::Extended),
            "(a|b)+"
        );
        // Emacs: \| が選択、+ は特殊のまま
        assert_eq!(
            convert_regex_syntax(r"a\|b+", RegexSyntax::Emacs),
            "a|b+"
        );
    }

    #[test]
    fn test_regex_matches_whole_path() {
        // GNU find と同様、-regex はパス全体にマッチする
        let args = vec!["-regex".to_string(), r".*\.txt".to_string()];
        let mut parser = Parser::new(args);
        let result = parser.parse().unwrap();
        let Expression::Test(Test::Regex { regex, .. }) = result.expression.unwrap() else {
            panic!("expected regex test");
        };
        assert!(regex.is_match("./foo/bar.txt"));
        // 部分一致では真にならない（"txt" の後に続きがある場合は不一致）
        assert!(!regex.is_match("./foo/bar.txt.bak"));
    }

    #[test]
    fn test_parse_anewer_missing_file_errors() {
        let args = vec!["-anewer".to_string(), "no_such_file_xyz".to_string()];
        let mut parser = Parser::new(args);
        assert!(parser.parse().is_err());
    }

    #[test]
    fn test_follow_symlink_commandline_mode_only_applies_to_start_paths() {
        assert!(should_follow_metadata(FollowSymlinks::Always, false));
        assert!(should_follow_metadata(FollowSymlinks::Always, true));
        assert!(should_follow_metadata(FollowSymlinks::Commandline, true));
        assert!(!should_follow_metadata(FollowSymlinks::Commandline, false));
        assert!(!should_follow_metadata(FollowSymlinks::Never, true));
        assert!(!should_follow_metadata(FollowSymlinks::Never, false));

        let walker = Walker::new(
            GlobalOptions {
                follow_symlinks: FollowSymlinks::Commandline,
                ..GlobalOptions::default()
            },
            None,
        );
        assert!(matches!(
            walker.options.follow_symlinks,
            FollowSymlinks::Commandline
        ));
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_start_path_glob_is_expanded() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("find_glob_test_{unique}"));
        fs::create_dir_all(base.join("alpha")).unwrap();
        fs::create_dir_all(base.join("beta")).unwrap();

        let pattern = format!("{}\\*", base.display());
        let mut parser = Parser::new(vec![pattern]);
        let result = parser.parse().unwrap();

        assert_eq!(result.paths.len(), 2);

        let _ = fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_start_path_glob_accepts_forward_slashes() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("find_glob_test_{unique}"));
        fs::create_dir_all(base.join("alpha")).unwrap();
        fs::create_dir_all(base.join("beta")).unwrap();

        let pattern = format!("{}/{}", base.display(), "*").replace('\\', "/");
        let mut parser = Parser::new(vec![pattern]);
        let result = parser.parse().unwrap();

        assert_eq!(result.paths.len(), 2);

        let _ = fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_path_glob_matches_with_posix_separator() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("find_path_test_{unique}"));
        let file_path = base.join("nested").join("sample.txt");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, b"hello").unwrap();

        let path_pattern = format!(
            "{}/nested/*.txt",
            base.display().to_string().replace('\\', "/")
        );

        let expr = match Parser::new(vec!["-path".to_string(), path_pattern])
            .parse()
            .unwrap()
            .expression
            .unwrap()
        {
            Expression::Test(test) => test,
            other => panic!("expected test expression, got {other:?}"),
        };

        let metadata = fs::symlink_metadata(&file_path).unwrap();
        let ctx = EvalContext::new(
            &file_path,
            &base,
            1,
            std::time::SystemTime::now(),
            metadata,
            None,
            false,
        );

        assert!(expr.evaluate(&ctx));

        let _ = fs::remove_dir_all(base);
    }
}
