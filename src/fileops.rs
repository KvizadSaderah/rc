// =============================================================================
// Background filesystem operations (copy / move / delete)
//
// Heavy operations run on a worker thread so the TUI never freezes on large
// files, deep trees, or slow/network mounts. The worker reports progress over
// an mpsc channel as lightweight delta messages; the UI accumulates them into a
// `JobState` snapshot for rendering. A shared atomic flag allows cancellation.
//
// Design notes:
//   * Symlinks are never followed — they are recreated on copy and unlinked on
//     delete, so symlink loops cannot cause infinite recursion or data loss.
//   * Errors are collected per-item instead of aborting on the first failure,
//     matching the behaviour of mc/ranger (skip-and-report).
//   * Copies are chunked so cancellation is responsive even mid-file.
// =============================================================================

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const CHUNK: usize = 128 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Delete,
}

impl OpKind {
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Copy => "Copying",
            OpKind::Move => "Moving",
            OpKind::Delete => "Deleting",
        }
    }

    pub fn past(self) -> &'static str {
        match self {
            OpKind::Copy => "Copied",
            OpKind::Move => "Moved",
            OpKind::Delete => "Deleted",
        }
    }
}

/// Lightweight delta messages sent from the worker thread to the UI.
pub enum Msg {
    /// Result of the initial scan: total work to do.
    Scan { total_files: u64, total_bytes: u64 },
    /// Worker started processing a new entry (by display name).
    StartFile(String),
    /// `n` more bytes have been processed (for the byte progress bar).
    Bytes(u64),
    /// One more file/entry finished.
    FileDone,
    /// A non-fatal, per-item error; the job keeps going.
    Error(String),
    /// The job finished (successfully, cancelled, or with collected errors).
    Done,
}

