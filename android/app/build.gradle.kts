plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val syncSharedPayloads by tasks.registering(Sync::class) {
    val rootResources = file("../../resources")
    val heroArt = file("../../src/ui/hero.webp")
    val genDir = layout.buildDirectory.dir("generated/sharedAssets")
    into(genDir)
    from(rootResources) {
        include(
            "mmap_key_4096.bin",
            "vstring_en.bin.gz",
            "vstring_id.bin.gz",
            "static_ui_en.bin.gz",
            "static_ui_id.bin.gz",
            "news_en.bin.gz",
            "news_id.bin.gz",
        )
        // Rename .gz -> .gzp so AAPT2 does not auto-gunzip & strip extensions
        rename { name -> if (name.endsWith(".gz")) name.removeSuffix(".gz") + ".gzp" else name }
        into("payloads")
    }
    from(heroArt) {
        into("ui")
    }
}

android {
    namespace = "com.ruzalang.patcher"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.ruzalang.patcher"
        minSdk = 26
        targetSdk = 35
        versionCode = 901
        versionName = "0.9.1"
    }

    buildFeatures {
        aidl = true
        buildConfig = true
    }

    sourceSets {
        getByName("main") {
            assets.srcDir(layout.buildDirectory.dir("generated/sharedAssets"))
        }
    }

    signingConfigs {
        val ksPath = System.getenv("ANDROID_KEYSTORE_PATH")
        if (!ksPath.isNullOrBlank() && file(ksPath).isFile) {
            create("release") {
                storeFile = file(ksPath)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD") ?: ""
                keyAlias = System.getenv("ANDROID_KEY_ALIAS") ?: "ruzalang"
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD") ?: ""
                enableV1Signing = true
                enableV2Signing = true
                enableV3Signing = true
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            isShrinkResources = false
            val relSign = signingConfigs.findByName("release")
            signingConfig = relSign ?: signingConfigs.getByName("debug")
        }
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    androidResources {
        noCompress += listOf("gzp", "bin", "webp")
    }

    testOptions {
        unitTests.all {
            it.maxHeapSize = "2048m"
        }
    }
}

tasks.named("preBuild").configure {
    dependsOn(syncSharedPayloads)
}

dependencies {
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.documentfile:documentfile:1.0.1")
    implementation("androidx.webkit:webkit:1.12.1")
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")

    testImplementation("junit:junit:4.13.2")
}
