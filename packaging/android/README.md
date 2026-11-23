# Android packaging

`RELEASE=1 packaging/android/build.sh` builds the release APK at `target/android/mailbench.apk`. Use JDK 21 for kotlinc (JDK 25 breaks it): `JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64`.

The debug APK is about 880 MB because of debug symbols in the Blitz and wgpu dependencies, and it does not fit the emulator storage. `build.sh` now builds debug with `CARGO_PROFILE_DEV_DEBUG=0`. Prefer the release APK for emulator checks.

Display cutout: the theme in `android/res/values/styles.xml` uses `shortEdges` and `values-v30/styles.xml` uses `always`, so the window is not letterboxed in landscape. `src/android.rs` merges the cutout safe insets with the system bar insets (`insets::merge_insets`) and re-reads them every 250 ms so rotating 180 degrees moves the inset to the other side.
