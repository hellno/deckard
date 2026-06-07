//! Caller authentication: only a process with the **same uid** as the daemon may connect.
//!
//! We use tokio's built-in peer-cred (`SO_PEERCRED` on Linux, `getpeereid(2)` /
//! `LOCAL_PEERCRED` on macOS) — verified to return the peer's effective uid on both — and
//! compare it against our own *effective* uid. The decision itself is a pure function so it
//! can be unit-tested without a live different-uid connection.

use tokio::net::UnixStream;

/// The peer's (effective) uid for a connected stream.
pub fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    Ok(stream.peer_cred()?.uid())
}

/// Our own effective uid (paired with `peer_cred`'s effective semantics).
pub fn our_uid() -> u32 {
    nix::unistd::geteuid().as_raw()
}

/// The whole authorization rule, pure and testable: a connection is allowed iff the peer
/// runs as the same uid as the daemon.
#[inline]
pub fn same_uid(peer_uid: u32, our_uid: u32) -> bool {
    peer_uid == our_uid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_different_uid() {
        let ours = 501;
        // A foreign uid is refused (the load-bearing check) ...
        assert!(!same_uid(ours + 1, ours));
        assert!(!same_uid(0, ours)); // even root, if it isn't us
                                     // ... and the same uid is accepted.
        assert!(same_uid(ours, ours));
    }

    #[test]
    fn our_uid_is_stable() {
        // geteuid is infallible and constant within a process.
        assert_eq!(our_uid(), our_uid());
    }
}
