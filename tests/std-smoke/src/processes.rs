// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn child(mode: &str) {
    match mode {
        "output" => {
            let out = [b'o'; 4096];
            let err = [b'e'; 4096];
            for _ in 0..320 {
                std::io::stdout().write_all(&out).unwrap();
                std::io::stderr().write_all(&err).unwrap();
            }
        }
        "empty-output" => {
            let _startup = hyper_rt::process::startup().unwrap();
            hyper_rt::process::stdout()
                .unwrap()
                .as_byte_channel()
                .send(b"")
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
            std::io::stderr()
                .write_all(&vec![b'e'; 2 * 1024 * 1024])
                .unwrap();
            println!("after-empty");
        }
        "input" => {
            let mut bytes = Vec::new();
            std::io::stdin().read_to_end(&mut bytes).unwrap();
            std::io::stdout().write_all(&bytes).unwrap();
        }
        "env" => {
            assert_eq!(std::env::var("STD_CHILD_VALUE").unwrap(), "with spaces");
            assert!(std::env::var("STD_REMOVED").is_err());
            println!("child-env-ok");
            std::process::exit(7);
        }
        "sleep" => std::thread::sleep(Duration::from_secs(30)),
        "held-output" => std::thread::sleep(Duration::from_secs(10)),
        "detached-output" => {
            let child = command("held-output").spawn();
            assert!(child.is_ok(), "failed to start output holder: {child:?}");
            drop(child);
            println!("OUTPUT_OWNER_EXITED");
        }
        "null" => println!("discarded output"),
        "cwd" => {
            std::fs::write("child-file", b"shared ramfs").unwrap();
        }
        _ => panic!("unknown std child mode"),
    }
}

fn command(mode: &str) -> Command {
    let mut command = Command::new(std::env::args().next().unwrap());
    command.args(["--child", mode]);
    command
}

pub fn run() {
    assert_eq!(
        u64::from(std::process::id()),
        hyper_os::task::current_process_id().unwrap()
    );
    let output = command("empty-output").output().unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"after-empty\n");
    assert_eq!(output.stderr, vec![b'e'; 2 * 1024 * 1024]);
    let output = command("output").output().unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, vec![b'o'; 320 * 4096]);
    assert_eq!(output.stderr, vec![b'e'; 320 * 4096]);
    let mut child = command("output")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut prefix = [0; 1];
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut prefix)
        .unwrap();
    assert_eq!(prefix, [b'o']);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, vec![b'o'; 320 * 4096 - 1]);
    assert_eq!(output.stderr, vec![b'e'; 320 * 4096]);
    let output = command("env")
        .env_clear()
        .env("STD_CHILD_VALUE", "with spaces")
        .env_remove("STD_REMOVED")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"child-env-ok\n");
    assert!(output.stderr.is_empty());
    let mut child = command("input")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"pipe-prefix").unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"pipe-prefix");
    let mut child = command("sleep").spawn().unwrap();
    assert!(child.id() > 0);
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success());
    assert_eq!(child.try_wait().unwrap(), Some(status));
    assert!(
        command("null")
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let directory = format!("/std-child-{}", std::process::id());
    std::fs::create_dir(&directory).unwrap();
    assert!(
        command("cwd")
            .current_dir(&directory)
            .status()
            .unwrap()
            .success()
    );
    let path = format!("{directory}/child-file");
    assert_eq!(std::fs::read(&path).unwrap(), b"shared ramfs");
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
    println!("HYPER_STD_PROCESSES_OK");
}
