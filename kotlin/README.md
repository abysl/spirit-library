# Kotlin bindings and demo

Audience: Kotlin/JVM or Android developers contributing to Spirit's bindings.
Read the [Spirit introduction](../README.md) and
[integration guide](../wiki/api/overview.md) first.

The Rust `spirit-client-ffi` crate exposes operations through UniFFI, a tool
that generates foreign-language bindings. This directory contains the Kotlin
wrapper and a demo application. It is experimental; do not assume the declared
iOS targets have a working, verified implementation.

## Start with Rust

From the repository root:

```sh
cargo build --locked -p spirit-client-ffi
cargo test --locked -p spirit-client
```

The public Rust client defines the behavior. Kotlin calls must preserve the
same store ownership, trust, and shutdown rules.

## Kotlin toolchain

The separate `kotlin/devenv.nix` provides Android tools and JDKs. It does not
make Xcode available on Linux. iOS work requires a macOS host and Xcode as well
as compatible bindings.

The Gradle wrapper JAR is not tracked in this extraction. Running `./gradlew`
before restoring a verified wrapper will fail. Use an installed compatible
Gradle, or regenerate the wrapper from a trusted Gradle distribution matching
`gradle/wrapper/gradle-wrapper.properties`.

From the Kotlin environment, `generate-bindings` generates bindings and
`jvm-native` builds the native library for JVM tests. Then run:

```sh
gradle :spirit-client:jvmTest
```

Do not treat successful Rust compilation as proof that bindings generated,
linked, or passed JVM tests. The current generator's Kotlin output and the
multiplatform source layout still need platform-specific verification.

## Android and iOS

`android-native` builds the Rust library for Android; the demo still needs
a Gradle build and an emulator/device test. Keep debug signing material local.

For iOS, validate the generator and native linking before claiming support.
A declared target or static library alone is not an application test.
