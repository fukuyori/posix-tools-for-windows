use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

fn cat_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cat")
}

fn unique_temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let dir = env::temp_dir().join(format!("cat-posix-test-{}-{}", std::process::id(), nanos));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("write test file");
}

#[test]
fn unknown_option_exits_nonzero() {
    let output = Command::new(cat_bin())
        .arg("--definitely-invalid-option")
        .output()
        .expect("run cat");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("不明なオプション"));
}

#[test]
fn reads_stdin_when_no_files_are_given() {
    let mut child = Command::new(cat_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cat");

    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        stdin.write_all(b"abc\x00def\n").expect("write stdin");
    }

    let output = child.wait_with_output().expect("wait for cat");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"abc\x00def\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn copies_file_bytes_without_text_conversion() {
    let dir = unique_temp_dir();
    let file = dir.join("binary.bin");
    let bytes = b"\x00\xffHello\r\nworld\x80";
    write_file(&file, bytes);

    let output = Command::new(cat_bin())
        .arg(&file)
        .output()
        .expect("run cat");

    assert!(output.status.success());
    assert_eq!(output.stdout, bytes);
    assert!(output.stderr.is_empty());
}

#[test]
fn unbuffered_option_flushes_before_stdin_closes() {
    let mut child = Command::new(cat_bin())
        .arg("-u")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cat");

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        let result = stdout.read_exact(&mut byte).map(|_| byte[0]);
        let _ = tx.send(result);
    });

    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        stdin.write_all(b"Z").expect("write stdin");
        stdin.flush().expect("flush stdin");
    }

    let first = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .expect("receive first byte before stdin closes")
        .expect("read first byte");
    assert_eq!(first, b'Z');

    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for cat");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn dash_operand_reads_stdin_in_sequence() {
    let dir = unique_temp_dir();
    let file = dir.join("tail.txt");
    write_file(&file, b"tail\n");

    let mut child = Command::new(cat_bin())
        .arg("-")
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cat");

    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        stdin.write_all(b"head\n").expect("write stdin");
    }

    let output = child.wait_with_output().expect("wait for cat");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"head\ntail\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn continues_after_file_error_and_returns_nonzero() {
    let dir = unique_temp_dir();
    let readable = dir.join("readable.txt");
    let subdir = dir.join("subdir");

    write_file(&readable, b"ok\n");
    fs::create_dir_all(&subdir).expect("create directory");

    let output = Command::new(cat_bin())
        .arg(&subdir)
        .arg(&readable)
        .output()
        .expect("run cat");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ディレクトリです"));
    assert_eq!(output.stdout, b"ok\n");
}
