use std::collections::{BTreeMap, BTreeSet};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, RandomState};
use std::net::{Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dfui::Notifier;

const PORT: u16 = 47_474;
const TICK: Duration = Duration::from_secs(1);
const PEER_TIMEOUT: Duration = Duration::from_secs(5);

/// Cards between showings of a liked name, times its streak.
const LIKE_EVERY: u32 = 10;
/// Same for a discarded name.
const NOPE_EVERY: u32 = 20;

/// One device's swipe on a name.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Vote {
    pub ts: u64,
    /// `None` is a cleared swipe, kept so the reset wins the merge instead of
    /// the old vote coming back.
    pub yes: Option<bool>,
    /// Swipes in a row with the same verdict.
    pub streak: u32,
    /// Deck position at which the name is shown again.
    pub due: u32,
    /// +1 per right swipe, -1 per left; a reset clears it.
    pub score: i32,
}

/// Names and every device's votes. Each device only writes its own votes,
/// so merging is a union with the newest vote per (device, name).
#[derive(Default, Debug, PartialEq)]
pub struct Store {
    pub me: u32,
    /// Cards this device has swiped; local, not synced.
    pub cards: u32,
    /// UI in German; local, not synced.
    pub german: bool,
    /// When the data was last reset everywhere, millis. Data from an older
    /// epoch is ignored and a newer one wipes ours, so a reset reaches both phones.
    pub epoch: u64,
    pub names: BTreeSet<String>,
    pub votes: BTreeMap<(u32, String), Vote>,
}

impl Store {
    /// The bundled names, added on every start so new ones reach old installs.
    pub fn seed(&mut self) {
        for n in include_str!("../assets/seed.txt").lines() {
            self.add(n);
        }
    }

    /// Forgets every name and vote on both phones, keeping the bundled names.
    pub fn reset(&mut self) {
        self.wipe(now().max(self.epoch + 1));
    }

    fn wipe(&mut self, epoch: u64) {
        self.names.clear();
        self.votes.clear();
        self.cards = 0;
        self.epoch = epoch;
        self.seed();
    }

    pub fn add(&mut self, name: &str) {
        let name = name.trim().replace(['\t', '\n', '\r'], " ");
        let key = fold(&name);
        if !name.is_empty() && !self.names.iter().any(|n| fold(n) == key) {
            self.names.insert(name);
        }
    }

    /// A swipe (`Some`) schedules the name again by its streak; `None` clears it
    /// so it comes back as if new.
    pub fn vote(&mut self, name: &str, yes: Option<bool>) {
        let now = now();
        let old = self
            .votes
            .get(&(self.me, name.into()))
            .copied()
            .unwrap_or_default();
        let mut v = Vote {
            ts: now.max(old.ts + 1),
            yes,
            ..Vote::default()
        };
        if let Some(y) = yes {
            v.score = old.score + if y { 1 } else { -1 };
            v.streak = if old.yes == yes { old.streak + 1 } else { 1 };
            v.due = self.cards + v.streak * if y { LIKE_EVERY } else { NOPE_EVERY };
            self.cards += 1;
        }
        self.votes.insert((self.me, name.into()), v);
    }

    pub fn mine(&self, name: &str) -> Option<bool> {
        self.votes.get(&(self.me, name.into())).and_then(|v| v.yes)
    }

    /// Repeats that are due first, then new names in this device's shuffled
    /// order; with neither, the next repeat.
    pub fn next(&self) -> Option<String> {
        let shuffled =
            |n: &str| BuildHasherDefault::<DefaultHasher>::default().hash_one((self.me, n));
        self.names
            .iter()
            .min_by_key(|n| match self.votes.get(&(self.me, n.to_string())) {
                Some(v) if v.yes.is_some() => (v.due.max(self.cards), false, shuffled(n)),
                _ => (self.cards, true, shuffled(n)),
            })
            .cloned()
    }

    pub fn score(&self, device: u32, name: &str) -> i32 {
        self.votes
            .get(&(device, name.into()))
            .map_or(0, |v| v.score)
    }

    /// The other device that voted most recently; a reinstalled phone gets a
    /// new id, so older ones are ignored.
    pub fn partner(&self) -> Option<u32> {
        self.votes
            .iter()
            .filter(|((d, _), _)| *d != self.me)
            .max_by_key(|(_, v)| v.ts)
            .map(|((d, _), _)| *d)
    }

