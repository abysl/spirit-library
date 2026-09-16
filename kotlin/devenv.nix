{
  pkgs,
  lib,
  ...
}: {
  # Isolated on purpose, same reasoning as kai/android/devenv.nix: the
  # Android SDK/NDK and Gradle are heavy and partly unfree, and iOS builds
  # need Xcode, which nixpkgs cannot package at all. None of that belongs in
  # spirit's main devenv (../devenv.nix) — enter this shell only when you're
  # actually building the Kotlin side. devenv.yaml's `allowUnfree: true`
  # (same as kai/android/devenv.yaml) is required for the Android SDK's
  # cmdline-tools license.
  cachix.enable = false;

  android = {
    enable = true;
    platforms.version = ["34" "36" "37"];
    buildTools.version = ["34.0.0" "36.0.0"];
    ndk.enable = true;
    ndk.version = ["26.3.11579264"]; # matches kai/android/devenv.nix
  };

  languages.rust = {
    enable = true;
    channel = "stable";
    # aarch64-apple-ios* only actually link on a macOS host with Xcode's SDK
    # installed; harmless to declare here, but `ios-native` will only work
    # on such a host.
    targets = [
      "aarch64-linux-android"
      "x86_64-linux-android"
      "aarch64-apple-ios"
      "aarch64-apple-ios-sim"
    ];
  };

  packages = with pkgs; [
    cargo-ndk
    jdk21
    jdk25
  ];

  enterShell = ''
    # Some Gradle/AGP versions still look for ANDROID_SDK_ROOT specifically;
    # devenv's android module only guarantees ANDROID_HOME.
    export ANDROID_SDK_ROOT="$ANDROID_HOME"
    export JAVA25_HOME="${pkgs.jdk25.home}"
    export JAVA21_HOME="${pkgs.jdk21.home}"
    export JAVA_HOME="$JAVA25_HOME"
    echo "spirit kotlin dev env — jdk 25 daemon, jdk 21 toolchain, gradle $(sed -n 's/.*gradle-\(.*\)-bin.zip/\1/p' "$DEVENV_ROOT/gradle/wrapper/gradle-wrapper.properties")"
  '';

  scripts."generate-bindings".exec = ''
    set -eu
    cd "$DEVENV_ROOT/.."
    cargo build -p spirit-client-ffi --bin uniffi-bindgen --features uniffi/cli
    lib="target/debug/libspirit_client_ffi.so"
    [ -f "$lib" ] || lib="target/debug/libspirit_client_ffi.dylib"
    ./target/debug/uniffi-bindgen generate --library "$lib" --language kotlin \
      --out-dir kotlin/spirit-client/src/commonMain/kotlin --no-format
    echo "generated bindings under kotlin/spirit-client/src/commonMain/kotlin/uniffi/spirit_client_ffi/"
  '';

  scripts."jvm-native".exec = ''
    set -eu
    cd "$DEVENV_ROOT/.."
    cargo build --release -p spirit-client-ffi
    echo "built target/release/libspirit_client_ffi.{so,dylib,dll}"
    echo "spirit-client's build.gradle.kts points jna.library.path at target/release directly — nothing to copy for local dev/test."
  '';

  scripts."android-native".exec = ''
    set -eu
    export ANDROID_NDK_HOME=$ANDROID_NDK_ROOT
    cd "$DEVENV_ROOT/.."
    cargo ndk -t arm64-v8a -t x86_64 -P 26 \
      -o kotlin/spirit-client/src/androidMain/jniLibs \
      build --release -p spirit-client-ffi
  '';

  scripts."ios-native".exec = ''
    set -eu
    cd "$DEVENV_ROOT/.."
    cargo build --release --target aarch64-apple-ios -p spirit-client-ffi
    cargo build --release --target aarch64-apple-ios-sim -p spirit-client-ffi
    echo "build the two static libs above into an .xcframework with xcodebuild -create-xcframework (macOS + Xcode only)"
  '';

  scripts."ide-gradle-props".exec = ''
        set -eu
        mkdir -p "$HOME/.gradle"
        cat > "$HOME/.gradle/gradle.properties" <<EOF
    org.gradle.java.installations.paths=$JAVA21_HOME,$JAVA25_HOME
    android.aapt2FromMavenOverride=$ANDROID_HOME/build-tools/36.0.0/aapt2
    EOF
        echo "wrote $HOME/.gradle/gradle.properties for IntelliJ:"
        cat "$HOME/.gradle/gradle.properties"
  '';

  scripts."assemble".exec = ''
    set -eu
    generate-bindings
    jvm-native
    android-native
    "$DEVENV_ROOT"/gradlew -p "$DEVENV_ROOT" build
  '';
}
