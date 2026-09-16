plugins {
    kotlin("multiplatform")
    id("com.android.library")
}

kotlin {
    jvmToolchain(21)
    androidTarget()
    jvm()
    iosArm64()
    iosSimulatorArm64()

    sourceSets {
        commonMain.dependencies {
            implementation("net.java.dev.jna:jna:5.15.0") // uniffi's Kotlin (JVM/Android) target loads native libs through JNA
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
        }
        jvmTest.dependencies {
            implementation(kotlin("test"))
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
        }
    }
}

android {
    namespace = "blue.rae.spirit.client"
    compileSdk = 37
    defaultConfig { minSdk = 26 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
}

// `jvm-native` (in ../devenv.nix) builds the cdylib straight into the
// Cargo workspace's own target/release — point JNA there directly rather
// than copying it into a resources dir that would just go stale.
val cargoTargetDir = rootDir.resolve("../target/release")

tasks.withType<Test>().configureEach {
    systemProperty("jna.library.path", cargoTargetDir.absolutePath)
}

