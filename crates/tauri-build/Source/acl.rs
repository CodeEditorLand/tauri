// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
	collections::{BTreeMap, HashMap},
	env,
	fs,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use tauri_utils::{
	acl::{
		ACL_MANIFESTS_FILE_NAME,
		APP_ACL_KEY,
		CAPABILITIES_FILE_NAME,
		capability::Capability,
		manifest::Manifest,
		schema::CAPABILITIES_SCHEMA_FOLDER_PATH,
	},
	platform::Target,
	write_if_changed,
};

use crate::Attributes;

/// Definition of a plugin that is part of the Tauri application instead of
/// having its own crate.
///
/// By default it generates a plugin manifest that parses permissions from the
/// `permissions/$plugin-name` directory. To change the glob pattern that is
/// used to find permissions, use [`Self::permissions_path_pattern`].
///
/// To autogenerate permissions for each of the plugin commands, see
/// [`Self::commands`].
#[derive(Debug, Default, Clone)]
pub struct InlinedPlugin {
	commands:&'static [&'static str],
	permissions_path_pattern:Option<&'static str>,
	default:Option<DefaultPermissionRule>,
}

/// Variants of a generated default permission that can be used on an
/// [`InlinedPlugin`].
#[derive(Debug, Clone)]
pub enum DefaultPermissionRule {
	/// Allow all commands from [`InlinedPlugin::commands`].
	AllowAllCommands,
	/// Allow the given list of permissions.
	///
	/// Note that the list refers to permissions instead of command names,
	/// so for example a command called `execute` would need to be allowed as
	/// `allow-execute`.
	Allow(Vec<String>),
}

impl InlinedPlugin {
	pub fn new() -> Self { Self::default() }

	/// Define a list of commands that gets permissions autogenerated in the
	/// format of `allow-$command` and `deny-$command` where $command is the
	/// command name in snake_case.
	pub fn commands(mut self, commands:&'static [&'static str]) -> Self {
		self.commands = commands;

		self
	}

	/// Sets a glob pattern that is used to find the permissions of this inlined
	/// plugin.
	///
	/// **Note:** You must emit [rerun-if-changed] instructions for the plugin
	/// permissions directory.
	///
	/// By default it is `./permissions/$plugin-name/**/*`
	pub fn permissions_path_pattern(mut self, pattern:&'static str) -> Self {
		self.permissions_path_pattern.replace(pattern);

		self
	}

	/// Creates a default permission for the plugin using the given rule.
	///
	/// Alternatively you can pull a permission in the filesystem in the
	/// permissions directory, see [`Self::permissions_path_pattern`].
	pub fn default_permission(mut self, default:DefaultPermissionRule) -> Self {
		self.default.replace(default);

		self
	}
}

/// Tauri application permission manifest.
///
/// By default it generates a manifest that parses permissions from the
/// `permissions` directory. To change the glob pattern that is used to find
/// permissions, use [`Self::permissions_path_pattern`].
///
/// To autogenerate permissions for each of the app commands, see
/// [`Self::commands`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AppManifest {
	commands:&'static [&'static str],
	permissions_path_pattern:Option<&'static str>,
}

impl AppManifest {
	pub fn new() -> Self { Self::default() }

	/// Define a list of commands that gets permissions autogenerated in the
	/// format of `allow-$command` and `deny-$command` where $command is the
	/// command name in snake_case.
	pub fn commands(mut self, commands:&'static [&'static str]) -> Self {
		self.commands = commands;

		self
	}

	/// Sets a glob pattern that is used to find the permissions of the app.
	///
	/// **Note:** You must emit [rerun-if-changed] instructions for the
	/// permissions directory.
	///
	/// By default it is `./permissions/**/*` ignoring any [`InlinedPlugin`].
	pub fn permissions_path_pattern(mut self, pattern:&'static str) -> Self {
		self.permissions_path_pattern.replace(pattern);

		self
	}
}

/// Saves capabilities in a file inside the project, mainly to be read by
/// tauri-cli.
fn save_capabilities(capabilities:&BTreeMap<String, Capability>) -> Result<PathBuf> {
	let dir = Path::new(CAPABILITIES_SCHEMA_FOLDER_PATH);
	fs::create_dir_all(dir)?;

	let path = dir.join(CAPABILITIES_FILE_NAME);
	let json = serde_json::to_string(&capabilities)?;
	write_if_changed(&path, json)?;

	Ok(path)
}

