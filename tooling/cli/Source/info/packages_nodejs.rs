// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use colored::Colorize;
use serde::Deserialize;

use super::{env_nodejs::manager_version, SectionItem, VersionMetadata};
use crate::helpers::{cross_command, npm::PackageManager};

#[derive(Deserialize)]
struct YarnVersionInfo {
	data:Vec<String>,
}

pub fn npm_latest_version(pm:&PackageManager, name:&str) -> crate::Result<Option<String>> {
	match pm {
		PackageManager::Yarn => {
			let mut cmd = cross_command("yarn");

			let output = cmd.arg("info").arg(name).args(["version", "--json"]).output()?;
			if output.status.success() {
				let stdout = String::from_utf8_lossy(&output.stdout);
				let info:YarnVersionInfo = serde_json::from_str(&stdout)?;
				Ok(Some(info.data.last().unwrap().to_string()))
			} else {
				Ok(None)
			}
		},
		PackageManager::YarnBerry => {
			let mut cmd = cross_command("yarn");

			let output = cmd
				.arg("npm")
				.arg("info")
				.arg(name)
				.args(["--fields", "version", "--json"])
				.output()?;
			if output.status.success() {
				let info:crate::PackageJson =
					serde_json::from_reader(std::io::Cursor::new(output.stdout)).unwrap();
				Ok(info.version)
			} else {
				Ok(None)
			}
		},
		PackageManager::Npm => {
			let mut cmd = cross_command("npm");

			let output = cmd.arg("show").arg(name).arg("version").output()?;
			if output.status.success() {
				let stdout = String::from_utf8_lossy(&output.stdout);
				Ok(Some(stdout.replace('\n', "")))
			} else {
				Ok(None)
			}
		},
		PackageManager::Pnpm => {
			let mut cmd = cross_command("pnpm");

			let output = cmd.arg("info").arg(name).arg("version").output()?;
			if output.status.success() {
				let stdout = String::from_utf8_lossy(&output.stdout);
				Ok(Some(stdout.replace('\n', "")))
			} else {
				Ok(None)
			}
		},
		// Bun doesn't support `info` command
		PackageManager::Bun => {
			let mut cmd = cross_command("npm");

			let output = cmd.arg("show").arg(name).arg("version").output()?;
			if output.status.success() {
				let stdout = String::from_utf8_lossy(&output.stdout);
				Ok(Some(stdout.replace('\n', "")))
			} else {
				Ok(None)
			}
		},
	}
}

pub fn package_manager(app_dir:&PathBuf) -> PackageManager {
	let mut use_npm = false;
	let mut use_pnpm = false;
	let mut use_yarn = false;
	let mut use_bun = false;

	for entry in std::fs::read_dir(app_dir)
		.unwrap()
		.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
	{
		match entry.as_str() {
			"pnpm-lock.yaml" => use_pnpm = true,
			"package-lock.json" => use_npm = true,
			"yarn.lock" => use_yarn = true,
			"bun.lockb" => use_bun = true,
			_ => {},
		}
	}

	if !use_npm && !use_pnpm && !use_yarn && !use_bun {
		println!("{}: no lock files found, defaulting to npm", "WARNING".yellow());
		return PackageManager::Npm;
	}

	let mut found = Vec::new();

	if use_npm {
		found.push(PackageManager::Npm);
	}
	if use_pnpm {
		found.push(PackageManager::Pnpm);
	}
	if use_yarn {
		found.push(PackageManager::Yarn);
	}
	if use_bun {
		found.push(PackageManager::Bun);
	}

	if found.len() > 1 {
		let pkg_manger = found[0];
		println!(
			"{}: Only one package manager should be used, but found {}.\n         Please remove \
			 unused package manager lock files, will use {} for now!",
			"WARNING".yellow(),
			found.iter().map(ToString::to_string).collect::<Vec<_>>().join(" and "),
			pkg_manger
		);
		return pkg_manger;
	}

	if use_npm {
		PackageManager::Npm
	} else if use_pnpm {
		PackageManager::Pnpm
	} else if use_bun {
		PackageManager::Bun
	} else if manager_version("yarn")
		.map(|v| v.chars().next().map(|c| c > '1').unwrap_or_default())
		.unwrap_or(false)
	{
		PackageManager::YarnBerry
	} else {
		PackageManager::Yarn
	}
}

pub fn items(
	app_dir:Option<&PathBuf>,
	package_manager:PackageManager,
	metadata:&VersionMetadata,
) -> Vec<SectionItem> {
	let mut items = Vec::new();
	if let Some(app_dir) = app_dir {
		for (package, version) in [
			("@tauri-apps/api", None),
			("@tauri-apps/cli", Some(metadata.js_cli.version.clone())),
		] {
			let app_dir = app_dir.clone();
			let item = nodejs_section_item(package.into(), version, app_dir, package_manager);
			items.push(item);
		}
	}

	items
}

pub fn nodejs_section_item(
	package:String,
	version:Option<String>,
	app_dir:PathBuf,
	package_manager:PackageManager,
) -> SectionItem {
	SectionItem::new().action(move || {
		let version = version.clone().unwrap_or_else(|| {
			package_manager
				.current_package_version(&package, &app_dir)
				.unwrap_or_default()
				.unwrap_or_default()
		});

		let latest_ver = super::packages_nodejs::npm_latest_version(&package_manager, &package)
			.unwrap_or_default()
			.unwrap_or_default();

		if version.is_empty() {
			format!("{} {}: not installed!", package, "".green())
		} else {
			format!(
				"{} {}: {}{}",
				package,
				"".dimmed(),
				version,
				if !(version.is_empty() || latest_ver.is_empty()) {
					let version = semver::Version::parse(version.as_str()).unwrap();
					let target_version = semver::Version::parse(latest_ver.as_str()).unwrap();

					if version < target_version {
						format!(" ({}, latest: {})", "outdated".yellow(), latest_ver.green())
					} else {
						"".into()
					}
				} else {
					"".into()
				}
			)
		}
		.into()
	})
}