// AGP 9 compiles Kotlin itself (built-in Kotlin): no kotlin-android plugin.
plugins {
    id("com.android.library")
}

android {
    namespace = "dev.glot.ota"
    compileSdk = 35
    defaultConfig { minSdk = 24 }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
}