/// Saves ACL manifests in a file inside the project, mainly to be read by
/// tauri-cli.
fn save_acl_manifests(acl_manifests:&BTreeMap<String, Manifest>) -> Result<PathBuf> {
	let dir = Path::new(CAPABILITIES_SCHEMA_FOLDER_PATH);
	fs::create_dir_all(dir)?;

	let path = dir.join(ACL_MANIFESTS_FILE_NAME);
	let json = serde_json::to_string(&acl_manifests)?;
	write_if_changed(&path, json)?;

	Ok(path)
}

/// Read plugin permissions and scope schema from env vars
fn read_plugins_manifests() -> Result<BTreeMap<String, Manifest>> {
	use tauri_utils::acl;

	let permission_map =
		acl::build::read_permissions().context("failed to read plugin permissions")?;
	let mut global_scope_map =
		acl::build::read_global_scope_schemas().context("failed to read global scope schemas")?;

	let mut manifests = BTreeMap::new();

	for (plugin_name, permission_files) in permission_map {
		let global_scope_schema = global_scope_map.remove(&plugin_name);

		let manifest = Manifest::new(permission_files, global_scope_schema);

		manifests.insert(plugin_name, manifest);
	}

	Ok(manifests)
}

fn inline_plugins(
	out_dir:&Path,
	inlined_plugins:HashMap<&'static str, InlinedPlugin>,
) -> Result<BTreeMap<String, Manifest>> {
	let mut acl_manifests = BTreeMap::new();

	for (name, plugin) in inlined_plugins {
		let plugin_out_dir = out_dir.join("plugins").join(name);

		fs::create_dir_all(&plugin_out_dir)?;

		let mut permission_files = if plugin.commands.is_empty() {
			Vec::new()
		} else {
			let autogenerated = tauri_utils::acl::build::autogenerate_command_permissions(
				&plugin_out_dir,
				plugin.commands,
				"",
				false,
			);

			let default_permissions = plugin.default.map(|default| {
				match default {
					DefaultPermissionRule::AllowAllCommands => autogenerated.allowed,
					DefaultPermissionRule::Allow(permissions) => permissions,
				}
			});
			if let Some(default_permissions) = default_permissions {
				let default_permissions = default_permissions
					.iter()
					.map(|p| format!("\"{p}\""))
					.collect::<Vec<String>>()
					.join(",");

				let default_permission = format!(
					r###"# Automatically generated - DO NOT EDIT!
[default]
permissions = [{default_permissions}]
"###
				);

				let default_permission_path = plugin_out_dir.join("default.toml");

				write_if_changed(&default_permission_path, default_permission).unwrap_or_else(
					|_| panic!("unable to autogenerate {default_permission_path:?}"),
				);
			}

			tauri_utils::acl::build::define_permissions(
				&plugin_out_dir.join("*").to_string_lossy(),
				name,
				&plugin_out_dir,
				|_| true,
			)?
		};

		if let Some(pattern) = plugin.permissions_path_pattern {
			permission_files.extend(tauri_utils::acl::build::define_permissions(
				pattern,
				name,
				&plugin_out_dir,
				|_| true,
			)?);
		} else {
			let default_permissions_path = Path::new("permissions").join(name);
			if default_permissions_path.exists() {
				println!("cargo:rerun-if-changed={}", default_permissions_path.display());
			}
			permission_files.extend(tauri_utils::acl::build::define_permissions(
				&default_permissions_path.join("**").join("*").to_string_lossy(),
				name,
				&plugin_out_dir,
				|_| true,
			)?);
		}

		let manifest = tauri_utils::acl::manifest::Manifest::new(permission_files, None);

		acl_manifests.insert(name.into(), manifest);
	}

	Ok(acl_manifests)
}