/// UI-side accumulated snapshot of a running job.
pub struct JobState {
    pub kind: OpKind,
    pub total_files: u64,
    pub total_bytes: u64,
    pub done_files: u64,
    pub done_bytes: u64,
    pub current: String,
    pub errors: Vec<String>,
    pub finished: bool,
    pub cancelled_flag: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl JobState {
    /// Drain all currently-available progress messages (non-blocking).
    /// Returns true if the job just finished on this call.
    pub fn poll(&mut self) -> bool {
        let was_finished = self.finished;
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Scan { total_files, total_bytes }) => {
                    self.total_files = total_files;
                    self.total_bytes = total_bytes;
                }
                Ok(Msg::StartFile(name)) => self.current = name,
                Ok(Msg::Bytes(n)) => self.done_bytes = self.done_bytes.saturating_add(n),
                Ok(Msg::FileDone) => self.done_files = self.done_files.saturating_add(1),
                Ok(Msg::Error(e)) => self.errors.push(e),
                Ok(Msg::Done) => self.finished = true,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        if self.finished {
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
        self.finished && !was_finished
    }

    pub fn cancel(&self) {
        self.cancelled_flag.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled_flag.load(Ordering::Relaxed)
    }

    /// Fraction in 0.0..=1.0 for the progress bar (byte-based, file-count fallback).
    pub fn ratio(&self) -> f64 {
        if self.total_bytes > 0 {
            (self.done_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0)
        } else if self.total_files > 0 {
            (self.done_files as f64 / self.total_files as f64).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Spawn a background filesystem job. Each item is a `(source, destination)`
/// pair where `destination` is the full target path (allowing rename on a
/// single copy/move). For `Delete`, the destination is ignored.
pub fn spawn(kind: OpKind, items: Vec<(PathBuf, PathBuf)>) -> JobState {
    let (tx, rx) = mpsc::channel::<Msg>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);

    let handle = thread::spawn(move || {
        let ctx = Worker { tx, cancel: cancel_worker };
        ctx.run(kind, items);
    });

    JobState {
        kind,
        total_files: 0,
        total_bytes: 0,
        done_files: 0,
        done_bytes: 0,
        current: String::new(),
        errors: Vec::new(),
        finished: false,
        cancelled_flag: cancel,
        rx,
        handle: Some(handle),
    }
}

struct Worker {
    tx: Sender<Msg>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }

    fn run(&self, kind: OpKind, items: Vec<(PathBuf, PathBuf)>) {
        // 1. Scan to estimate total work (does not follow symlinks).
        let (mut total_files, mut total_bytes) = (0u64, 0u64);
        for (src, _) in &items {
            let (f, b) = scan(src);
            total_files += f;
            total_bytes += b;
        }
        self.send(Msg::Scan { total_files, total_bytes });

        // 2. Execute per item, collecting errors instead of aborting.
        for (src, dst) in &items {
            if self.cancelled() {
                self.send(Msg::Error("Cancelled by user.".to_string()));
                break;
            }
            let result = match kind {
                OpKind::Delete => self.delete_tree(src),
                OpKind::Copy => self.copy_tree(src, dst),
                OpKind::Move => self.move_one(src, dst),
            };
            if let Err(e) = result {
                if !is_cancelled_err(&e) {
                    self.send(Msg::Error(format!("{}: {}", display(src), e)));
                }
            }
        }

        self.send(Msg::Done);
    }

    // ---- Copy --------------------------------------------------------------

    fn copy_tree(&self, src: &Path, dst: &Path) -> io::Result<()> {
        if self.cancelled() {
            return Err(cancelled());
        }
        let meta = fs::symlink_metadata(src)?;
        let ft = meta.file_type();

        if ft.is_symlink() {
            self.send(Msg::StartFile(display(src)));
            let target = fs::read_link(src)?;
            // Replace any existing entry at dst so re-copies are idempotent.
            let _ = fs::remove_file(dst);
            recreate_symlink(&target, dst, src.is_dir())?;
            self.send(Msg::FileDone);
            Ok(())
        } else if ft.is_dir() {
            fs::create_dir_all(dst)?;
            // Collect child errors but keep copying siblings.
            let entries = match fs::read_dir(src) {
                Ok(e) => e,
                Err(e) => return Err(e),
            };
            for entry in entries.flatten() {
                if self.cancelled() {
                    return Err(cancelled());
                }
                let child = entry.path();
                let child_dst = dst.join(entry.file_name());
                if let Err(e) = self.copy_tree(&child, &child_dst) {
                    if is_cancelled_err(&e) {
                        return Err(e);
                    }
                    self.send(Msg::Error(format!("{}: {}", display(&child), e)));
                }
            }
            self.send(Msg::FileDone);
            Ok(())
        } else {
            self.copy_file(src, dst)?;
            self.send(Msg::FileDone);
            Ok(())
        }
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        self.send(Msg::StartFile(display(src)));
        let mut reader = File::open(src)?;
        let mut writer = File::create(dst)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if self.cancelled() {
                return Err(cancelled());
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            self.send(Msg::Bytes(n as u64));
        }
        writer.flush()?;
        // Preserve permissions where possible (best-effort).
        if let Ok(meta) = fs::metadata(src) {
            let _ = fs::set_permissions(dst, meta.permissions());
        }
        Ok(())
    }

    // ---- Move --------------------------------------------------------------

    fn move_one(&self, src: &Path, dst: &Path) -> io::Result<()> {
        self.send(Msg::StartFile(display(src)));
        match fs::rename(src, dst) {
            Ok(()) => {
                // Whole subtree moved in O(1); approximate progress accounting.
                self.send(Msg::FileDone);
                Ok(())
            }
            Err(e) if is_cross_device(&e) => {
                // Fall back to copy + delete with byte-level progress.
                self.copy_tree(src, dst)?;
                if self.cancelled() {
                    return Err(cancelled());
                }
                self.delete_tree(src)
            }
            Err(e) => Err(e),
        }
    }

    // ---- Delete ------------------------------------------------------------

    fn delete_tree(&self, path: &Path) -> io::Result<()> {
        if self.cancelled() {
            return Err(cancelled());
        }
        let meta = fs::symlink_metadata(path)?;
        let ft = meta.file_type();

        if ft.is_symlink() || !ft.is_dir() {
            self.send(Msg::StartFile(display(path)));
            fs::remove_file(path)?;
            self.send(Msg::FileDone);
            Ok(())
        } else {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    if self.cancelled() {
                        return Err(cancelled());
                    }
                    let child = entry.path();
                    if let Err(e) = self.delete_tree(&child) {
                        if is_cancelled_err(&e) {
                            return Err(e);
                        }
                        self.send(Msg::Error(format!("{}: {}", display(&child), e)));
                    }
                }
            }
            self.send(Msg::StartFile(display(path)));
            fs::remove_dir(path)?;
            self.send(Msg::FileDone);
            Ok(())
        }
    }
}

// ---- Helpers ---------------------------------------------------------------

