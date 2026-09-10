// SPDX-FileCopyrightText: 2026 roolrz
// SPDX-License-Identifier: Apache-2.0

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

pub fn run() {
    let directory = format!("/std-check-{}", std::process::id());
    fs::create_dir(&directory).unwrap();
    let path = format!("{directory}/data");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(b"abc").unwrap();
    file.seek(SeekFrom::Start(7)).unwrap();
    file.write_all(b"z").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"abc\0\0\0\0z");
    assert_eq!(file.metadata().unwrap().len(), 8);
    assert!(file.set_len(u64::MAX).is_err());
    assert_eq!(file.metadata().unwrap().len(), 8);
    file.set_len(2).unwrap();
    file.set_len(8).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"ab\0\0\0\0\0\0");
    file.rewind().unwrap();
    let mut clone = file.try_clone().unwrap();
    let mut bytes = [0; 1];
    clone.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [b'a']);
    assert_eq!(file.stream_position().unwrap(), 1);
    fs::remove_file(&path).unwrap();
    assert!(!fs::exists(&path).unwrap());
    fs::write(&path, b"new").unwrap();
    file.rewind().unwrap();
    file.write_all(b"old").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"new");
    drop(clone);
    drop(file);

    // Keep enough descriptors live to cross several Native handle-table
    // segments, then exercise reuse after closing the complete batch.
    for _ in 0..2 {
        let mut readers = (0..256)
            .map(|_| File::open(&path))
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        for reader in &mut readers {
            let mut bytes = [0; 3];
            reader.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"new");
        }
    }

    let mut append = OpenOptions::new().append(true).open(&path).unwrap();
    append.seek(SeekFrom::Start(1)).unwrap();
    assert_eq!(append.write(b"").unwrap(), 0);
    assert_eq!(append.stream_position().unwrap(), 1);
    drop(append);
    let mut workers = Vec::new();
    for byte in *b"xyzw" {
        let path = path.clone();
        workers.push(std::thread::spawn(move || {
            let mut file = OpenOptions::new().append(true).open(path).unwrap();
            for _ in 0..32 {
                file.write_all(&[byte; 16]).unwrap();
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let contents = fs::read(&path).unwrap();
    assert_eq!(contents.len(), 3 + 4 * 32 * 16);
    for record in contents[3..].chunks_exact(16) {
        assert!(record.iter().all(|byte| *byte == record[0]));
    }
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        fs::remove_dir(&directory).unwrap_err().kind(),
        std::io::ErrorKind::DirectoryNotEmpty
    );
    let copied = format!("{directory}/copy");
    assert_eq!(fs::copy(&path, &copied).unwrap(), contents.len() as u64);
    assert_eq!(fs::read(&copied).unwrap(), contents);
    let entries = fs::read_dir(&directory)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| entry.file_type().unwrap().is_file())
    );
    assert!(File::open(&directory).is_err());
    fs::remove_file(&path).unwrap();
    fs::remove_file(&copied).unwrap();
    fs::remove_dir(&directory).unwrap();
    assert_eq!(
        File::open("/missing").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    extended();
    println!("HYPER_STD_FILES_OK");
}

fn extended() {
    use std::os::hyper::fs::{PermissionsExt, symlink};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let base = format!("/std-fs-full-{}", std::process::id());
    fs::create_dir_all(format!("{base}/tree/child")).unwrap();
    let file_path = format!("{base}/file");
    fs::write(&file_path, b"original").unwrap();
    let opened = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .unwrap();
    opened.sync_data().unwrap();
    opened.sync_all().unwrap();
    let now = SystemTime::now();
    assert!(now.duration_since(UNIX_EPOCH).unwrap().as_secs() > 1_577_836_800);
    assert!(opened.metadata().unwrap().created().unwrap() <= now);
    std::thread::sleep(Duration::from_millis(2));
    assert!(SystemTime::now() > now);
    let before_epoch = UNIX_EPOCH - Duration::from_nanos(1);
    let modified = UNIX_EPOCH + Duration::new(123456789, 987654321);
    opened
        .set_times(
            fs::FileTimes::new()
                .set_accessed(before_epoch)
                .set_modified(modified),
        )
        .unwrap();
    let metadata = opened.metadata().unwrap();
    assert_eq!(metadata.accessed().unwrap(), before_epoch);
    assert_eq!(metadata.modified().unwrap(), modified);
    opened.set_modified(before_epoch).unwrap();
    assert_eq!(opened.metadata().unwrap().accessed().unwrap(), before_epoch);
    assert_eq!(opened.metadata().unwrap().modified().unwrap(), before_epoch);
    opened
        .set_permissions(fs::Permissions::from_mode(0o444))
        .unwrap();
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .open(&file_path)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    // Granted write authority survives later changes to admission metadata.
    (&opened).write_all(b"updated!").unwrap();
    opened
        .set_permissions(fs::Permissions::from_mode(0o755))
        .unwrap();
    let copied = format!("{base}/copied");
    assert_eq!(fs::copy(&file_path, &copied).unwrap(), 8);
    assert_eq!(
        fs::metadata(&copied).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let linked = format!("{base}/linked");
    fs::hard_link(&file_path, &linked).unwrap();
    fs::remove_file(&file_path).unwrap();
    assert_eq!(fs::read(&linked).unwrap(), b"updated!");
    fs::write(&file_path, b"replacement").unwrap();
    fs::rename(&file_path, &linked).unwrap();
    assert_eq!(fs::read(&linked).unwrap(), b"replacement");
    (&opened).rewind().unwrap();
    let mut old = String::new();
    (&opened).read_to_string(&mut old).unwrap();
    assert_eq!(old, "updated!");
    let alias = format!("{base}/alias");
    symlink("linked", &alias).unwrap();
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    assert!(fs::metadata(&alias).unwrap().is_file());
    assert_eq!(
        fs::read_link(&alias).unwrap(),
        std::path::Path::new("linked")
    );
    assert_eq!(
        fs::canonicalize(&alias).unwrap(),
        std::path::Path::new(&linked)
    );
    let dangling = format!("{base}/dangling");
    symlink("created-through-link", &dangling).unwrap();
    fs::write(&dangling, b"target").unwrap();
    assert_eq!(
        fs::read(format!("{base}/created-through-link")).unwrap(),
        b"target"
    );
    assert!(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dangling)
            .is_err()
    );
    let saved = std::env::current_dir().unwrap();
    std::env::set_current_dir(format!("{base}/tree/child")).unwrap();
    fs::rename(format!("{base}/tree"), format!("{base}/renamed")).unwrap();
    assert_eq!(
        std::env::current_dir().unwrap(),
        std::path::PathBuf::from(format!("{base}/renamed/child"))
    );
    assert_eq!(
        fs::canonicalize("..").unwrap(),
        std::path::PathBuf::from(format!("{base}/renamed"))
    );
    symlink(&linked, "absolute-link").unwrap();
    assert_eq!(fs::read("absolute-link").unwrap(), b"replacement");
    std::env::set_current_dir(&saved).unwrap();
    let sentinel = format!("{base}/sentinel");
    fs::write(&sentinel, b"preserve").unwrap();
    symlink(&base, format!("{base}/renamed/child/outside")).unwrap();
    assert_eq!(
        fs::remove_dir_all(format!("{base}/renamed/."))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert!(
        fs::metadata(format!("{base}/renamed/child"))
            .unwrap()
            .is_dir()
    );
    fs::remove_dir_all(format!("{base}/renamed/")).unwrap();
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
    file_locks(&linked);
    fs::remove_dir_all(&base).unwrap();
}

fn file_locks(path: &str) {
    use std::fs::TryLockError;
    use std::sync::{Arc, Barrier};
    let first = File::open(path).unwrap();
    let second = File::open(path).unwrap();
    first.lock_shared().unwrap();
    second.try_lock_shared().unwrap();
    assert!(matches!(first.try_lock(), Err(TryLockError::WouldBlock)));
    second.unlock().unwrap();
    first.try_lock().unwrap();
    let clone = first.try_clone().unwrap();
    drop(first);
    assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
    let barrier = Arc::new(Barrier::new(2));
    let child_barrier = barrier.clone();
    let worker = std::thread::spawn(move || {
        child_barrier.wait();
        second.lock().unwrap();
        second.unlock().unwrap();
    });
    barrier.wait();
    drop(clone);
    worker.join().unwrap();
    let mut child = std::process::Command::new(std::env::args().next().unwrap())
        .args(["--child", "file-lock"])
        .env("STD_LOCK_PATH", path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut signal = [0; 7];
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut signal)
        .unwrap();
    assert_eq!(&signal, b"locked\n");
    let waiter = File::open(path).unwrap();
    assert!(matches!(waiter.try_lock(), Err(TryLockError::WouldBlock)));
    child.kill().unwrap();
    child.wait().unwrap();
    waiter.lock().unwrap();
    waiter.unlock().unwrap();
}

// Construct a valid Native root cursor whose authority root and current node
// differ. The std runtime must normalize it once, not interpret /x and x in
// different directories. This deliberately uses real Native ProcessBuilder
// delegation rather than only modeling Directory resolution on the host.
pub fn scoped_startup_root(startup: &mut hyper_os::startup::Startup<'_>) {
    use hyper_os::fs::{DirectoryRights, FileRights};
    use hyper_os::handle::RightsOffer;
    use hyper_os::startup::{
        DYNAMIC_LIBRARY_DIRECTORY, RESOURCE_DOMAIN, ROOT_DIRECTORY, TASK_FACTORY, TASK_GROUP,
    };
    use hyper_os::task::{ProcessBuilder, ProcessTermination};
    let base = format!("/std-root-cursor-{}", std::process::id());
    fs::create_dir(&base).unwrap();
    fs::create_dir(format!("{base}/child")).unwrap();
    fs::write(format!("{base}/x"), b"selected root").unwrap();
    let root = startup.take_root_directory().unwrap();
    let rights = DirectoryRights::from_rights(root.as_handle_ref().info().unwrap().rights).unwrap();
    let selected = root.open_directory(&base, rights).unwrap();
    let cursor = root.scope_at(&selected, rights).unwrap();
    let program = std::env::args().next().unwrap();
    let executable = root.open(&program, FileRights::EXECUTE).unwrap();
    let builder = ProcessBuilder::create(
        startup.borrow(TASK_FACTORY).unwrap(),
        startup.borrow(TASK_GROUP).unwrap(),
        startup.borrow(RESOURCE_DOMAIN).unwrap(),
        executable.as_handle_ref(),
    )
    .unwrap();
    builder.set_name("std-root-cursor").unwrap();
    builder.add_argument(&program).unwrap();
    builder.add_argument("--child").unwrap();
    builder.add_argument("scoped-root").unwrap();
    builder
        .add_handle_duplicate(
            cursor.as_handle_ref(),
            ROOT_DIRECTORY.as_raw(),
            RightsOffer::SameRights,
        )
        .unwrap();
    builder
        .add_handle_duplicate(
            startup.borrow(DYNAMIC_LIBRARY_DIRECTORY).unwrap(),
            DYNAMIC_LIBRARY_DIRECTORY.as_raw(),
            RightsOffer::SameRights,
        )
        .unwrap();
    builder.seal().unwrap();
    let process = builder.start().map_err(|failure| failure.error()).unwrap();
    let supervisor = process.as_process_supervisor();
    let deadline = hyper_os::time::deadline_after(std::time::Duration::from_secs(30)).unwrap();
    let waited = supervisor.wait_terminated(deadline.as_raw());
    if waited.is_err() {
        supervisor.request_stop().unwrap();
    }
    waited.unwrap();
    assert_eq!(
        supervisor.info().unwrap().terminal,
        Some(ProcessTermination::ProcessExited { status: 0 })
    );
    fs::remove_dir_all(&base).unwrap();
}

pub fn scoped_root_child() {
    assert_eq!(std::env::current_dir().unwrap(), std::path::Path::new("/"));
    assert_eq!(fs::read("x").unwrap(), b"selected root");
    assert_eq!(fs::read("/x").unwrap(), b"selected root");
    assert_eq!(fs::canonicalize("x").unwrap(), std::path::Path::new("/x"));
    assert_eq!(fs::canonicalize("/x").unwrap(), std::path::Path::new("/x"));
    std::env::set_current_dir("child").unwrap();
    assert_eq!(
        std::env::current_dir().unwrap(),
        std::path::Path::new("/child")
    );
    assert_eq!(fs::read("../x").unwrap(), fs::read("/x").unwrap());
    std::process::exit(0);
}