fn app_manifest_permissions(
	out_dir:&Path,
	manifest:AppManifest,
	inlined_plugins:&HashMap<&'static str, InlinedPlugin>,
) -> Result<Manifest> {
	let app_out_dir = out_dir.join("app-manifest");
	fs::create_dir_all(&app_out_dir)?;
	let pkg_name = "__app__";

	let mut permission_files = if manifest.commands.is_empty() {
		Vec::new()
	} else {
		let autogenerated_path = Path::new("./permissions/autogenerated");

		tauri_utils::acl::build::autogenerate_command_permissions(
			autogenerated_path,
			manifest.commands,
			"",
			false,
		);

		tauri_utils::acl::build::define_permissions(
			&autogenerated_path.join("*").to_string_lossy(),
			pkg_name,
			&app_out_dir,
			|_| true,
		)?
	};

	if let Some(pattern) = manifest.permissions_path_pattern {
		permission_files.extend(tauri_utils::acl::build::define_permissions(
			pattern,
			pkg_name,
			&app_out_dir,
			|_| true,
		)?);
	} else {
		let default_permissions_path = Path::new("permissions");

		if default_permissions_path.exists() {
			println!("cargo:rerun-if-changed={}", default_permissions_path.display());
		}

		let permissions_root = env::current_dir()?.join("permissions");

		let inlined_plugins_permissions:Vec<_> =
			inlined_plugins.keys().map(|name| permissions_root.join(name)).collect();

		permission_files.extend(tauri_utils::acl::build::define_permissions(
			&default_permissions_path.join("**").join("*").to_string_lossy(),
			pkg_name,
			&app_out_dir,
			// filter out directories containing inlined plugins
			|p| {
				!inlined_plugins_permissions
					.iter()
					.any(|inlined_path| p.starts_with(inlined_path))
			},
		)?);
	}

	Ok(tauri_utils::acl::manifest::Manifest::new(permission_files, None))
}

fn validate_capabilities(
	acl_manifests:&BTreeMap<String, Manifest>,
	capabilities:&BTreeMap<String, Capability>,
) -> Result<()> {
	let target = tauri_utils::platform::Target::from_triple(&std::env::var("TARGET").unwrap());

	for capability in capabilities.values() {
		if !capability
			.platforms
			.as_ref()
			.map(|platforms| platforms.contains(&target))
			.unwrap_or(true)
		{
			continue;
		}

		for permission_entry in &capability.permissions {
			let permission_id = permission_entry.identifier();

			let key = permission_id.get_prefix().unwrap_or(APP_ACL_KEY);
			let permission_name = permission_id.get_base();

			let permission_exists = acl_manifests
				.get(key)
				.map(|manifest| {
					// the default permission is always treated as valid, the CLI automatically adds
					// it on the `tauri add` command
					permission_name == "default"
						|| manifest.permissions.contains_key(permission_name)
						|| manifest.permission_sets.contains_key(permission_name)
				})
				.unwrap_or(false);

			if !permission_exists {
				let mut available_permissions = Vec::new();

				for (key, manifest) in acl_manifests {
					let prefix =
						if key == APP_ACL_KEY { "".to_string() } else { format!("{key}:") };
					if manifest.default_permission.is_some() {
						available_permissions.push(format!("{prefix}default"));
					}
					for p in manifest.permissions.keys() {
						available_permissions.push(format!("{prefix}{p}"));
					}
					for p in manifest.permission_sets.keys() {
						available_permissions.push(format!("{prefix}{p}"));
					}
				}

				anyhow::bail!(
					"Permission {} not found, expected one of {}",
					permission_id.get(),
					available_permissions.join(", ")
				);
			}
		}
	}

	Ok(())
}

pub fn build(out_dir:&Path, target:Target, attributes:&Attributes) -> super::Result<()> {
	let mut acl_manifests = read_plugins_manifests()?;

	let app_manifest =
		app_manifest_permissions(out_dir, attributes.app_manifest, &attributes.inlined_plugins)?;
	if app_manifest.default_permission.is_some()
		|| !app_manifest.permission_sets.is_empty()
		|| !app_manifest.permissions.is_empty()
	{
		acl_manifests.insert(APP_ACL_KEY.into(), app_manifest);
	}

	acl_manifests.extend(inline_plugins(out_dir, attributes.inlined_plugins.clone())?);

	let acl_manifests_path = save_acl_manifests(&acl_manifests)?;
	fs::copy(acl_manifests_path, out_dir.join(ACL_MANIFESTS_FILE_NAME))?;

	tauri_utils::acl::schema::generate_capability_schema(&acl_manifests, target)?;

	let capabilities = if let Some(pattern) = attributes.capabilities_path_pattern {
		tauri_utils::acl::build::parse_capabilities(pattern)?
	} else {
		println!("cargo:rerun-if-changed=capabilities");

		tauri_utils::acl::build::parse_capabilities("./capabilities/**/*")?
	};
	validate_capabilities(&acl_manifests, &capabilities)?;

	let capabilities_path = save_capabilities(&capabilities)?;
	fs::copy(capabilities_path, out_dir.join(CAPABILITIES_FILE_NAME))?;

	tauri_utils::plugin::save_global_api_scripts_paths(out_dir);

	Ok(())
}
