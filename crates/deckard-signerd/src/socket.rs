//! Socket lifecycle: where the UDS lives, its permissions, stale-socket cleanup, and the
//! single-instance lock.
//!
//! Path: `$XDG_RUNTIME_DIR/deckard/signerd.sock` (Linux); when `$XDG_RUNTIME_DIR` is unset
//! (the usual macOS case) it falls back to `$TMPDIR/deckard-$UID/signerd.sock`. The parent
//! dir is forced to `0700` and the socket to `0600` — `bind`/`mkdir` honor the umask, so we
//! chmod explicitly rather than trusting it.

use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::fcntl::{Flock, FlockArg};
use tokio::net::UnixListener;

/// The default socket path (pure — does not touch the filesystem). Call [`prepare_parent`]
/// before binding.
pub fn default_socket_path() -> PathBuf {
    runtime_dir().join("signerd.sock")
}

/// The runtime directory that holds the socket + lockfile.
fn runtime_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("deckard");
        }
    }
    // macOS (XDG_RUNTIME_DIR usually unset): per-uid dir under TMPDIR.
    let tmp = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let uid = nix::unistd::geteuid().as_raw();
    tmp.join(format!("deckard-{uid}"))
}

/// Create the socket's parent dir (if needed) and force it to `0700`.
pub fn prepare_parent(socket_path: &Path) -> std::io::Result<()> {
    if let Some(dir) = socket_path.parent() {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Acquire the single-instance lock — an exclusive, non-blocking `flock` on a sibling
/// `signerd.lock`. The returned guard must be held for the daemon's whole lifetime; the OS
/// releases the lock automatically on process exit (even on SIGKILL), so no stale-lock
/// cleanup is needed. A second daemon fails fast instead of racing on the socket.
pub fn single_instance(socket_path: &Path) -> anyhow::Result<Flock<fs::File>> {
    let lock_path = socket_path.with_extension("lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false) // it's only a lock handle; its contents are irrelevant
        .mode(0o600)
        .open(&lock_path)?;
    Flock::lock(file, FlockArg::LockExclusiveNonblock).map_err(|(_f, errno)| {
        anyhow::anyhow!("another deckard-signerd is already running ({errno})")
    })
}

/// Remove any stale socket node, then bind and chmod to `0600`.
pub fn bind(socket_path: &Path) -> std::io::Result<UnixListener> {
    match fs::remove_file(socket_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(socket_path)?;
    // bind honored the umask; force owner-only.
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_ends_in_signerd_sock() {
        assert!(default_socket_path().ends_with("signerd.sock"));
    }

    #[tokio::test]
    async fn bind_yields_a_0600_socket_in_a_0700_dir() {
        // Use an isolated temp dir as the "runtime dir".
        let base = std::env::temp_dir().join(format!("deckard-sock-test-{}", std::process::id()));
        let sock = base.join("signerd.sock");
        let _ = fs::remove_dir_all(&base);

        prepare_parent(&sock).unwrap();
        let dir_mode = fs::metadata(&base).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "parent dir must be 0700");

        let _listener = bind(&sock).unwrap();
        let sock_mode = fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(sock_mode, 0o600, "socket must be 0600");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn single_instance_is_exclusive() {
        let base = std::env::temp_dir().join(format!("deckard-lock-test-{}", std::process::id()));
        let sock = base.join("signerd.sock");
        let _ = fs::remove_dir_all(&base);
        prepare_parent(&sock).unwrap();

        let first = single_instance(&sock).expect("first lock");
        // A second attempt on the same lockfile must fail while the first is held.
        assert!(
            single_instance(&sock).is_err(),
            "second instance must be refused"
        );
        drop(first);
        // Once released, a fresh lock succeeds.
        assert!(single_instance(&sock).is_ok());

        let _ = fs::remove_dir_all(&base);
    }
}