    /// Names both partners scored above zero, as (name, pink, blue) by
    /// highest combined score. The lower device id is pink on both phones.
    pub fn agreed(&self) -> Vec<(String, i32, i32)> {
        let Some(p) = self.partner() else {
            return vec![];
        };
        let (pink, blue) = (self.me.min(p), self.me.max(p));
        let mut out: Vec<_> = self
            .names
            .iter()
            .map(|n| (n.clone(), self.score(pink, n), self.score(blue, n)))
            .filter(|(_, a, b)| *a > 0 && *b > 0)
            .collect();
        out.sort_by_key(|(n, a, b)| (-(a + b), n.clone()));
        out
    }

    /// Returns whether anything changed.
    pub fn merge(&mut self, other: Store) -> bool {
        if other.epoch < self.epoch {
            return false;
        }
        let mut changed = other.epoch > self.epoch;
        if changed {
            self.wipe(other.epoch);
        }
        for n in other.names {
            changed |= self.names.insert(n);
        }
        for (k, v) in other.votes {
            if self.votes.get(&k).is_none_or(|old| old.ts < v.ts) {
                self.names.insert(k.1.clone());
                self.votes.insert(k, v);
                changed = true;
            }
        }
        changed
    }

    pub fn encode(&self) -> String {
        let lang = if self.german { "de" } else { "en" };
        let mut s = format!(
            "I\t{}\nC\t{}\nE\t{}\nL\t{lang}\n",
            self.me, self.cards, self.epoch
        );
        for n in &self.names {
            s += &format!("N\t{n}\n");
        }
        for ((d, n), v) in &self.votes {
            let yes = match v.yes {
                Some(true) => '+',
                Some(false) => '-',
                None => '0',
            };
            s += &format!(
                "V\t{d}\t{}\t{yes}\t{}\t{}\t{}\t{n}\n",
                v.ts, v.streak, v.due, v.score
            );
        }
        s
    }

