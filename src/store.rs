use std::collections::{BTreeMap, BTreeSet};
use std::hash::{BuildHasher, RandomState};
use std::net::{Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dfui::Notifier;

const PORT: u16 = 47_474;
const TICK: Duration = Duration::from_secs(1);
const PEER_TIMEOUT: Duration = Duration::from_secs(5);

/// Names and every device's votes. Each device only writes its own votes,
/// so merging is a union with the newest vote per (device, name).
#[derive(Default, Debug, PartialEq)]
pub struct Store {
    pub me: u32,
    pub names: BTreeSet<String>,
    /// (device, name) → (millis, accepted); `None` is a cleared swipe, kept
    /// so the reset wins the merge instead of the old vote coming back.
    pub votes: BTreeMap<(u32, String), (u64, Option<bool>)>,
}

impl Store {
    pub fn add(&mut self, name: &str) {
        let name = name.trim().replace(['\t', '\n', '\r'], " ");
        if !name.is_empty() {
            self.names.insert(name);
        }
    }

    pub fn vote(&mut self, name: &str, yes: Option<bool>) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let ts = self.votes.get(&(self.me, name.into())).map_or(now, |v| now.max(v.0 + 1));
        self.votes.insert((self.me, name.into()), (ts, yes));
    }

    pub fn mine(&self, name: &str) -> Option<bool> {
        self.votes.get(&(self.me, name.into())).and_then(|v| v.1)
    }

    pub fn pending(&self) -> Vec<String> {
        self.names.iter().filter(|n| self.mine(n).is_none()).cloned().collect()
    }

    pub fn agreed(&self) -> Vec<String> {
        let others_like = |n: &str| {
            self.votes.iter().any(|((d, m), v)| *d != self.me && m == n && v.1 == Some(true))
        };
        self.names
            .iter()
            .filter(|n| self.mine(n) == Some(true) && others_like(n))
            .cloned()
            .collect()
    }

    /// Returns whether anything changed.
    pub fn merge(&mut self, other: Store) -> bool {
        let mut changed = false;
        for n in other.names {
            changed |= self.names.insert(n);
        }
        for (k, v) in other.votes {
            if self.votes.get(&k).is_none_or(|old| old.0 < v.0) {
                self.names.insert(k.1.clone());
                self.votes.insert(k, v);
                changed = true;
            }
        }
        changed
    }

    pub fn encode(&self) -> String {
        let mut s = format!("I\t{}\n", self.me);
        for n in &self.names {
            s += &format!("N\t{n}\n");
        }
        for ((d, n), (ts, yes)) in &self.votes {
            s += &format!("V\t{d}\t{ts}\t{}\t{n}\n", match yes { Some(true) => '+', Some(false) => '-', None => '0' });
        }
        s
    }

    /// Lenient: malformed lines are skipped, since packets come from the network.
    pub fn decode(s: &str) -> Store {
        let mut st = Store::default();
        for line in s.lines() {
            let f: Vec<&str> = line.splitn(5, '\t').collect();
            match f[..] {
                ["I", id] => st.me = id.parse().unwrap_or(0),
                ["N", n] => st.add(n),
                ["V", d, ts, yes, n] => {
                    if let (Ok(d), Ok(ts)) = (d.parse(), ts.parse()) {
                        st.names.insert(n.into());
                        st.votes.insert((d, n.into()), (ts, match yes { "+" => Some(true), "-" => Some(false), _ => None }));
                    }
                }
                _ => {}
            }
        }
        st
    }
}

pub struct Shared {
    pub store: Store,
    pub peer: bool,
    path: PathBuf,
}

pub type Sync = Arc<Mutex<Shared>>;

impl Shared {
    /// Writes through a temp file so a crash mid-write can't lose the list.
    pub fn save(&self) {
        let tmp = self.path.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, self.store.encode())
            .and_then(|_| std::fs::rename(&tmp, &self.path))
        {
            eprintln!("can't save {}: {e}", self.path.display());
        }
    }
}

