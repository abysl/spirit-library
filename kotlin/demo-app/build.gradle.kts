plugins {
    kotlin("multiplatform")
    kotlin("plugin.compose")
    id("org.jetbrains.compose")
    id("com.android.application")
}

kotlin {
    jvmToolchain(21)
    androidTarget()
    jvm("desktop")
    listOf(iosArm64(), iosSimulatorArm64()).forEach { target ->
        target.binaries.framework {
            baseName = "DemoApp"
            isStatic = true
            // spirit-client-ffi's .a (from `ios-native`) links into this
            // framework's binary; wiring that up is an Xcode-project step
            // once one exists, not something Gradle alone can finish here.
        }
    }

    sourceSets {
        commonMain.dependencies {
            implementation(project(":spirit-client"))
            implementation(compose.runtime)
            implementation(compose.foundation)
            implementation(compose.material3)
            implementation(compose.ui)
            implementation(compose.components.resources)
            implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
        }
        androidMain.dependencies {
            implementation("androidx.activity:activity-compose:1.9.3")
            implementation("com.google.zxing:core:3.5.3") // QR generation, JVM/Android only — see Platform.android.kt
            implementation("com.journeyapps:zxing-android-embedded:4.3.0") // QR scanning, same lib spirit-sync and kai already use
        }
        val desktopMain by getting {
            dependencies {
                implementation(compose.desktop.currentOs)
                implementation("com.google.zxing:core:3.5.3") // QR generation, shared shape with androidMain's
            }
        }
    }
}

android {
    namespace = "blue.rae.spirit.demo"
    compileSdk = 37
    defaultConfig {
        applicationId = "blue.rae.spirit.demo"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
}

compose.desktop {
    application {
        mainClass = "blue.rae.spirit.demo.MainKt"
    }
}
