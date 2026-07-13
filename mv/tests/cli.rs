// mv の統合テスト
// cargo test で実行。ビルド済みバイナリを一時ディレクトリ上で動かして検証する。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mv_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mv")
}

/// テストごとに独立した作業ディレクトリを作る
fn setup(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("mv_cli_tests")
        .join(format!("{}_{}", name, std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_mv(cwd: &Path, args: &[&str]) -> Output {
    Command::new(mv_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("mv の実行に失敗")
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

#[test]
fn basic_rename() {
    let d = setup("basic_rename");
    write(&d, "a.txt", "hello");
    let out = run_mv(&d, &["a.txt", "b.txt"]);
    assert!(out.status.success());
    assert!(!d.join("a.txt").exists());
    assert_eq!(fs::read_to_string(d.join("b.txt")).unwrap(), "hello");
}

#[test]
fn move_into_directory() {
    let d = setup("move_into_directory");
    write(&d, "a.txt", "x");
    fs::create_dir(d.join("sub")).unwrap();
    let out = run_mv(&d, &["a.txt", "sub"]);
    assert!(out.status.success());
    assert!(d.join("sub").join("a.txt").exists());
}

#[test]
fn refuses_to_overwrite_nonempty_directory() {
    let d = setup("nonempty_dir");
    fs::create_dir(d.join("src")).unwrap();
    write(&d.join("src"), "f.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();
    write(&d.join("dest"), "important.txt", "keep me");

    let out = run_mv(&d, &["-T", "src", "dest"]);
    assert!(!out.status.success(), "非空ディレクトリの上書きは失敗すべき");
    // 既存データが失われていないこと
    assert_eq!(
        fs::read_to_string(d.join("dest").join("important.txt")).unwrap(),
        "keep me"
    );
    assert!(d.join("src").exists());
}

#[test]
fn overwrites_empty_directory() {
    let d = setup("empty_dir");
    fs::create_dir(d.join("src")).unwrap();
    write(&d.join("src"), "f.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();

    let out = run_mv(&d, &["-T", "src", "dest"]);
    assert!(out.status.success());
    assert!(d.join("dest").join("f.txt").exists());
    assert!(!d.join("src").exists());
}

#[test]
fn refuses_file_over_directory() {
    let d = setup("file_over_dir");
    write(&d, "plain.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();
    write(&d.join("dest"), "keep.txt", "keep");

    let out = run_mv(&d, &["-T", "plain.txt", "dest"]);
    assert!(!out.status.success());
    assert!(d.join("dest").join("keep.txt").exists());
    assert!(d.join("plain.txt").exists());
}

#[test]
fn refuses_directory_over_file() {
    let d = setup("dir_over_file");
    fs::create_dir(d.join("srcdir")).unwrap();
    write(&d, "plain.txt", "x");

    let out = run_mv(&d, &["-T", "srcdir", "plain.txt"]);
    assert!(!out.status.success());
    assert!(d.join("plain.txt").exists());
    assert!(d.join("srcdir").exists());
}

#[test]
fn verbose_goes_to_stdout() {
    let d = setup("verbose_stdout");
    write(&d, "a.txt", "x");
    let out = run_mv(&d, &["-v", "a.txt", "b.txt"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("'a.txt' -> 'b.txt'"), "stdout: {}", stdout);
}

#[test]
fn no_clobber_keeps_destination() {
    let d = setup("no_clobber");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = run_mv(&d, &["-n", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert!(d.join("src.txt").exists());
    assert_eq!(fs::read_to_string(d.join("dest.txt")).unwrap(), "old");
}

#[test]
fn backup_simple() {
    let d = setup("backup_simple");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = run_mv(&d, &["-b", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(d.join("dest.txt~")).unwrap(), "old");
}

#[test]
fn backup_numbered() {
    let d = setup("backup_numbered");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = run_mv(&d, &["--backup=numbered", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt.~1~")).unwrap(), "old");
}

#[test]
fn backup_suffix_env() {
    let d = setup("backup_suffix_env");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = Command::new(mv_bin())
        .args(["-b", "src.txt", "dest.txt"])
        .env("SIMPLE_BACKUP_SUFFIX", ".bak")
        .current_dir(&d)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt.bak")).unwrap(), "old");
}

#[test]
fn same_file_is_error() {
    let d = setup("same_file");
    write(&d, "a.txt", "x");
    let out = run_mv(&d, &["a.txt", "a.txt"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("同じファイル"), "stderr: {}", stderr);
}

#[cfg(windows)]
#[test]
fn case_only_rename_file() {
    let d = setup("case_rename_file");
    write(&d, "case.txt", "x");
    let out = run_mv(&d, &["case.txt", "CASE.TXT"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let names: Vec<String> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&"CASE.TXT".to_string()), "files: {:?}", names);
}

#[cfg(windows)]
#[test]
fn case_only_rename_directory() {
    let d = setup("case_rename_dir");
    fs::create_dir(d.join("mydir")).unwrap();
    write(&d.join("mydir"), "f.txt", "x");
    let out = run_mv(&d, &["mydir", "MYDIR"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let names: Vec<String> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&"MYDIR".to_string()), "files: {:?}", names);
}

#[test]
fn move_into_own_subdirectory_is_error() {
    let d = setup("own_subdir");
    fs::create_dir(d.join("pd")).unwrap();
    let out = run_mv(&d, &["pd", "pd/sub"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("自分自身のサブディレクトリ"), "stderr: {}", stderr);
    assert!(d.join("pd").exists());
}

#[test]
fn target_dir_and_no_target_dir_conflict() {
    let d = setup("t_and_big_t");
    write(&d, "a.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_mv(&d, &["-T", "-t", "dest", "a.txt"]);
    assert!(!out.status.success());
}

#[test]
fn no_target_dir_extra_operand() {
    let d = setup("extra_operand");
    write(&d, "a.txt", "x");
    write(&d, "b.txt", "y");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_mv(&d, &["-T", "a.txt", "b.txt", "dest"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("余分なオペランド"), "stderr: {}", stderr);
}

#[test]
fn target_dir_with_glob() {
    let d = setup("t_glob");
    write(&d, "g1.txt", "1");
    write(&d, "g2.txt", "2");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_mv(&d, &["-t", "dest", "g*.txt"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(d.join("dest").join("g1.txt").exists());
    assert!(d.join("dest").join("g2.txt").exists());
}

#[test]
fn update_skips_older_source() {
    let d = setup("update_skip");
    let src = write(&d, "src.txt", "older");
    let dest = write(&d, "dest.txt", "newer");
    // src を確実に古くする
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    let f = fs::OpenOptions::new().write(true).open(&src).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(old)).unwrap();
    drop(f);

    let out = run_mv(&d, &["-u", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert!(src.exists());
    assert_eq!(fs::read_to_string(&dest).unwrap(), "newer");
}

#[test]
fn multiple_sources_partial_failure() {
    let d = setup("partial_failure");
    write(&d, "m1.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_mv(&d, &["m1.txt", "missing.txt", "dest"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(d.join("dest").join("m1.txt").exists(), "エラー後も他のファイルは移動される");
}

#[test]
fn missing_operand_errors() {
    let d = setup("missing_operand");
    let out = run_mv(&d, &[]);
    assert_eq!(out.status.code(), Some(1));
    let out = run_mv(&d, &["only_one"]);
    assert_eq!(out.status.code(), Some(1));
}
