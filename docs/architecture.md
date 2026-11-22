# Architecture

Two phones, no server. Each phone keeps the whole state, sends it to its
paired partner every second and merges what the partner sends.

## Files

| File | Role |
|---|---|
| `src/store.rs` | `Store` (names, votes, scheduling, merge, wire format), persistence, the UDP sync thread. |
| `src/lib.rs` | The dfui UI: navigation, drawer menu, pages, the swipe `Card` and the `ScoreRow` widgets, `android_main`. |
| `src/wallpaper.rs` | Card looks: a pastel with one motif on a lattice and a typeface for the name, picked from the name. |
| `src/tr.rs` | Every UI text, in English and German. |
| `src/main.rs` | Desktop entry point. |
| `assets/seed.txt` | Bundled names. |
| `assets/icons/` | In-app icons as dfui paths. |
| `assets/fonts/` | Typefaces for the names on the cards. |
| `res/` | Android launcher icon. |

## Data

```
Store
  me      device id, random, made on first start
  name    display name, set on the welcome screen
  color   index into the UI's colors
  partner device id of the paired phone, 0 until paired
  mate    partner's name and color as last heard  (local)
  addr    partner's last IP                       (local)
  cards   cards this device has swiped          (local)
  german  UI language                           (local)
  epoch   time of the last full reset, millis
  names   set of names
  removed names deleted for good
  votes   (device, name) → Vote

Vote
  ts      millis of the last change; the newest wins a merge
  yes     last swipe: Some(true) right, Some(false) left, None reset
  streak  swipes in a row with the same verdict
  due     card number at which the name comes back
  score   +1 per right swipe, -1 per left
```

Each device only writes its own votes, so two phones never edit the same entry
and merging needs no conflict handling: union of names, newest `ts` per
(device, name). A score reset is a vote with `yes = None` rather than a
deletion, so it wins the merge instead of the partner's copy bringing the old
vote back.

A removed name is kept in `removed`, which merges by union like `names`;
merging drops it and its votes, and `add` skips it, so neither the partner's
copy nor the bundled list brings it back. Only a full reset clears it.

Apart from removals, names are only ever added. `add` skips a name that
equals an existing one ignoring case and accents ("lucia" vs "Lucía").

## Sync

`store::start` runs one thread with one UDP socket on port 47474. Every second:

- if the partner's address is known, it sends a hello and the encoded store
  there by unicast (Wi-Fi retries unicast; broadcast is sent once,
  unacknowledged, and a lost fragment loses the whole datagram). The
  one-packet hello keeps the partner marked connected when the fragmented
  store gets lost;
- while the partner isn't heard, it sends a small hello (`I`, `U`, `K`, `P`
  lines, one packet) to `255.255.255.255` and the `/24` broadcast, and every
  fifth second to each host of the `/24`, since Android drops broadcasts
  without a multicast lock.

Each received datagram goes through `Store::hear`: a phone with a name and no
partner pairs with the first one that is free too, or that already picked it.
A phone whose partner says it picked someone else
unpairs and looks again; a phone hearing from one that lost such a race
answers with its hello so it finds out. Only the partner's datagrams are merged, and the address they came from
becomes `addr`, saved so a restart reaches the partner without discovery.
Five seconds without one marks the partner gone.

The device id is random per install, so a reinstalled phone is a new partner:
"New partner" in the menu unpairs, and must be done on both phones, since a
phone that still names this one as partner pairs it back.

The whole state goes in each datagram, which caps it at about 64 KB, roughly
a thousand names with two voters. Delta sync would lift that.

Saves from before pairing take as partner the other device that voted last.

### Reset

A full reset must reach both phones: each holds the other's votes and would
send them back. `reset` sets `epoch` to now and clears names and votes (the
bundled names are added again). In `merge`, data with an older epoch is
ignored and a newer epoch wipes the local data before merging, so the reset
spreads in both directions, also to a phone that was offline.

### Wire and file format

The same text format is sent and saved (`files/names.txt` on Android), one
record per line, tab-separated:

```
I  <device id>
U  <name>
K  <color>
P  <partner id>
M  <partner color> <partner name>
A  <partner ip>
C  <cards>
E  <epoch>
L  en|de
N  <name>
V  <device> <ts> <+|-|0> <streak> <due> <score> <name>
```

Older saves with shorter `V` lines still load. Saving writes a temp file and
renames it, so a crash can't leave a half-written file.

## Deck order

`Store::next` picks the name with the smallest key:

- a swiped name: `(max(due, cards), repeat)`
- an unswiped or reset name: `(cards, new)`

so repeats that are due come first, then new names, and when neither is due the
nearest repeat comes early: the deck never runs out. Ties break on a hash of
(device id, name), which shuffles the deck differently on each phone but
stably across restarts.

A swipe sets `due = cards + streak × 10` for a like and `× 20` for a nope, so
a name liked again and again comes back after 10, 20, 30… cards. Several names
can fall due on the same card, so the gaps are minimums.

## UI

dfui rebuilds the view from state. The root is `overlays` (for the drawer and
dialogs) around a `stateful` `Nav` (current tab, Names screen open, Agreed
revealed), around a `watch` on the store's `Notifier`. Any store change, local
or from the network, rebuilds the screen.

- **Swipe**: the `Card` widget follows the finger and reports a swipe past
  30% of its width; it's keyed by name so each new card starts undragged.
- **Agreed**: `Store::agreed` returns names both partners scored above zero
  with each one's score, painted in each partner's color.
- **Discard**: Reveal first lists the names both partners scored −3 or
  lower (`Store::disliked`). Each phone's agreement is the `ready` flag in its
  hello (`D` line); `Store::settle` removes those names once both flags are
  set, and clears the flag when none are left, so an old agreement never
  removes names disliked later. Keep skips to the agreed names.
- **Welcome**: shown until a name is set and a partner paired.
  `ScoreRow` paints the two bands. Reveal is part of `Nav`, so any
  navigation hides the list again.
- **Names**: a scroll of every name; tapping a swiped one asks before
  resetting its score.
- **Menu**: a dfui `drawer` with Names, the language switch and the full
  reset behind a confirmation. Dialogs are built here rather than with dfui's
  `confirm` so their buttons are translated.

Every scroll needs `.fill()` on each flex box above it (not `.grow`), or the
box grows to the content and the scroll never scrolls.
