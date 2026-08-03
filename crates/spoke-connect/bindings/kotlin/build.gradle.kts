plugins {
    kotlin("jvm") version "1.9.25"
    `maven-publish`
}

group = "io.github.42ch-dev"
// Lockstep SemVer — asserted/bumped with tooling/release lockstep surfaces.
version = "0.8.0"

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

// JNA loads natives from classpath prefixes (e.g. darwin-aarch64/libspoke_connect.dylib).
// Assembled CI natives live under src/main/resources/ (default Gradle resources dir).
// Committed native/ is copied only when no assembled dir exists for that RID.
tasks.processResources {
    listOf("darwin-aarch64", "linux-x86-64", "win32-x86-64").forEach { rid ->
        val fromResources = file("src/main/resources/$rid")
        val fromNative = file("native/$rid")
        if (!fromResources.isDirectory && fromNative.isDirectory) {
            from(fromNative) { into(rid) }
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

publishing {
    publications {
        create<MavenPublication>("maven") {
            groupId = "io.github.42ch-dev"
            artifactId = "spoke-connect"
            from(components["java"])

            pom {
                name.set("spoke-connect")
                description.set(
                    "SPOKE Connect session-core Kotlin bindings (uniffi + JNA native spoke_connect FFI). " +
                        "Transport stays product-owned.",
                )
                url.set("https://github.com/42ch-dev/spoke")
                licenses {
                    license {
                        name.set("Apache License 2.0")
                        url.set("https://www.apache.org/licenses/LICENSE-2.0")
                    }
                }
                developers {
                    developer {
                        name.set("42ch")
                        organization.set("42ch")
                        organizationUrl.set("https://github.com/42ch-dev")
                    }
                }
                scm {
                    connection.set("scm:git:git://github.com/42ch-dev/spoke.git")
                    developerConnection.set("scm:git:ssh://github.com:42ch-dev/spoke.git")
                    url.set("https://github.com/42ch-dev/spoke")
                }
            }
        }
    }
    repositories {
        maven {
            name = "GitHubPackages"
            url = uri("https://maven.pkg.github.com/42ch-dev/spoke")
            credentials {
                username =
                    project.findProperty("gpr.user") as String?
                        ?: System.getenv("GITHUB_ACTOR")
                        ?: System.getenv("GPR_USER")
                password =
                    project.findProperty("gpr.key") as String?
                        ?: System.getenv("GITHUB_TOKEN")
                        ?: System.getenv("GPR_KEY")
            }
        }
    }
}
