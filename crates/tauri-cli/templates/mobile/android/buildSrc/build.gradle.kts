plugins {
    `kotlin-dsl`
}

gradlePlugin {
    plugins {
        create("pluginsForCoolKids") {
            id = "rust"
            implementationClass = "RustPlugin"
        }
    }
}

repositories {
    google()
    mavenCentral()
}

dependencies {
    compileOnly(gradleApi())
    implementation("com.android.tools.build:gradle:8.5.1")
}

sourceSets {
    main {
        kotlin {
            srcDir("Source/main/kotlin") // instead of src/main/kotlin
        }
        resources {
            srcDir("Source/main/resources")
        }
    }
}
