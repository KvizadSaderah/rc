// =============================================================================
// Filesystem change watcher
//
// Replaces the old blind "refresh both panels every 2 seconds" loop with event
// driven refreshes via the `notify` crate. Each panel directory is watched
// non-recursively; incoming events set a dirty flag that the main loop debounces
// before refreshing, so a burst of external writes coalesces into one refresh.
//
// The watcher is best-effort: if it fails to initialise (unsupported platform,
// permission issues), the caller falls back to a slow periodic refresh.
// =============================================================================

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

pub struct FsWatcher {
    watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
    watched: Vec<PathBuf>,
}

impl FsWatcher {
    /// Create a watcher, or `None` if the platform backend can't be set up.
    pub fn new() -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        match notify::recommended_watcher(tx) {
            Ok(watcher) => Some(Self {
                watcher,
                rx,
                watched: Vec::new(),
            }),
            Err(_) => None,
        }
    }

    /// Ensure exactly `paths` are being watched (no-op if unchanged). Watching
    /// is non-recursive: we only care about direct children of a panel dir.
    pub fn sync_paths(&mut self, paths: &[PathBuf]) {
        if self.watched == paths {
            return;
        }
        for p in &self.watched {
            let _ = self.watcher.unwatch(p);
        }
        self.watched.clear();
        for p in paths {
            if self.watched.contains(p) {
                continue;
            }
            if self.watcher.watch(p, RecursiveMode::NonRecursive).is_ok() {
                self.watched.push(p.clone());
            }
        }
    }

    /// Drain pending events. Returns true if any actionable change was seen.
    pub fn drain(&self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(Ok(ev)) => {
                    if is_actionable(&ev) {
                        changed = true;
                    }
                }
                // An error event (e.g. overflow) — refresh to be safe.
                Ok(Err(_)) => changed = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }
}

/// Ignore pure access/metadata-only events; refresh on create/remove/modify/rename.
fn is_actionable(ev: &Event) -> bool {
    use notify::EventKind;
    matches!(
        ev.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) | EventKind::Any
    )
}
