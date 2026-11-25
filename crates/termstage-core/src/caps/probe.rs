//! Terminal probe I/O: write the query batch to the controlling terminal and
//! collect replies until the DA1 end marker arrives or the timeout expires.
//!
//! Uses the controlling terminal directly (`/dev/tty`, `CONIN$`/`CONOUT$`) so it
//! works even when stdout is redirected (`termstage inspect > caps.json`).

use std::io;
use std::time::{Duration, Instant};

/// Raw probe result.
#[derive(Debug, Default)]
pub struct ProbeOutput {
    pub bytes: Vec<u8>,
    /// Round-trip time until the DA1 reply arrived.
    pub rtt: Option<Duration>,
    pub timed_out: bool,
}

/// Send `query` and collect replies.
pub fn run(query: &[u8], timeout: Duration) -> io::Result<ProbeOutput> {
    imp::run(query, timeout)
}

/// Shared read loop: `read_some(remaining)` returns bytes read (0 = nothing yet).
#[allow(dead_code)] // unused on platforms without a probe implementation
fn collect(
    timeout: Duration,
    mut read_some: impl FnMut(Duration, &mut Vec<u8>) -> io::Result<()>,
) -> io::Result<ProbeOutput> {
    let start = Instant::now();
    let mut out = ProbeOutput::default();
    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            out.timed_out = true;
            return Ok(out);
        }
        read_some(timeout - elapsed, &mut out.bytes)?;
        if super::parse::has_da1(&out.bytes) {
            out.rtt = Some(start.elapsed());
            return Ok(out);
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::fs::{Mode, OFlags, open};
    use rustix::termios::{OptionalActions, Termios, tcgetattr, tcsetattr};
    use std::os::fd::{AsFd, OwnedFd};

    /// Restores the original termios on drop, even on early return.
    struct RawGuard<'a> {
        fd: &'a OwnedFd,
        original: Termios,
    }

    impl Drop for RawGuard<'_> {
        fn drop(&mut self) {
            let _ = tcsetattr(self.fd, OptionalActions::Now, &self.original);
        }
    }

    pub fn run(query: &[u8], timeout: Duration) -> io::Result<ProbeOutput> {
        let fd = open(
            "/dev/tty",
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let original = tcgetattr(&fd)?;
        let mut raw = original.clone();
        raw.make_raw();
        tcsetattr(&fd, OptionalActions::Now, &raw)?;
        let _guard = RawGuard { fd: &fd, original };

        let mut written = 0;
        while written < query.len() {
            written += rustix::io::write(&fd, &query[written..])?;
        }

        collect(timeout, |remaining, buf| {
            let ts = Timespec {
                tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
                tv_nsec: remaining.subsec_nanos().into(),
            };
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            match poll(&mut fds, Some(&ts)) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
            let mut chunk = [0u8; 1024];
            match rustix::io::read(fd.as_fd(), &mut chunk) {
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => {}
                Err(e) => return Err(e.into()),
            }
            Ok(())
        })
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub fn run(query: &[u8], timeout: Duration) -> io::Result<ProbeOutput> {
        let console = crate::os::windows::ProbeConsole::open()?;
        console.write_all(query)?;
        collect(timeout, |remaining, buf| console.read_chars(remaining, buf))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::*;

    pub fn run(_query: &[u8], _timeout: Duration) -> io::Result<ProbeOutput> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "terminal probing is not supported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_stops_at_da1() {
        let mut chunks = vec![b"\x1b[?62".to_vec(), b";22c".to_vec()].into_iter();
        let out = collect(Duration::from_secs(5), |_, buf| {
            if let Some(c) = chunks.next() {
                buf.extend(c);
            }
            Ok(())
        })
        .unwrap();
        assert!(!out.timed_out);
        assert!(out.rtt.is_some());
    }

    #[test]
    fn collect_times_out() {
        let out = collect(Duration::from_millis(20), |_, _| {
            std::thread::sleep(Duration::from_millis(5));
            Ok(())
        })
        .unwrap();
        assert!(out.timed_out && out.rtt.is_none());
    }
}
