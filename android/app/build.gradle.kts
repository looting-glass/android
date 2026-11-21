plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The app version lives in ../version.txt (bumped by release-please). LG_VERSION overrides it, e.g.
// "0.2.0-dev" for CI test builds. versionCode = major*10000 + minor*100 + patch.
val appVersion = System.getenv("LG_VERSION")?.takeIf { it.isNotBlank() }
    ?: rootProject.projectDir.parentFile.resolve("version.txt").readText().trim()
val appVersionCode = appVersion.substringBefore('-').split('.').map { it.toInt() }
    .let { (major, minor, patch) -> major * 10000 + minor * 100 + patch }

android {
    namespace = "dev.lootingglass.android"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "dev.lootingglass.android"
        minSdk = 26
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersion
        // The Rust core is built for these two only (64-bit phones and the emulator); without the
        // filter, ML Kit's 32-bit libraries would let the app install on phones where it crashes.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    // Release signing: CI passes the keystore through LG_KEYSTORE_* (GitHub secrets). Without them
    // (local builds) release APKs are signed with the debug key, which phones treat as a different
    // app: an installed CI build can't be updated by a local one, and vice versa.
    val keystore = System.getenv("LG_KEYSTORE_FILE")?.let(::file)?.takeIf { it.exists() }
    signingConfigs {
        if (keystore != null) create("release") {
            storeFile = keystore
            storePassword = System.getenv("LG_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("LG_KEY_ALIAS")
            keyPassword = System.getenv("LG_KEY_PASSWORD")
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }
    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

// Rust core: build the shared library for the phone (arm64) and the emulator (x86_64) and
// regenerate the Kotlin bindings. Needs cargo + cargo-ndk; see ../README.md.
val buildRust by tasks.registering(Exec::class) {
    workingDir = rootProject.projectDir.parentFile
    commandLine("sh", "scripts/build-rust.sh")
}
tasks.named("preBuild") { dependsOn(buildRust) }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.activity:activity-compose:1.12.4")
    implementation("androidx.core:core-ktx:1.19.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.exifinterface:exifinterface:1.4.1")

    // On-device OCR, bundled Latin model (no download, works offline).
    implementation("com.google.mlkit:text-recognition:16.0.1")

    val camerax = "1.6.2"
    implementation("androidx.camera:camera-camera2:$camerax")
    implementation("androidx.camera:camera-lifecycle:$camerax")
    implementation("androidx.camera:camera-view:$camerax")

    // UniFFI's Kotlin bindings call the Rust library through JNA.
    implementation("net.java.dev.jna:jna:5.19.0@aar")
}
