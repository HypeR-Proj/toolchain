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
    let unsupported_copy = format!("{directory}/executable");
    assert_eq!(
        fs::copy(std::env::args().next().unwrap(), &unsupported_copy)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    assert!(!fs::exists(&unsupported_copy).unwrap());
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
    println!("HYPER_STD_FILES_OK");
}
