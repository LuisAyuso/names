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
/// Ticks between sweeps of the whole /24 while the partner isn't heard.
const SWEEP_EVERY: u32 = 5;

/// Cards before a liked name comes back, doubling with each like in a row.
const LIKE_EVERY: u32 = 25;
/// Same for a discarded name.
const NOPE_EVERY: u32 = 60;
/// While new names last, only every this many cards may be a repeat.
const REPEAT_TURN: u32 = 3;
/// A name is disliked from this score down: more than two net discards.
const DISLIKED_AT: i32 = -3;
/// Duels rate names by Elo on top of the swipe score; one point is this much Elo.
const ELO_PER_POINT: f32 = 25.0;
/// The most Elo one duel moves.
const ELO_K: f32 = 32.0;
/// Duels each name gets per round on each phone before the bottom half drops out.
const DUELS_PER_ROUND: u32 = 3;

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
    /// Elo won or lost in duels; a reset clears it.
    pub elo: i32,
    pub duels: u32,
}

/// Names and every device's votes. Each device only writes its own votes,
/// so merging is a union with the newest vote per (device, name).
#[derive(Default, Debug, PartialEq)]
pub struct Store {
    pub me: u32,
    /// This device's display name; empty until set on the welcome screen.
    pub name: String,
    /// Index into the UI's colors.
    pub color: u8,
    /// The paired device, 0 until paired; only its data is merged.
    pub partner: u32,
    /// The partner's name and color as last heard; local.
    pub mate: (String, u8),
    /// This device agreed to discard the names both partners dislike.
    pub ready: bool,
    /// The partner's `ready` as last heard; local, not saved.
    pub mate_ready: bool,
    /// Where the partner was last heard; local, so a restart can reach it at once.
    pub addr: Option<Ipv4Addr>,
    /// Cards this device has swiped; local, not synced.
    pub cards: u32,
    /// UI in German; local, not synced.
    pub german: bool,
    /// When the data was last reset everywhere, millis. Data from an older
    /// epoch is ignored and a newer one wipes ours, so a reset reaches both phones.
    pub epoch: u64,
    pub names: BTreeSet<String>,
    /// Names deleted for good; synced like names, so a delete on one phone
    /// wins over the other phone's copy and over the bundled list.
    pub removed: BTreeSet<String>,
    pub votes: BTreeMap<(u32, String), Vote>,
    /// Names voted out of the duels, with the round they lost. Synced as a
    /// whole, newest `out_ts` first, so both phones settle on one elimination.
    pub out: Vec<(u32, String)>,
    pub out_ts: u64,
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
        self.removed.clear();
        self.votes.clear();
        self.out.clear();
        self.out_ts = 0;
        self.cards = 0;
        self.epoch = epoch;
        self.seed();
    }

    pub fn add(&mut self, name: &str) {
        let name = clean(name);
        let key = fold(&name);
        let taken = |set: &BTreeSet<String>| set.iter().any(|n| fold(n) == key);
        if !name.is_empty() && !taken(&self.names) && !taken(&self.removed) {
            self.names.insert(name);
        }
    }

    /// Deletes `name` and every vote on it, on both phones.
    pub fn remove(&mut self, name: &str) {
        self.names.remove(name);
        self.votes.retain(|(_, n), _| n != name);
        self.removed.insert(name.into());
        self.bring_back(name);
    }

    fn bring_back(&mut self, name: &str) {
        if self.out.iter().any(|(_, n)| n == name) {
            self.out.retain(|(_, n)| n != name);
            self.out_ts = now().max(self.out_ts + 1);
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
            let every = if y { LIKE_EVERY } else { NOPE_EVERY };
            v.due = self.cards + (every << (v.streak - 1).min(6));
            v.elo = old.elo;
            v.duels = old.duels;
            self.cards += 1;
        } else {
            self.bring_back(name);
        }
        self.votes.insert((self.me, name.into()), v);
    }

    pub fn mine(&self, name: &str) -> Option<bool> {
        self.votes.get(&(self.me, name.into())).and_then(|v| v.yes)
    }

    /// New names in this device's shuffled order, with a due repeat every
    /// `REPEAT_TURN` cards; once both run out, the next repeat.
    pub fn next(&self) -> Option<String> {
        let shuffled =
            |n: &str| BuildHasherDefault::<DefaultHasher>::default().hash_one((self.me, n));
        let repeat_turn = self.cards % REPEAT_TURN == REPEAT_TURN - 1;
        self.names
            .iter()
            .min_by_key(|n| match self.votes.get(&(self.me, n.to_string())) {
                Some(v) if v.yes.is_some() => (v.due.max(self.cards), !repeat_turn, shuffled(n)),
                _ => (self.cards, repeat_turn, shuffled(n)),
            })
            .cloned()
    }

    pub fn score(&self, device: u32, name: &str) -> i32 {
        self.votes
            .get(&(device, name.into()))
            .map_or(0, |v| v.score)
    }

    /// Swipe score plus duel results, in swipe points.
    pub fn points(&self, device: u32, name: &str) -> f32 {
        self.votes
            .get(&(device, name.into()))
            .map_or(0.0, |v| v.score as f32 + v.elo as f32 / ELO_PER_POINT)
    }

    fn duels(&self, device: u32, name: &str) -> u32 {
        self.votes
            .get(&(device, name.into()))
            .map_or(0, |v| v.duels)
    }

    /// The round being dueled, from 1.
    pub fn round(&self) -> u32 {
        self.out.iter().map(|(r, _)| r + 1).max().unwrap_or(1)
    }

    /// The agreed names still in the duels, once this device has swiped every
    /// name and at least two are agreed.
    fn pool(&self) -> Option<Vec<String>> {
        if self.names.iter().any(|n| self.mine(n).is_none()) {
            return None;
        }
        let agreed: Vec<String> = self.agreed().into_iter().map(|(n, ..)| n).collect();
        let pool: Vec<String> = agreed
            .iter()
            .filter(|n| !self.out.iter().any(|(_, o)| o == *n))
            .cloned()
            .collect();
        (agreed.len() >= 2 && !pool.is_empty()).then_some(pool)
    }

    /// Swiping is over and the duels have started.
    pub fn dueling(&self) -> bool {
        self.pool().is_some()
    }

    pub fn winner(&self) -> Option<String> {
        self.pool()
            .filter(|p| p.len() == 1)
            .map(|mut p| p.remove(0))
    }

    /// Two names to choose between: one this phone hasn't dueled enough this
    /// round, fewest duels first, and the one closest to it in points. None
    /// once this phone's round is done.
    pub fn duel(&self) -> Option<(String, String)> {
        let pool = self.pool()?;
        let need = DUELS_PER_ROUND * self.round();
        let duels = |n: &str| self.duels(self.me, n);
        let shuffled = |n: &str| {
            BuildHasherDefault::<DefaultHasher>::default().hash_one((self.me, n, duels(n)))
        };
        let a = pool
            .iter()
            .filter(|n| duels(n) < need)
            .min_by_key(|n| (duels(n), shuffled(n)))?;
        let gap = |n: &str| (self.points(self.me, n) - self.points(self.me, a)).abs();
        let b = pool
            .iter()
            .filter(|n| *n != a)
            .min_by(|x, y| gap(x).total_cmp(&gap(y)).then(duels(x).cmp(&duels(y))))?;
        // Which one sits on top varies, so position doesn't pick.
        let pair = (a.clone(), b.clone());
        Some(if shuffled(a) < shuffled(b) {
            pair
        } else {
            (pair.1, pair.0)
        })
    }

    /// Once both phones dueled every name enough this round, the bottom half
    /// by combined points drops out. Returns whether anything changed.
    pub fn eliminate(&mut self) -> bool {
        let Some(pool) = self.pool() else {
            return false;
        };
        let (round, need) = (self.round(), DUELS_PER_ROUND * self.round());
        let done =
            |n: &String| self.duels(self.me, n) >= need && self.duels(self.partner, n) >= need;
        if pool.len() < 2 || self.partner == 0 || !pool.iter().all(done) {
            return false;
        }
        // agreed() sorts best first, so the pool's tail is the bottom.
        let keep = pool.len().div_ceil(2);
        for n in &pool[keep..] {
            self.out.push((round, n.clone()));
        }
        self.out_ts = now().max(self.out_ts + 1);
        true
    }

    /// Records this device picking `winner` over `loser`: Elo moves more for an upset.
    pub fn pick(&mut self, winner: &str, loser: &str) {
        let gap = (self.points(self.me, loser) - self.points(self.me, winner)) * ELO_PER_POINT;
        let expected = 1.0 / (1.0 + 10f32.powf(gap / 400.0));
        let delta = (ELO_K * (1.0 - expected)).round() as i32;
        let now = now();
        for (n, d) in [(winner, delta), (loser, -delta)] {
            if let Some(v) = self.votes.get_mut(&(self.me, n.into())) {
                v.elo += d;
                v.duels += 1;
                v.ts = now.max(v.ts + 1);
            }
        }
        self.eliminate();
    }

    /// Pairs with `other` when both are free, or when it already picked us,
    /// and returns whether it is our partner, whose data may be merged. A
    /// partner that picked someone else won a race elsewhere, so we look again.
    pub fn hear(&mut self, other: &Store) -> bool {
        if other.me == 0 || other.me == self.me {
            return false;
        }
        if other.me == self.partner && other.partner != 0 && other.partner != self.me {
            self.unpair();
            return false;
        }
        let free = |s: &Store| s.partner == 0 && !s.name.is_empty();
        if free(self) && (free(other) || other.partner == self.me) {
            self.partner = other.me;
        }
        if other.me != self.partner {
            return false;
        }
        self.mate = (other.name.clone(), other.color);
        self.mate_ready = other.ready;
        true
    }

    /// Names both partners disliked, at `DISLIKED_AT` or below.
    pub fn disliked(&self) -> Vec<String> {
        if self.partner == 0 {
            return vec![];
        }
        self.names
            .iter()
            .filter(|n| {
                self.score(self.me, n) <= DISLIKED_AT && self.score(self.partner, n) <= DISLIKED_AT
            })
            .cloned()
            .collect()
    }

    /// Once both partners are ready, removes the names both dislike for good.
    /// `ready` clears when none are left, so an old agreement never discards
    /// names disliked later. Returns whether anything changed.
    pub fn settle(&mut self) -> bool {
        let gone = self.disliked();
        let both = self.ready && self.mate_ready;
        if both {
            for n in &gone {
                self.remove(n);
            }
        }
        let was = self.ready;
        if both || gone.is_empty() {
            self.ready = false;
        }
        (both && !gone.is_empty()) || was != self.ready
    }

    pub fn unpair(&mut self) {
        self.partner = 0;
        self.mate = Default::default();
        self.addr = None;
    }

    /// Names both partners swiped above zero, as (name, my points, partner's)
    /// by highest combined points.
    pub fn agreed(&self) -> Vec<(String, f32, f32)> {
        if self.partner == 0 {
            return vec![];
        }
        let mut out: Vec<_> = self
            .names
            .iter()
            .map(|n| {
                (
                    n.clone(),
                    self.points(self.me, n),
                    self.points(self.partner, n),
                )
            })
            .filter(|(n, ..)| self.score(self.me, n) > 0 && self.score(self.partner, n) > 0)
            .collect();
        out.sort_by(|(n, a, b), (m, c, d)| (c + d).total_cmp(&(a + b)).then(n.cmp(m)));
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
        // Same timestamp on both sides: the lower device id wins on both.
        if (other.out_ts, self.me) > (self.out_ts, other.me)
            && (other.out_ts, &other.out) != (self.out_ts, &self.out)
        {
            self.out = other.out;
            self.out_ts = other.out_ts;
            changed = true;
        }
        for n in other.removed {
            if !self.removed.contains(&n) {
                self.remove(&n);
                changed = true;
            }
        }
        for n in other.names {
            changed |= !self.removed.contains(&n) && self.names.insert(n);
        }
        for (k, v) in other.votes {
            if self.removed.contains(&k.1) {
                continue;
            }
            if self.votes.get(&k).is_none_or(|old| old.ts < v.ts) {
                self.names.insert(k.1.clone());
                self.votes.insert(k, v);
                changed = true;
            }
        }
        changed
    }

    /// Who this device is and whom it paired with; small enough for one
    /// packet, so it's what goes out while looking for the partner.
    pub fn hello(&self) -> String {
        format!(
            "I\t{}\nU\t{}\nK\t{}\nP\t{}\nD\t{}\n",
            self.me,
            self.name,
            self.color,
            self.partner,
            u8::from(self.ready)
        )
    }

    pub fn encode(&self) -> String {
        let lang = if self.german { "de" } else { "en" };
        let mut s = self.hello();
        s += &format!(
            "C\t{}\nE\t{}\nL\t{lang}\nM\t{}\t{}\n",
            self.cards, self.epoch, self.mate.1, self.mate.0
        );
        if let Some(a) = self.addr {
            s += &format!("A\t{a}\n");
        }
        if self.out_ts > 0 {
            s += &format!("T\t{}\n", self.out_ts);
        }
        for (r, n) in &self.out {
            s += &format!("O\t{r}\t{n}\n");
        }
        for n in &self.removed {
            s += &format!("R\t{n}\n");
        }
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
            // Its own record for the vote above, which phones without duels skip.
            if v.duels > 0 {
                s += &format!("W\t{}\t{}\n", v.elo, v.duels);
            }
        }
        s
    }

    /// Lenient: malformed lines are skipped, since packets come from the network.
    pub fn decode(s: &str) -> Store {
        let mut st = Store::default();
        let mut last = None;
        for line in s.lines() {
            let f: Vec<&str> = line.splitn(8, '\t').collect();
            let (d, ts, yes, streak, due, score, n) = match f[..] {
                ["I", id] => {
                    st.me = id.parse().unwrap_or(0);
                    continue;
                }
                ["U", n] => {
                    st.name = n.into();
                    continue;
                }
                ["K", c] => {
                    st.color = c.parse().unwrap_or(0);
                    continue;
                }
                ["D", r] => {
                    st.ready = r == "1";
                    continue;
                }
                ["P", p] => {
                    st.partner = p.parse().unwrap_or(0);
                    continue;
                }
                ["M", c, n] => {
                    st.mate = (n.into(), c.parse().unwrap_or(0));
                    continue;
                }
                ["A", a] => {
                    st.addr = a.parse().ok();
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
                ["T", t] => {
                    st.out_ts = t.parse().unwrap_or(0);
                    continue;
                }
                ["O", r, n] => {
                    if let Ok(r) = r.parse() {
                        st.out.push((r, n.into()));
                    }
                    continue;
                }
                ["R", n] => {
                    st.removed.insert(n.into());
                    continue;
                }
                ["W", elo, duels] => {
                    if let (Ok(elo), Ok(duels)) = (elo.parse(), duels.parse())
                        && let Some(v) = last.as_ref().and_then(|k| st.votes.get_mut(k))
                    {
                        v.elo = elo;
                        v.duels = duels;
                    }
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
            last = None;
            if let (Ok(d), Ok(ts), Ok(streak), Ok(due), Ok(score)) =
                (d.parse(), ts.parse(), streak.parse(), due.parse(), score)
            {
                st.names.insert(n.into());
                last = Some((d, n.to_string()));
                st.votes.insert(
                    (d, n.into()),
                    Vote {
                        ts,
                        yes,
                        streak,
                        due,
                        score,
                        ..Vote::default()
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

/// One line, no tabs, so it fits a record of the wire format.
pub fn clean(s: &str) -> String {
    s.trim().replace(['\t', '\n', '\r'], " ")
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
    if store.partner == 0 {
        // Saves from before pairing: the other device that voted last.
        store.partner = store
            .votes
            .iter()
            .filter(|((d, _), _)| *d != store.me)
            .max_by_key(|(_, v)| v.ts)
            .map_or(0, |((d, _), _)| *d);
    }
    Arc::new(Mutex::new(Shared {
        store,
        peer: false,
        path: path.into(),
    }))
}

/// Every second sends the whole state to the partner's last address and, while
/// the partner isn't heard, a hello to the whole /24; merges only the partner.
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
        let mut ticks = 0u32;
        loop {
            if sent.elapsed() >= TICK {
                sent = Instant::now();
                ticks += 1;
                let (hello, full, addr, peer) = {
                    let sh = sync.lock().unwrap();
                    (sh.store.hello(), sh.store.encode(), sh.store.addr, sh.peer)
                };
                // The hello fits one packet, so it keeps the partner marked
                // connected even when the fragmented full state gets lost.
                if let Some(a) = addr {
                    sock.send_to(hello.as_bytes(), (a, PORT)).ok();
                    sock.send_to(full.as_bytes(), (a, PORT)).ok();
                }
                if !peer {
                    for to in targets(ticks.is_multiple_of(SWEEP_EVERY)) {
                        sock.send_to(hello.as_bytes(), (to, PORT)).ok();
                    }
                }
            }
            if let Ok((n, std::net::SocketAddr::V4(from))) = sock.recv_from(&mut buf)
                && let Ok(s) = std::str::from_utf8(&buf[..n])
            {
                let other = Store::decode(s);
                let mut sh = sync.lock().unwrap();
                let st = &mut sh.store;
                let before = (st.partner, st.mate.clone(), st.addr, st.mate_ready);
                let mut changed = false;
                if st.hear(&other) {
                    seen = Some(Instant::now());
                    st.addr = Some(*from.ip());
                    changed = st.merge(other);
                    changed |= st.settle();
                    changed |= st.eliminate();
                } else if other.partner == st.me {
                    // It lost a pairing race to us; our hello tells it to look again.
                    sock.send_to(st.hello().as_bytes(), from).ok();
                }
                if changed || before != (st.partner, st.mate.clone(), st.addr, st.mate_ready) {
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

/// Limited and /24 directed broadcast; with `sweep` also every host of the
/// /24, since Wi-Fi drops broadcasts and Android filters them without a lock.
fn targets(sweep: bool) -> Vec<Ipv4Addr> {
    let mut t = vec![Ipv4Addr::BROADCAST];
    let local = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|s| s.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).map(|_| s))
        .and_then(|s| s.local_addr());
    if let Ok(std::net::SocketAddr::V4(a)) = local {
        let [a, b, c, me] = a.ip().octets();
        let hosts = if sweep { 1..=255 } else { 255..=255 };
        t.extend(
            hosts
                .filter(|&d| d != me)
                .map(|d| Ipv4Addr::new(a, b, c, d)),
        );
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
            partner: 2,
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            partner: 1,
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

        let row = |n: &str, mine, theirs| (n.to_string(), mine, theirs);
        assert_eq!(a.agreed(), [row("Ana", 1.0, 1.0)]);
        assert_eq!(a.mine("Luis"), Some(false));
        assert_eq!(a.next().as_deref(), Some("Eva"));
        // Scores add up: -1 +1 +1 is a net like; the higher total sorts first.
        a.vote("Luis", Some(true));
        a.vote("Luis", Some(true));
        b.vote("Luis", Some(true));
        b.merge(Store::decode(&a.encode()));
        assert_eq!(b.agreed(), [row("Luis", 2.0, 1.0), row("Ana", 1.0, 1.0)]);
        // A reset swipe puts the name back in the deck on both sides of a merge.
        a.vote("Ana", None);
        b.merge(Store::decode(&a.encode()));
        a.merge(Store::decode(&b.encode()));
        assert_eq!(a.mine("Ana"), None);
        assert_eq!(b.agreed(), [row("Luis", 2.0, 1.0)]);
        assert_eq!(Store::decode(&b.encode()), b);
    }

    #[test]
    fn phones_pair_once_and_ignore_others() {
        let phone = |me, name: &str| Store {
            me,
            name: name.into(),
            ..Default::default()
        };
        let (mut a, mut b, mut c) = (phone(1, "Ana"), phone(2, "Ben"), phone(3, "Cy"));
        let mut nameless = phone(4, "");
        assert!(
            !a.hear(&nameless) && !nameless.hear(&a),
            "no pairing before a name is set"
        );
        assert!(a.hear(&Store::decode(&b.hello())));
        assert!(!c.hear(&Store::decode(&a.hello())), "a is taken");
        assert!(b.hear(&Store::decode(&a.encode())), "b follows a's pick");
        assert!(!a.hear(&c) && !b.hear(&c));
        assert_eq!((a.partner, b.partner, c.partner), (2, 1, 0));
        assert_eq!(a.mate, ("Ben".into(), 0));

        a.addr = Some(Ipv4Addr::new(192, 168, 1, 7));
        a.color = 3;
        assert_eq!(Store::decode(&a.encode()), a);
        a.unpair();
        assert!(a.hear(&b), "b still picks a, so a pairs back");

        // c picked a, but a picked b meanwhile: c gives up and looks again.
        c.partner = 1;
        assert!(!c.hear(&a) && c.partner == 0);
    }

    #[test]
    fn names_both_dislike_go_once_both_agree() {
        let mut a = Store {
            me: 1,
            partner: 2,
            name: "Ana".into(),
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            partner: 1,
            name: "Ben".into(),
            ..Default::default()
        };
        for n in ["X", "Y", "Z"] {
            a.add(n);
        }
        for _ in 0..3 {
            a.vote("X", Some(false));
            a.vote("Y", Some(false));
            b.vote("X", Some(false));
        }
        a.vote("Z", Some(false));
        b.vote("Y", Some(false));
        b.vote("Y", Some(false));
        b.vote("Z", Some(false));
        b.merge(Store::decode(&a.encode()));
        a.merge(Store::decode(&b.encode()));
        assert_eq!(a.disliked(), ["X"], "Y has two discards from b, Z one each");

        a.ready = true;
        assert!(!a.settle() && a.ready, "waits for b");
        b.hear(&Store::decode(&a.hello()));
        b.ready = true;
        assert!(b.settle());
        assert!(!b.names.contains("X") && !b.ready);

        let msg = Store::decode(&b.encode());
        a.hear(&msg);
        a.merge(msg);
        assert!(a.settle());
        assert!(!a.names.contains("X") && a.names.contains("Y") && !a.ready);
    }

    #[test]
    fn duels_rank_the_agreed_names_and_sync() {
        let mut a = Store {
            me: 1,
            partner: 2,
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            partner: 1,
            ..Default::default()
        };
        for n in ["Ana", "Eva", "Zoe"] {
            a.add(n);
        }
        a.vote("Ana", Some(true));
        a.vote("Ana", Some(true));
        a.vote("Eva", Some(true));
        assert_eq!(a.duel(), None, "Zoe is still unswiped");
        a.vote("Zoe", Some(false));
        assert_eq!(a.duel(), None, "nothing agreed yet");
        for n in ["Ana", "Eva", "Zoe"] {
            b.vote(n, Some(true));
        }
        a.merge(Store::decode(&b.encode()));
        let (x, y) = a.duel().expect("Ana and Eva are agreed");
        let mut pair = [x, y];
        pair.sort();
        assert_eq!(pair, ["Ana", "Eva"]);

        // The underdog keeps winning until it leads; the duels sync over.
        for _ in 0..10 {
            a.pick("Eva", "Ana");
        }
        assert_eq!(a.agreed()[0].0, "Eva");
        assert!(
            (a.points(1, "Eva") + a.points(1, "Ana") - 3.0).abs() < 0.01,
            "zero-sum"
        );
        b.merge(Store::decode(&a.encode()));
        assert_eq!(b.points(1, "Eva"), a.points(1, "Eva"));
        assert_eq!(Store::decode(&b.encode()), b);

        // A swipe keeps the duel results, a reset clears them.
        a.vote("Eva", Some(true));
        assert_eq!(a.votes[&(1, "Eva".into())].duels, 10);
        a.vote("Eva", None);
        assert_eq!(a.points(1, "Eva"), 0.0);
    }

    #[test]
    fn rounds_halve_the_names_until_one_wins_on_both_phones() {
        let phone = |me, partner| Store {
            me,
            partner,
            ..Default::default()
        };
        let (mut a, mut b) = (phone(1, 2), phone(2, 1));
        // Swiped in this order, so the early names lead on points; both
        // phones actually prefer the last one.
        let names = ["Ana", "Bea", "Cleo", "Dana", "Eva"];
        for (i, n) in names.iter().enumerate() {
            a.add(n);
            for s in [&mut a, &mut b] {
                for _ in i..names.len() {
                    s.vote(n, Some(true));
                }
            }
        }
        let sync = |a: &mut Store, b: &mut Store| {
            for _ in 0..2 {
                b.merge(Store::decode(&a.encode()));
                b.eliminate();
                a.merge(Store::decode(&b.encode()));
                a.eliminate();
            }
        };
        sync(&mut a, &mut b);
        let favorite = |x: &str, y: &str| {
            if x > y {
                (x.to_string(), y.to_string())
            } else {
                (y.to_string(), x.to_string())
            }
        };
        let mut taps = 0;
        while a.winner().is_none() {
            // Phone a finishes its round first and then waits for b.
            for s in [&mut a, &mut b] {
                while let Some((x, y)) = s.duel() {
                    let (w, l) = favorite(&x, &y);
                    s.pick(&w, &l);
                    taps += 1;
                }
            }
            sync(&mut a, &mut b);
            assert!(taps < 200, "rounds never end");
        }
        assert_eq!(a.winner().as_deref(), Some("Eva"));
        assert_eq!(b.winner(), a.winner());
        assert_eq!(a.round(), 4, "5 -> 3 -> 2 -> 1");
        assert_eq!(a.out, b.out);
        assert_eq!(Store::decode(&a.encode()), a);

        // A loser whose score is reset comes back into the duels.
        a.vote("Ana", None);
        a.vote("Ana", Some(true));
        assert!(a.out.iter().all(|(_, n)| n != "Ana"));
        assert_eq!(a.winner(), None);
    }

    #[test]
    fn simultaneous_eliminations_settle_on_one() {
        let mut a = Store {
            me: 1,
            out: vec![(1, "X".into())],
            out_ts: 5,
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            out: vec![(1, "Y".into())],
            out_ts: 5,
            ..Default::default()
        };
        let (from_a, from_b) = (Store::decode(&a.encode()), Store::decode(&b.encode()));
        assert!(!a.merge(from_b));
        assert!(b.merge(from_a));
        assert_eq!(a.out, b.out);
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
        // Discards 60, 120, 240; a change of mind restarts the streak.
        assert_eq!(gaps, [60, 120, 240, 25, 50, 100, 60]);

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
    fn new_names_outnumber_repeats_and_every_name_comes_up() {
        let mut s = Store {
            me: 7,
            ..Default::default()
        };
        s.seed();
        let total = s.names.len();
        let mut seen = BTreeSet::new();
        let mut repeats = 0;
        for i in 0..total {
            let n = s.next().unwrap();
            repeats += usize::from(!seen.insert(n.clone()));
            s.vote(&n, Some(i % 3 == 0));
        }
        assert!(repeats * 3 <= total, "{repeats} repeats in {total} cards");
        while seen.len() < total {
            let n = s.next().unwrap();
            seen.insert(n.clone());
            s.vote(&n, Some(false));
            assert!(s.cards < 2 * total as u32, "some names never came up");
        }
    }

    #[test]
    fn removed_names_stay_gone_on_both_phones() {
        let mut a = Store {
            me: 1,
            ..Default::default()
        };
        let mut b = Store {
            me: 2,
            ..Default::default()
        };
        a.seed();
        a.vote("Emma", Some(true));
        b.merge(Store::decode(&a.encode()));
        b.vote("Emma", Some(true));
        let stale = Store::decode(&b.encode());

        a.remove("Emma");
        assert!(
            !a.merge(stale),
            "the partner's old copy brings nothing back"
        );
        assert!(!a.names.contains("Emma") && a.score(2, "Emma") == 0);
        b.merge(Store::decode(&a.encode()));
        assert!(!b.names.contains("Emma") && b.votes.keys().all(|(_, n)| n != "Emma"));
        b.seed();
        b.add("emma");
        assert!(!b.names.contains("Emma") && !b.names.contains("emma"));
        assert_eq!(Store::decode(&b.encode()), b);

        b.reset();
        assert!(b.names.contains("Emma"), "reset brings removed names back");
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
        // Every bundled name swiped by both phones, plus room for added names,
        // and a shortlist of 50 dueled on both.
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
        for (i, n) in names.into_iter().enumerate() {
            for d in [u32::MAX, u32::MAX - 1] {
                let v = Vote {
                    ts: u64::MAX / 2,
                    yes: Some(true),
                    streak: 999,
                    due: u32::MAX,
                    score: -999,
                    elo: -999,
                    duels: if i < 50 { 9_999 } else { 0 },
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
