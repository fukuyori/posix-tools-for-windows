use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

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
