// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{fs::write, path::PathBuf};

use anyhow::{Context, Result};
use semver::Version;
use tauri_utils::{config::Config, write_if_changed};

use crate::is_dev;

pub fn generate_gradle_files(project_dir:PathBuf, config:&Config) -> Result<()> {
	let gradle_settings_path = project_dir.join("tauri.settings.gradle");

	let app_build_gradle_path = project_dir.join("app").join("tauri.build.gradle.kts");

	let app_tauri_properties_path = project_dir.join("app").join("tauri.properties");

	let mut gradle_settings =
		"// THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n".to_string();

	let mut app_build_gradle = "// THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
val implementation by configurations
dependencies {"
		.to_string();

	let mut app_tauri_properties = Vec::new();

	for (env, value) in std::env::vars_os() {
		let env = env.to_string_lossy();

		if env.starts_with("DEP_") && env.ends_with("_ANDROID_LIBRARY_PATH") {
			let name_len = env.len() - "DEP_".len() - "_ANDROID_LIBRARY_PATH".len();

			let mut plugin_name = env
				.chars()
				.skip("DEP_".len())
				.take(name_len)
				.collect::<String>()
				.to_lowercase()
				.replace('_', "-");

			if plugin_name == "tauri" {
				plugin_name = "tauri-android".into();
			}

			let plugin_path = PathBuf::from(value);

			gradle_settings.push_str(&format!("include ':{plugin_name}'"));

			gradle_settings.push('\n');

			gradle_settings.push_str(&format!(
				"project(':{plugin_name}').projectDir = new File({:?})",
				tauri_utils::display_path(plugin_path)
			));

			gradle_settings.push('\n');

			app_build_gradle.push('\n');

			app_build_gradle.push_str(&format!(r#"  implementation(project(":{plugin_name}"))"#));
		}
	}

	app_build_gradle.push_str("\n}");

	if let Some(version) = config.version.as_ref() {
		app_tauri_properties.push(format!("tauri.android.versionName={version}"));

		if let Some(version_code) = config.bundle.android.version_code.as_ref() {
			app_tauri_properties.push(format!("tauri.android.versionCode={version_code}"));
		} else if let Ok(version) = Version::parse(version) {
			let mut version_code = version.major * 1000000 + version.minor * 1000 + version.patch;

			if is_dev() {
				version_code = version_code.clamp(1, 2100000000);
			}

			if version_code == 0 {
				return Err(anyhow::anyhow!(
					"You must change the `version` in `tauri.conf.json`. The default value \
					 `0.0.0` is not allowed for Android package and must be at least `0.0.1`."
				));
			} else if version_code > 2100000000 {
				return Err(anyhow::anyhow!(
					"Invalid version code {}. Version code must be between 1 and 2100000000. You \
					 must change the `version` in `tauri.conf.json`.",
					version_code
				));
			}

			app_tauri_properties.push(format!("tauri.android.versionCode={version_code}"));
		}
	}

	// Overwrite only if changed to not trigger rebuilds
	write_if_changed(&gradle_settings_path, gradle_settings)
		.context("failed to write tauri.settings.gradle")?;

	write_if_changed(&app_build_gradle_path, app_build_gradle)
		.context("failed to write tauri.build.gradle.kts")?;

	if !app_tauri_properties.is_empty() {
		let app_tauri_properties_content = format!(
			"// THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n{}",
			app_tauri_properties.join("\n")
		);

		if std::fs::read_to_string(&app_tauri_properties_path)
			.map(|o| o != app_tauri_properties_content)
			.unwrap_or(true)
		{
			write(&app_tauri_properties_path, app_tauri_properties_content)
				.context("failed to write tauri.properties")?;
		}
	}

	println!("cargo:rerun-if-changed={}", gradle_settings_path.display());

	println!("cargo:rerun-if-changed={}", app_build_gradle_path.display());

	if !app_tauri_properties.is_empty() {
		println!("cargo:rerun-if-changed={}", app_tauri_properties_path.display());
	}

	Ok(())
}
