# names

Tinder-style baby name picker for two phones. Built with
[dfui](https://github.com/LuisAyuso/dfui). Phones on the same Wi-Fi find each
other by UDP broadcast (port 47474) and sync names and swipes.

```sh
cargo run                     # desktop, saves to ./names.txt
cargo test
```

## Releases

Pushing a `v*` tag builds a signed APK and attaches it to a GitHub release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Running the workflow by hand (Actions → release → Run workflow) publishes a
`build-<sha>` release instead.

The APK is signed with the key in the `ANDROID_KEYSTORE_BASE64` and
`ANDROID_KEYSTORE_PASSWORD` secrets. Android only installs an update signed
with the same key, so keep a backup of it. To create it once:

```sh
keytool -genkeypair -keystore ~/.android/names-release.jks -alias names \
  -keyalg RSA -keysize 4096 -validity 20000 -dname CN=names
base64 -w0 ~/.android/names-release.jks | gh secret set ANDROID_KEYSTORE_BASE64
gh secret set ANDROID_KEYSTORE_PASSWORD   # same password as above
```

Use the same password for the keystore and the key.
