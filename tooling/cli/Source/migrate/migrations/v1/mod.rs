// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use anyhow::Context;

use crate::{
	Result,
	helpers::app_paths::{app_dir, tauri_dir},
};

mod config;
mod frontend;
mod manifest;

pub fn run() -> Result<()> {
	let tauri_dir = tauri_dir();

	let app_dir = app_dir();

	let mut migrated = config::migrate(tauri_dir).context("Could not migrate config")?;

	manifest::migrate(tauri_dir).context("Could not migrate manifest")?;

	let plugins = frontend::migrate(app_dir)?;

	migrated.plugins.extend(plugins);

	// Add plugins
	for plugin in migrated.plugins {
		crate::add::run(crate::add::Options {
			plugin:plugin.clone(),
			branch:None,
			tag:None,
			rev:None,
			no_fmt:false,
		})
		.with_context(|| format!("Could not migrate plugin '{plugin}'"))?;
	}

	Ok(())
}
