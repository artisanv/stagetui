//! Line protocol between the app and the reaper sidecar.

/// One registration message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Track a process group led by this pid.
    Pid(u32),
    /// Stop tracking a pid (it exited or was killed normally).
    Unpid(u32),
    /// Track a shared-memory name.
    Shm(String),
    /// Stop tracking a shared-memory name.
    Unshm(String),
}

impl Msg {
    /// Encode as one `\n`-terminated line.
    pub fn encode(&self) -> String {
        match self {
            Self::Pid(p) => format!("pid {p}\n"),
            Self::Unpid(p) => format!("unpid {p}\n"),
            Self::Shm(n) => format!("shm {n}\n"),
            Self::Unshm(n) => format!("unshm {n}\n"),
        }
    }

    /// Decode one line (without or with its trailing newline).
    pub fn decode(line: &str) -> Option<Self> {
        let (kind, arg) = line.trim_end_matches(['\r', '\n']).split_once(' ')?;
        let valid_name =
            |n: &str| !n.is_empty() && n.len() <= 255 && !n.contains(char::is_whitespace);
        match kind {
            "pid" => arg.parse().ok().filter(|&p| p > 1).map(Self::Pid),
            "unpid" => arg.parse().ok().map(Self::Unpid),
            "shm" if valid_name(arg) => Some(Self::Shm(arg.to_string())),
            "unshm" if valid_name(arg) => Some(Self::Unshm(arg.to_string())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for m in [
            Msg::Pid(4242),
            Msg::Unpid(4242),
            Msg::Shm("/tsg-1-0".into()),
            Msg::Unshm("/tsg-1-0".into()),
        ] {
            assert_eq!(Msg::decode(&m.encode()), Some(m));
        }
    }

    #[test]
    fn rejects_garbage_and_dangerous_pids() {
        assert_eq!(Msg::decode("pid 1"), None); // never kill init
        assert_eq!(Msg::decode("pid 0"), None);
        assert_eq!(Msg::decode("pid -5"), None);
        assert_eq!(Msg::decode("hello"), None);
        assert_eq!(Msg::decode("shm "), None);
        assert_eq!(Msg::decode("shm a b"), None);
    }
}