    /// Lenient: malformed lines are skipped, since packets come from the network.
    pub fn decode(s: &str) -> Store {
        let mut st = Store::default();
        for line in s.lines() {
            let f: Vec<&str> = line.splitn(8, '\t').collect();
            let (d, ts, yes, streak, due, score, n) = match f[..] {
                ["I", id] => {
                    st.me = id.parse().unwrap_or(0);
                    continue;
                }
                ["C", c] => {
                    st.cards = c.parse().unwrap_or(0);
                    continue;
                }
                ["E", e] => {
                    st.epoch = e.parse().unwrap_or(0);
                    continue;
                }
                ["L", l] => {
                    st.german = l == "de";
                    continue;
                }
                ["N", n] => {
                    st.add(n);
                    continue;
                }
                ["V", d, ts, yes, streak, due, score, n] => (d, ts, yes, streak, due, score, n),
                // Older saves: one swipe's worth of score, due right away.
                ["V", d, ts, yes, streak, due, n] => (d, ts, yes, streak, due, yes, n),
                ["V", d, ts, yes, n] => (d, ts, yes, "1", "0", yes, n),
                _ => continue,
            };
            let yes = match yes {
                "+" => Some(true),
                "-" => Some(false),
                _ => None,
            };
            let score = match score {
                "+" => Ok(1),
                "-" => Ok(-1),
                "0" => Ok(0),
                s => s.parse(),
            };
            if let (Ok(d), Ok(ts), Ok(streak), Ok(due), Ok(score)) =
                (d.parse(), ts.parse(), streak.parse(), due.parse(), score)
            {
                st.names.insert(n.into());
                st.votes.insert(
                    (d, n.into()),
                    Vote {
                        ts,
                        yes,
                        streak,
                        due,
                        score,
                    },
                );
            }
        }
        st
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Lowercase without accents, so "Lucía" and "lucia" count as one name.
fn fold(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .collect()
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
    let mut store = std::fs::read_to_string(path)
        .map(|s| Store::decode(&s))
        .unwrap_or_default();
    store.seed();
    if store.me == 0 {
        store.me = (RandomState::new().hash_one(SystemTime::now()) as u32).max(1);
    }
    Arc::new(Mutex::new(Shared {
        store,
        peer: false,
        path: path.into(),
    }))
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
        let mut a = Store {
            me: 1,
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            ..Default::default()
        };
        a.add("Ana");
        a.add("Luis");
        b.merge(Store::decode(&a.encode()));
        assert_eq!(b.names.len(), 2);

        a.vote("Ana", Some(true));
        a.vote("Luis", Some(false));
        b.vote("Ana", Some(true));
        b.vote("Luis", Some(true));
        b.add("Eva");
        assert!(a.merge(Store::decode(&b.encode())));
        assert!(b.merge(Store::decode(&a.encode())));
        assert!(!b.merge(Store::decode(&a.encode())));

        let row = |n: &str, pink, blue| (n.to_string(), pink, blue);
        assert_eq!(a.agreed(), [row("Ana", 1, 1)]);
        assert_eq!(a.mine("Luis"), Some(false));
        assert_eq!(a.next().as_deref(), Some("Eva"));
        // Scores add up: -1 +1 +1 is a net like; the higher total sorts first.
        a.vote("Luis", Some(true));
        a.vote("Luis", Some(true));
        b.vote("Luis", Some(true));
        b.merge(Store::decode(&a.encode()));
        assert_eq!(b.agreed(), [row("Luis", 1, 2), row("Ana", 1, 1)]);
        // A reset swipe puts the name back in the deck on both sides of a merge.
        a.vote("Ana", None);
        b.merge(Store::decode(&a.encode()));
        a.merge(Store::decode(&b.encode()));
        assert_eq!(a.mine("Ana"), None);
        assert_eq!(b.agreed(), [row("Luis", 1, 2)]);
        assert_eq!(Store::decode(&b.encode()), b);
    }

    #[test]
    fn swiped_names_come_back_later_each_time() {
        let mut s = Store {
            me: 1,
            ..Default::default()
        };
        s.add("x");
        let mut gaps = vec![];
        for yes in [false, false, false, true, true, true, false] {
            let at = s.cards;
            s.vote("x", Some(yes));
            gaps.push(s.votes[&(1, "x".into())].due - at);
        }
        // Discards 20, 40, 60; a change of mind restarts the streak.
        assert_eq!(gaps, [20, 40, 60, 10, 20, 30, 20]);

        let mut s = Store {
            me: 1,
            ..Default::default()
        };
        s.add("a");
        s.add("b");
        s.vote("a", Some(true));
        s.vote("b", Some(true));
        // Nothing due yet, so the earliest repeat comes forward.
        assert_eq!(s.next().as_deref(), Some("a"));
    }

    #[test]
    fn phones_shuffle_differently_and_spellings_dedupe() {
        let deck = |me| {
            let mut s = Store {
                me,
                ..Default::default()
            };
            s.seed();
            (0..10)
                .map(|_| {
                    let n = s.next().unwrap();
                    s.vote(&n, Some(false));
                    n
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(deck(1), deck(1));
        assert_ne!(deck(1), deck(2));
        assert!(deck(1).windows(2).any(|w| w[0] > w[1]), "not alphabetical");

        let mut s = Store::default();
        s.add("Lucía");
        s.add("lucia");
        s.add("LUCIA ");
        assert_eq!(s.names.len(), 1);
    }

    #[test]
    fn reset_reaches_both_phones_and_old_data_is_ignored() {
        let mut a = Store {
            me: 1,
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            ..Default::default()
        };
        a.add("Zzyzx");
        a.vote("Zzyzx", Some(true));
        b.merge(Store::decode(&a.encode()));
        let before = Store::decode(&b.encode());

        b.reset();
        assert!(!b.names.contains("Zzyzx") && b.votes.is_empty());
        assert!(b.names.contains("Emma"), "bundled names come back");
        // A's stale broadcast doesn't bring the old votes back to B...
        assert!(!b.merge(before));
        assert!(b.votes.is_empty());
        // ...and B's newer epoch wipes A.
        assert!(a.merge(Store::decode(&b.encode())));
        assert!(!a.names.contains("Zzyzx") && a.votes.is_empty());
        assert_eq!(a.epoch, b.epoch);
    }

    #[test]
    fn whole_state_fits_one_datagram() {
        // Every bundled name swiped by both phones, plus room for added names.
        let mut s = Store {
            me: u32::MAX,
            epoch: u64::MAX / 2,
            cards: u32::MAX,
            ..Default::default()
        };
        s.seed();
        for i in 0..100 {
            s.add(&format!("Added name {i}"));
        }
        let names: Vec<_> = s.names.iter().cloned().collect();
        for n in names {
            for d in [u32::MAX, u32::MAX - 1] {
                let v = Vote {
                    ts: u64::MAX / 2,
                    yes: Some(true),
                    streak: 999,
                    due: u32::MAX,
                    score: -999,
                };
                s.votes.insert((d, n.clone()), v);
            }
        }
        let size = s.encode().len();
        assert!(
            size < 60_000,
            "{size} bytes; sync sends the whole state in one UDP datagram"
        );
    }
}