/// Count files and bytes under `path` without following symlinks.
fn scan(path: &Path) -> (u64, u64) {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return (1, 0),
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        (1, 0)
    } else if ft.is_dir() {
        let mut files = 1; // the dir itself counts as one unit of work
        let mut bytes = 0;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let (f, b) = scan(&entry.path());
                files += f;
                bytes += b;
            }
        }
        (files, bytes)
    } else {
        (1, meta.len())
    }
}

fn recreate_symlink(target: &Path, dst: &Path, _is_dir: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, dst)
    }
    #[cfg(not(unix))]
    {
        if _is_dir {
            std::os::windows::fs::symlink_dir(target, dst)
        } else {
            std::os::windows::fs::symlink_file(target, dst)
        }
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::CrossesDevices
        || e.raw_os_error() == Some(18) // EXDEV (Unix)
        || e.raw_os_error() == Some(17) // ERROR_NOT_SAME_DEVICE (Windows)
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

fn is_cancelled_err(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Interrupted
}

/// Lossy display name for a path (robust to non-UTF8 paths).
fn display(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rc_fileops_{}_{}", tag, chrono::Utc::now().timestamp_micros()))
    }

    fn drain(mut job: JobState) -> JobState {
        // Block until the worker finishes (test-only).
        loop {
            let _ = job.poll();
            if job.finished {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        job
    }

    #[test]
    fn test_copy_tree_with_progress() {
        let root = tmp("copy");
        let src = root.join("src");
        let dst_dir = root.join("dstdir");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();
        fs::write(src.join("a.txt"), vec![7u8; 200_000]).unwrap();
        fs::write(src.join("sub/b.txt"), b"hi").unwrap();

        let job = drain(spawn(OpKind::Copy, vec![(src.clone(), dst_dir.join("src"))]));

        let copied = dst_dir.join("src");
        assert!(copied.join("a.txt").exists());
        assert!(copied.join("sub/b.txt").exists());
        assert_eq!(fs::read(copied.join("a.txt")).unwrap().len(), 200_000);
        assert!(job.errors.is_empty());
        assert!(job.done_bytes >= 200_000);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_delete_tree() {
        let root = tmp("del");
        let target = root.join("keep");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("inside.txt"), b"x").unwrap();

        let victim = root.join("victim");
        fs::create_dir_all(victim.join("deep")).unwrap();
        fs::write(victim.join("deep/f.txt"), b"bye").unwrap();

        let job = drain(spawn(OpKind::Delete, vec![(victim.clone(), PathBuf::new())]));

        assert!(!victim.exists());
        assert!(target.exists(), "unrelated dir must survive");
        assert!(job.errors.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_move_same_device() {
        let root = tmp("move");
        let src_dir = root.join("a");
        let dst_dir = root.join("b");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();
        let f = src_dir.join("file.txt");
        fs::write(&f, b"move me").unwrap();

        let job = drain(spawn(OpKind::Move, vec![(f.clone(), dst_dir.join("file.txt"))]));

        assert!(!f.exists());
        assert_eq!(fs::read_to_string(dst_dir.join("file.txt")).unwrap(), "move me");
        assert!(job.errors.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_copy_does_not_follow_symlink_loop() {
        // A directory containing a symlink back to itself must not cause
        // infinite recursion: the link is recreated, not traversed.
        let root = tmp("loop");
        let src = root.join("src");
        let dst_dir = root.join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();
        fs::write(src.join("real.txt"), b"data").unwrap();
        let loop_link = src.join("self");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, &loop_link).unwrap();
        #[cfg(not(unix))]
        std::os::windows::fs::symlink_dir(&src, &loop_link).unwrap();

        let job = drain(spawn(OpKind::Copy, vec![(src.clone(), dst_dir.join("src"))]));

        let copied = dst_dir.join("src");
        assert!(copied.join("real.txt").exists());
        let link_meta = fs::symlink_metadata(copied.join("self")).unwrap();
        assert!(link_meta.file_type().is_symlink(), "loop link recreated, not followed");
        assert!(job.errors.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_delete_symlink_keeps_target() {
        let root = tmp("symdel");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), b"keep").unwrap();
        let link = root.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        std::os::windows::fs::symlink_dir(&target, &link).unwrap();

        let job = drain(spawn(OpKind::Delete, vec![(link.clone(), PathBuf::new())]));

        assert!(!link.exists());
        assert!(target.join("keep.txt").exists(), "symlink target must survive");
        assert!(job.errors.is_empty());

        let _ = fs::remove_dir_all(&root);
    }
}
