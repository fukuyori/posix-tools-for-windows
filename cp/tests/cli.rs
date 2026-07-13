// cp の統合テスト
// cargo test で実行。ビルド済みバイナリを一時ディレクトリ上で動かして検証する。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cp")
}

/// テストごとに独立した作業ディレクトリを作る
fn setup(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("cp_cli_tests")
        .join(format!("{}_{}", name, std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_cp(cwd: &Path, args: &[&str]) -> Output {
    Command::new(cp_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("cp の実行に失敗")
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

#[test]
fn basic_copy() {
    let d = setup("basic_copy");
    write(&d, "a.txt", "hello");
    let out = run_cp(&d, &["a.txt", "b.txt"]);
    assert!(out.status.success());
    assert!(d.join("a.txt").exists());
    assert_eq!(fs::read_to_string(d.join("b.txt")).unwrap(), "hello");
}

#[test]
fn recursive_copy_into_own_subdirectory_fails_fast() {
    let d = setup("own_subdir");
    fs::create_dir(d.join("pd")).unwrap();
    write(&d.join("pd"), "f.txt", "x");

    let out = run_cp(&d, &["-R", "pd", "pd/sub"]);
    assert!(!out.status.success(), "自身のサブディレクトリへのコピーは失敗すべき");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("サブディレクトリ"), "stderr: {}", stderr);
    // 無限再帰でゴミが作られていないこと
    assert!(!d.join("pd").join("sub").exists());
}

#[test]
fn recursive_copy_into_itself_fails() {
    let d = setup("into_itself");
    fs::create_dir(d.join("pd")).unwrap();
    write(&d.join("pd"), "f.txt", "x");

    let out = run_cp(&d, &["-R", "pd", "pd"]);
    assert!(!out.status.success());
    assert!(!d.join("pd").join("pd").exists());
}

#[test]
fn refuses_directory_over_file() {
    let d = setup("dir_over_file");
    fs::create_dir(d.join("srcdir")).unwrap();
    write(&d.join("srcdir"), "x.txt", "x");
    write(&d, "plain.txt", "keep");

    let out = run_cp(&d, &["-R", "srcdir", "plain.txt"]);
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(d.join("plain.txt")).unwrap(), "keep");
}

#[test]
fn refuses_file_over_directory() {
    let d = setup("file_over_dir");
    write(&d, "plain.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();
    write(&d.join("dest"), "keep.txt", "keep");

    let out = run_cp(&d, &["-T", "plain.txt", "dest"]);
    assert!(!out.status.success());
    assert!(d.join("dest").join("keep.txt").exists());
}

#[test]
fn recursive_copy_tree() {
    let d = setup("tree");
    fs::create_dir_all(d.join("tree").join("sub")).unwrap();
    write(&d.join("tree"), "f1.txt", "1");
    write(&d.join("tree").join("sub"), "f2.txt", "2");

    let out = run_cp(&d, &["-R", "tree", "copy"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("copy").join("sub").join("f2.txt")).unwrap(), "2");
}

#[test]
fn directory_without_recursive_is_omitted() {
    let d = setup("omit_dir");
    fs::create_dir(d.join("dir")).unwrap();
    write(&d, "f.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();

    let out = run_cp(&d, &["dir", "f.txt", "dest"]);
    assert_eq!(out.status.code(), Some(1));
    // ディレクトリは省略されるが、他のファイルはコピーされる
    assert!(d.join("dest").join("f.txt").exists());
    assert!(!d.join("dest").join("dir").exists());
}

#[test]
fn backup_simple_uses_tilde() {
    let d = setup("backup_simple");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = run_cp(&d, &["-b", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(d.join("dest.txt~")).unwrap(), "old");
}

#[test]
fn backup_numbered() {
    let d = setup("backup_numbered");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = run_cp(&d, &["--backup=numbered", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt.~1~")).unwrap(), "old");
}

#[test]
fn backup_suffix_option_and_env() {
    let d = setup("backup_suffix");
    write(&d, "src.txt", "new");
    write(&d, "dest.txt", "old");
    let out = run_cp(&d, &["-b", "-S", ".bak", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt.bak")).unwrap(), "old");

    write(&d, "dest2.txt", "old2");
    let out = Command::new(cp_bin())
        .args(["-b", "src.txt", "dest2.txt"])
        .env("SIMPLE_BACKUP_SUFFIX", ".orig")
        .current_dir(&d)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest2.txt.orig")).unwrap(), "old2");
}

#[test]
fn target_dir_and_no_target_dir_conflict() {
    let d = setup("t_conflict");
    write(&d, "a.txt", "x");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_cp(&d, &["-T", "-t", "dest", "a.txt"]);
    assert!(!out.status.success());
}

#[test]
fn no_target_dir_extra_operand() {
    let d = setup("extra_operand");
    write(&d, "a.txt", "x");
    write(&d, "b.txt", "y");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_cp(&d, &["-T", "a.txt", "b.txt", "dest"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("余分なオペランド"), "stderr: {}", stderr);
}

#[test]
fn readonly_dest_fails_without_force() {
    let d = setup("readonly_dest");
    write(&d, "src.txt", "new");
    let dest = write(&d, "dest.txt", "old");
    let mut perms = fs::metadata(&dest).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&dest, perms).unwrap();

    let out = run_cp(&d, &["src.txt", "dest.txt"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("dest.txt"), "エラーにパスが含まれるべき: {}", stderr);

    let out = run_cp(&d, &["-f", "src.txt", "dest.txt"]);
    assert!(out.status.success(), "-f は読み取り専用を上書きできるべき");
    assert_eq!(fs::read_to_string(&dest).unwrap(), "new");

    let mut perms = fs::metadata(&dest).unwrap().permissions();
    perms.set_readonly(false);
    fs::set_permissions(&dest, perms).unwrap();
}

#[cfg(windows)]
#[test]
fn symlink_dereferenced_by_default_without_recursive() {
    let d = setup("symlink_deref");
    write(&d, "target.txt", "content");
    // 権限がない環境（開発者モード無効）ではスキップ
    if std::os::windows::fs::symlink_file("target.txt", d.join("link.txt")).is_err() {
        eprintln!("skip: symlink作成権限なし");
        return;
    }

    // 非 -R: 実体をコピー
    let out = run_cp(&d, &["link.txt", "copy.txt"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(!d.join("copy.txt").symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read_to_string(d.join("copy.txt")).unwrap(), "content");

    // -P: リンクとしてコピー
    let out = run_cp(&d, &["-P", "link.txt", "copy2.txt"]);
    assert!(out.status.success());
    assert!(d.join("copy2.txt").symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn update_skips_newer_destination() {
    let d = setup("update_skip");
    let src = write(&d, "src.txt", "older");
    write(&d, "dest.txt", "newer");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    let f = fs::OpenOptions::new().write(true).open(&src).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(old)).unwrap();
    drop(f);

    let out = run_cp(&d, &["-u", "src.txt", "dest.txt"]);
    assert!(out.status.success());
    assert_eq!(fs::read_to_string(d.join("dest.txt")).unwrap(), "newer");
}

#[test]
fn glob_sources_into_directory() {
    let d = setup("glob");
    write(&d, "g1.txt", "1");
    write(&d, "g2.txt", "2");
    fs::create_dir(d.join("dest")).unwrap();
    let out = run_cp(&d, &["g*.txt", "dest"]);
    assert!(out.status.success());
    assert!(d.join("dest").join("g1.txt").exists());
    assert!(d.join("dest").join("g2.txt").exists());
}

#[test]
fn missing_operand_errors() {
    let d = setup("missing_operand");
    let out = run_cp(&d, &[]);
    assert_eq!(out.status.code(), Some(1));
    let out = run_cp(&d, &["only_one"]);
    assert_eq!(out.status.code(), Some(1));
}
