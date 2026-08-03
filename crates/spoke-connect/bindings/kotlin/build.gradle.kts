plugins {
    kotlin("jvm") version "1.9.25"
}

group = "io.github.42ch-dev"
version = "0.7.1"

repositories {
    mavenCentral()
}

dependencies {
    implementation("net.java.dev.jna:jna:5.16.0")
    testImplementation(kotlin("test"))
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

kotlin {
    jvmToolchain(17)
}

sourceSets {
    main {
        kotlin {
            srcDir("generated")
        }
    }
    test {
        kotlin {
            srcDir("Smoke")
        }
    }
}

tasks.test {
    useJUnitPlatform()
    val override = nativeLibraryOverride()
    if (override != null) {
        systemProperty("uniffi.component.spoke_connect.libraryOverride", override.absolutePath)
    }
}

fun nativeLibraryOverride(): java.io.File? {
    val explicit = project.findProperty("nativeLib") as String?
    if (explicit != null) {
        return file(explicit)
    }
    val os = System.getProperty("os.name").lowercase()
    val arch = System.getProperty("os.arch").lowercase()
    val rid = when {
        os.contains("mac") && (arch == "aarch64" || arch == "arm64") -> "darwin-aarch64"
        os.contains("mac") && arch == "x86_64" -> "darwin-x86-64"
        os.contains("linux") && (arch == "amd64" || arch == "x86_64") -> "linux-x86-64"
        os.contains("win") && (arch == "amd64" || arch == "x86_64") -> "win32-x86-64"
        else -> return null
    }
    val lib = when {
        rid.startsWith("win") -> file("native/$rid/spoke_connect.dll")
        rid.startsWith("linux") -> file("native/$rid/libspoke_connect.so")
        else -> file("native/$rid/libspoke_connect.dylib")
    }
    return if (lib.isFile) lib else null
}