pub fn load(path: &Path) -> Sync {
    let mut store = std::fs::read_to_string(path).map(|s| Store::decode(&s)).unwrap_or_default();
    for n in include_str!("../assets/seed.txt").lines() {
        store.add(n);
    }
    if store.me == 0 {
        store.me = (RandomState::new().hash_one(SystemTime::now()) as u32).max(1);
    }
    Arc::new(Mutex::new(Shared { store, peer: false, path: path.into() }))
}

/// Broadcasts the whole state every second and merges whatever arrives.
// ponytail: full state per datagram, caps out near 64 KB (~1000 names with two voters); delta sync past that.
pub fn start(sync: Sync, notifier: Notifier) {
    let sock = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)) {
        Ok(s) => s,
        Err(e) => return eprintln!("sync off, can't bind :{PORT}: {e}"),
    };
    sock.set_broadcast(true).ok();
    sock.set_read_timeout(Some(TICK)).ok();
    std::thread::spawn(move || {
        let mut buf = vec![0; 65_536];
        let mut sent = Instant::now() - TICK;
        let mut seen: Option<Instant> = None;
        loop {
            if sent.elapsed() >= TICK {
                sent = Instant::now();
                let msg = sync.lock().unwrap().store.encode();
                for to in targets() {
                    sock.send_to(msg.as_bytes(), (to, PORT)).ok();
                }
            }
            if let Ok((n, _)) = sock.recv_from(&mut buf)
                && let Ok(s) = std::str::from_utf8(&buf[..n])
            {
                let other = Store::decode(s);
                let mut sh = sync.lock().unwrap();
                if other.me != 0 && other.me != sh.store.me {
                    seen = Some(Instant::now());
                }
                if sh.store.merge(other) {
                    sh.save();
                    notifier.notify();
                }
            }
            let peer = seen.is_some_and(|t| t.elapsed() < PEER_TIMEOUT);
            let mut sh = sync.lock().unwrap();
            if sh.peer != peer {
                sh.peer = peer;
                notifier.notify();
            }
        }
    });
}

/// Limited broadcast plus the /24 directed one, as some routers and phones drop either.
fn targets() -> Vec<Ipv4Addr> {
    let mut t = vec![Ipv4Addr::BROADCAST];
    let local = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|s| s.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).map(|_| s))
        .and_then(|s| s.local_addr());
    if let Ok(std::net::SocketAddr::V4(a)) = local {
        let [a, b, c, _] = a.ip().octets();
        t.push(Ipv4Addr::new(a, b, c, 255));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_phones_converge() {
        let mut a = Store { me: 1, ..Default::default() };
        let mut b = Store { me: 2, ..Default::default() };
        a.add("Ana");
        a.add("Luis");
        b.merge(Store::decode(&a.encode()));
        assert_eq!(b.pending(), ["Ana", "Luis"]);

        a.vote("Ana", Some(true));
        a.vote("Luis", Some(false));
        b.vote("Ana", Some(true));
        b.vote("Luis", Some(true));
        b.add("Eva");
        assert!(a.merge(Store::decode(&b.encode())));
        assert!(b.merge(Store::decode(&a.encode())));
        assert!(!b.merge(Store::decode(&a.encode())));

        assert_eq!(a.agreed(), ["Ana"]);
        assert_eq!(a.mine("Luis"), Some(false));
        assert_eq!(a.pending(), ["Eva"]);
        // Changing your mind on a rejected name wins over the old vote.
        a.vote("Luis", Some(true));
        b.merge(Store::decode(&a.encode()));
        assert_eq!(b.agreed(), ["Ana", "Luis"]);
        // A reset swipe puts the name back in the deck on both sides of a merge.
        a.vote("Ana", None);
        b.merge(Store::decode(&a.encode()));
        a.merge(Store::decode(&b.encode()));
        assert_eq!(a.pending(), ["Ana", "Eva"]);
        assert_eq!(b.agreed(), ["Luis"]);
        assert_eq!(Store::decode(&b.encode()), b);
    }
}
