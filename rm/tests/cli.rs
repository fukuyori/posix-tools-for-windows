use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn rm() -> Command {
    Command::cargo_bin("rm").expect("binary should build")
}

#[test]
fn binary_rejects_dot_operand() {
    Command::cargo_bin("rm")
        .expect("binary should build")
        .arg(".")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("'.' または '..' は削除できません"));
}

#[test]
fn binary_expands_glob_operand() {
    let temp = tempdir().expect("tempdir should be created");
    let matching_file = temp.path().join("match.tmp");
    let other_file = temp.path().join("other.log");
    fs::write(&matching_file, b"data").expect("matching file should be created");
    fs::write(&other_file, b"data").expect("other file should be created");

    Command::cargo_bin("rm")
        .expect("binary should build")
        .current_dir(temp.path())
        .arg("*.tmp")
        .assert()
        .success();

    assert!(!matching_file.exists());
    assert!(other_file.exists());
}

#[test]
fn force_does_not_hide_directory_error() {
    let temp = tempdir().unwrap();
    fs::create_dir(temp.path().join("dir")).unwrap();

    // GNU準拠: rm -f dir はエラー（-f が無視するのは「存在しない」のみ）
    rm().current_dir(temp.path())
        .args(["-f", "dir"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ディレクトリ"));
    assert!(temp.path().join("dir").exists());
}

#[test]
fn force_ignores_missing_file() {
    let temp = tempdir().unwrap();
    rm().current_dir(temp.path())
        .args(["-f", "nofile"])
        .assert()
        .success();
    rm().current_dir(temp.path())
        .arg("nofile")
        .assert()
        .code(1);
}

#[test]
fn verbose_goes_to_stdout() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("v.txt"), b"x").unwrap();

    rm().current_dir(temp.path())
        .args(["-v", "v.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("'v.txt' を削除しました"));
}

#[test]
fn recursive_removes_tree_with_readonly_files() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("tree");
    fs::create_dir_all(dir.join("sub")).unwrap();
    let ro = dir.join("sub").join("ro.txt");
    fs::write(&ro, b"x").unwrap();
    let mut perms = fs::metadata(&ro).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&ro, perms).unwrap();

    rm().current_dir(temp.path())
        .args(["-r", "tree"])
        .assert()
        .success();
    assert!(!dir.exists());
}

#[test]
fn dir_option_removes_only_empty_directory() {
    let temp = tempdir().unwrap();
    fs::create_dir(temp.path().join("empty")).unwrap();
    fs::create_dir(temp.path().join("full")).unwrap();
    fs::write(temp.path().join("full").join("f.txt"), b"x").unwrap();

    rm().current_dir(temp.path())
        .args(["-d", "empty"])
        .assert()
        .success();
    assert!(!temp.path().join("empty").exists());

    rm().current_dir(temp.path())
        .args(["-d", "full"])
        .assert()
        .code(1);
    assert!(temp.path().join("full").exists());
}

#[test]
fn partial_failure_continues_and_exits_nonzero() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"x").unwrap();

    rm().current_dir(temp.path())
        .args(["a.txt", "missing.txt"])
        .assert()
        .code(1);
    assert!(!temp.path().join("a.txt").exists());
}

#[test]
fn preserve_root_all_is_accepted() {
    let temp = tempdir().unwrap();
    rm().current_dir(temp.path())
        .args(["--preserve-root=all", "-f", "nofile"])
        .assert()
        .success();
}

#[test]
fn root_removal_is_refused() {
    let root = if cfg!(windows) { "C:\\" } else { "/" };
    rm().args(["-rf", root])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no-preserve-root"));
}

#[test]
fn one_file_system_on_same_volume_removes_tree() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("tree");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("sub").join("f.txt"), b"x").unwrap();

    rm().current_dir(temp.path())
        .args(["-r", "--one-file-system", "tree"])
        .assert()
        .success();
    assert!(!dir.exists());
}

#[cfg(windows)]
#[test]
fn symlink_to_directory_removes_link_only() {
    let temp = tempdir().unwrap();
    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("keep.txt"), b"keep").unwrap();
    // 権限がない環境（開発者モード無効）ではスキップ
    if std::os::windows::fs::symlink_dir(&real, temp.path().join("dlink")).is_err() {
        eprintln!("skip: symlink作成権限なし");
        return;
    }

    rm().current_dir(temp.path())
        .arg("dlink")
        .assert()
        .success();
    assert!(!temp.path().join("dlink").exists());
    assert!(real.join("keep.txt").exists(), "リンク先の実体は残るべき");
}

#[cfg(windows)]
#[test]
fn broken_symlink_is_removable() {
    let temp = tempdir().unwrap();
    if std::os::windows::fs::symlink_file("nonexistent", temp.path().join("broken")).is_err() {
        eprintln!("skip: symlink作成権限なし");
        return;
    }

    rm().current_dir(temp.path())
        .arg("broken")
        .assert()
        .success();
    assert!(temp.path().join("broken").symlink_metadata().is_err());
}

#[cfg(windows)]
#[test]
fn junction_inside_tree_does_not_delete_target_contents() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("jtarget");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("data.txt"), b"important").unwrap();

    let tree = temp.path().join("tree");
    fs::create_dir(&tree).unwrap();
    // ジャンクション作成は権限不要（mklink /J）
    let status = std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            tree.join("junc").to_str().unwrap(),
            target.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    if !status.status.success() {
        eprintln!("skip: ジャンクションを作成できません");
        return;
    }

    rm().current_dir(temp.path())
        .args(["-r", "tree"])
        .assert()
        .success();
    assert!(!tree.exists());
    assert!(target.join("data.txt").exists(), "ジャンクション先の実体は残るべき");
}
