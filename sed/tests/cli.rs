use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sed-cli-tests-{name}-{unique}"))
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run_sed(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sed"))
        .args(args)
        .output()
        .unwrap()
}

fn run_sed_in_dir(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sed"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_supports_basic_substitution() {
    let root = test_dir("basic-substitution");
    let input = root.join("input.txt");
    write_file(&input, "hello\n");

    let output = run_sed(&["s/hello/world/", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "world\n");
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_resolves_existing_paths_case_insensitively_on_windows() {
    let root = test_dir("case-insensitive-path");
    let input = root.join("MiXeD.TXT");
    write_file(&input, "value\n");

    let requested = root.join("mixed.txt");
    let output = run_sed(&["s/value/ok/", &requested.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ok\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_expands_globs_internally() {
    let root = test_dir("glob-expansion");
    write_file(&root.join("a.txt"), "alpha\n");
    write_file(&root.join("b.TXT"), "beta\n");
    write_file(&root.join(".hidden.txt"), "hidden\n");
    write_file(&root.join("nested").join("c.txt"), "nested\n");

    let pattern = root.join("*.txt");
    let output = run_sed(&["-n", "p", &pattern.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "alpha\nbeta\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_in_place_edit_accepts_arbitrary_backup_suffix() {
    let root = test_dir("in-place");
    let input = root.join("sample.txt");
    write_file(&input, "before\n");

    let output = run_sed(&["-i", "backup", "s/before/after/", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(fs::read_to_string(&input).unwrap(), "after\n");
    assert_eq!(fs::read_to_string(root.join("sample.txtbackup")).unwrap(), "before\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_reuses_last_regex_for_empty_patterns() {
    let root = test_dir("regex-reuse");
    let input = root.join("input.txt");
    write_file(&input, "foofoo\n");

    let output = run_sed(&["s/foo/X/;s//Y/", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "XY\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_branch_command_skips_following_commands() {
    let root = test_dir("branch");
    let input = root.join("input.txt");
    write_file(&input, "line\n");

    let output = run_sed(&["b done;s/line/NO/;:done;s/line/ok/", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ok\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_test_command_branches_after_successful_substitution() {
    let root = test_dir("test-branch");
    let input = root.join("input.txt");
    write_file(&input, "cat\nbird\n");

    let output = run_sed(&["s/cat/dog/;t done;s/.*/miss/;:done", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "dog\nmiss\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_test_not_command_branches_after_failed_substitution() {
    let root = test_dir("test-not-branch");
    let input = root.join("input.txt");
    write_file(&input, "cat\nbird\n");

    let output = run_sed(&["s/cat/dog/;T miss;b done;:miss;s/.*/miss/;:done", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "dog\nmiss\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_read_command_appends_file_contents() {
    let root = test_dir("read-file");
    let input = root.join("input.txt");
    let extra = root.join("extra.txt");
    write_file(&input, "first\n");
    write_file(&extra, "second\nthird\n");

    let output = run_sed_in_dir(&root, &["r extra.txt", &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "first\nsecond\nthird\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_write_command_writes_matched_lines_to_a_file() {
    let root = test_dir("write-file");
    let input = root.join("input.txt");
    let out = root.join("captured.txt");
    write_file(&input, "keep\nskip\n");

    let script = format!("/keep/w {}", out.to_string_lossy());
    let output = run_sed(&[&script, &input.to_string_lossy()]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "keep\nskip\n");
    assert_eq!(fs::read_to_string(&out).unwrap(), "keep\n");

    fs::remove_dir_all(root).unwrap();
}
