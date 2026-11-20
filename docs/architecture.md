# Architecture

Two phones, no server. Each phone keeps the whole state, broadcasts it on the
local network every second and merges whatever it hears.

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
  cards   cards this device has swiped          (local)
  german  UI language                           (local)
  epoch   time of the last full reset, millis
  names   set of names
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

Names are only ever added. `add` skips a name that equals an existing one
ignoring case and accents ("lucia" vs "Lucía").

## Sync

`store::start` runs one thread with one UDP socket on port 47474:

- every second it sends the encoded store to `255.255.255.255` and to the
  `/24` broadcast of the local address (some routers and phones drop one of
  them);
- it merges every datagram it receives, saves and wakes the UI through a
  dfui `Notifier` when something changed;
- a datagram from another device id marks the partner as connected; five
  seconds without one marks it gone.

The whole state goes in each datagram, which caps it at about 64 KB, roughly
a thousand names with two voters. Delta sync would lift that.

The partner is the other device that voted most recently, so ids left by a
reinstalled phone stop counting.

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
  with each one's score; the lower device id is pink on both phones.
  `ScoreRow` paints the two bands. Reveal is part of `Nav`, so any
  navigation hides the list again.
- **Names**: a scroll of every name; tapping a swiped one asks before
  resetting its score.
- **Menu**: a dfui `drawer` with Names, the language switch and the full
  reset behind a confirmation. Dialogs are built here rather than with dfui's
  `confirm` so their buttons are translated.

Every scroll needs `.fill()` on each flex box above it (not `.grow`), or the
box grows to the content and the scroll never scrolls.
