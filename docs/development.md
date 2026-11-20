# Development

## Desktop

```sh
cargo run                  # saves to ./names.txt
cargo run -- other.txt     # another data file
cargo test
cargo clippy --all-targets
```

Two copies on one machine can't sync: both bind UDP port 47474, so the second
one runs without sync. Use a phone or the emulator for the other side.

## Android

Needs the Android SDK with platform `android-35` and an NDK, plus `cargo-apk`
(see dfui's README for the one-time setup).

```sh
export ANDROID_HOME=/opt/android-sdk NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo apk build --lib --target x86_64-linux-android    # emulator
cargo apk build --lib --release                         # phone (arm64)
adb install -r target/release/apk/names.apk
```

`--lib` is needed because the crate also has a desktop binary. Release builds
need a keystore; for local builds the debug one works:

```sh
export CARGO_APK_RELEASE_KEYSTORE=$HOME/.android/debug.keystore
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
```

Android only installs an update signed with the same key, so switching between
a locally built APK and a released one needs an uninstall, which deletes the
app's data.

### Testing sync with the emulator

The emulator sits behind its own NAT and doesn't see Wi-Fi broadcasts. Forward
the port and send it state from the host:

```sh
adb emu redir add udp:47474:47474
# send datagrams in the wire format (architecture.md) to 127.0.0.1:47474
adb emu redir del udp:47474
```

The app's data file is readable with
`adb exec-out run-as dev.dfui.names cat files/names.txt`.

## Icons

- `icons/app_foreground.svg`, `icons/app_legacy.svg`: the launcher icon.
  After editing, run `icons/render.sh` (needs `rsvg-convert`) to regenerate
  the PNGs in `res/`.
- In-app icons are converted ahead of time with dfui's `svg2path`:
  `cargo run -p svg2path -- icons/<name>.svg assets/icons/` from a dfui
  checkout. The heart, menu and arrow come from
  [Lucide](https://lucide.dev) (ISC license).
- Card motifs live in `icons/motifs/` and convert the same way into
  `assets/icons/motifs/`; they come from
  [Material Icons](https://fonts.google.com/icons) (Apache 2.0). A new motif
  also needs a line in `MOTIFS` in `src/wallpaper.rs`.

## Fonts

The name typefaces in `assets/fonts/` come from
[Google Fonts](https://fonts.google.com) under the SIL Open Font License, with
each license next to its font. A new one needs a line in `FONTS` in
`src/wallpaper.rs`, with a size factor so it looks as big as the others, and
must cover every letter in `assets/seed.txt`; missing letters fall back to
DejaVu Sans.

## Names

The bundled names are in `assets/seed.txt`, one per line. They are added on
every start, so new ones reach existing installs; removing a line doesn't
remove the name from phones that already have it.

## Releases

Pushing a `v*` tag builds a signed APK and attaches it to a GitHub release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Running the workflow by hand (Actions → release → Run workflow) publishes a
`build-<sha>` release instead.

The APK is signed with the key in the `ANDROID_KEYSTORE_BASE64` and
`ANDROID_KEYSTORE_PASSWORD` secrets; keep a backup of it. To create it once:

```sh
keytool -genkeypair -keystore ~/.android/names-release.jks -alias names \
  -keyalg RSA -keysize 4096 -validity 20000 -dname CN=names
base64 -w0 ~/.android/names-release.jks | gh secret set ANDROID_KEYSTORE_BASE64
gh secret set ANDROID_KEYSTORE_PASSWORD   # same password as above
```

Use the same password for the keystore and the key.
