// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
	collections::{BTreeMap, BTreeSet, HashMap},
	env::current_dir,
	fs::{copy, create_dir_all, read_to_string, write},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use schemars::{
	schema::{
		ArrayValidation,
		InstanceType,
		Metadata,
		ObjectValidation,
		RootSchema,
		Schema,
		SchemaObject,
		SubschemaValidation,
	},
	schema_for,
};
use tauri_utils::{
	acl::{
		APP_ACL_KEY,
		capability::{Capability, CapabilityFile},
		manifest::Manifest,
	},
	platform::Target,
	write_if_changed,
};

const CAPABILITIES_SCHEMA_FILE_NAME:&str = "schema.json";
/// Path of the folder where schemas are saved.
const CAPABILITIES_SCHEMA_FOLDER_PATH:&str = "gen/schemas";
const CAPABILITIES_FILE_NAME:&str = "capabilities.json";
const ACL_MANIFESTS_FILE_NAME:&str = "acl-manifests.json";

/// Definition of a plugin that is part of the Tauri application instead of
/// having its own crate.
///
/// By default it generates a plugin manifest that parses permissions from the
/// `permissions/$plugin-name` directory. To change the glob pattern that is
/// used to find permissions, use [`Self::permissions_path_pattern`].
///
/// To autogenerate permissions for each of the plugin commands, see
/// [`Self::commands`].
#[derive(Debug, Default)]
pub struct InlinedPlugin {
	commands:&'static [&'static str],
	permissions_path_pattern:Option<&'static str>,
	default:Option<DefaultPermissionRule>,
}

/// Variants of a generated default permission that can be used on an
/// [`InlinedPlugin`].
#[derive(Debug)]
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
#[derive(Debug, Default)]
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

fn capabilities_schema(acl_manifests:&BTreeMap<String, Manifest>) -> RootSchema {
	let mut schema = schema_for!(CapabilityFile);

	fn schema_from(key:&str, id:&str, description:Option<&str>) -> Schema {
		let command_name = if key == APP_ACL_KEY { id.to_string() } else { format!("{key}:{id}") };

		Schema::Object(SchemaObject {
			metadata:Some(Box::new(Metadata {
				description:description.as_ref().map(|d| format!("{command_name} -> {d}")),
				..Default::default()
			})),
			instance_type:Some(InstanceType::String.into()),
			enum_values:Some(vec![serde_json::Value::String(command_name)]),
			..Default::default()
		})
	}

	let mut permission_schemas = Vec::new();

	for (key, manifest) in acl_manifests {
		for (set_id, set) in &manifest.permission_sets {
			permission_schemas.push(schema_from(key, set_id, Some(&set.description)));
		}

		permission_schemas.push(schema_from(
			key,
			"default",
			manifest.default_permission.as_ref().map(|d| d.description.as_ref()),
		));

		for (permission_id, permission) in &manifest.permissions {
			permission_schemas.push(schema_from(
				key,
				permission_id,
				permission.description.as_deref(),
			));
		}
	}

	if let Some(Schema::Object(obj)) = schema.definitions.get_mut("Identifier") {
		obj.object = None;

		obj.instance_type = None;

		obj.metadata.as_mut().map(|metadata| {
			metadata.description.replace("Permission identifier".to_string());

			metadata
		});

		obj.subschemas.replace(Box::new(SubschemaValidation {
			one_of:Some(permission_schemas),
			..Default::default()
		}));
	}

	let mut definitions = Vec::new();

	if let Some(Schema::Object(obj)) = schema.definitions.get_mut("PermissionEntry") {
		let permission_entry_any_of_schemas = obj.subschemas().any_of.as_mut().unwrap();

		if let Schema::Object(scope_extended_schema_obj) =
			permission_entry_any_of_schemas.last_mut().unwrap()
		{
			let mut global_scope_one_of = Vec::new();

			for (key, manifest) in acl_manifests {
				if let Some(global_scope_schema) = &manifest.global_scope_schema {
					let global_scope_schema_def:RootSchema =
						serde_json::from_value(global_scope_schema.clone()).unwrap_or_else(|e| {
							panic!("invalid JSON schema for plugin {key}: {e}")
						});

					let global_scope_schema = Schema::Object(SchemaObject {
						array:Some(Box::new(ArrayValidation {
							items:Some(Schema::Object(global_scope_schema_def.schema).into()),
							..Default::default()
						})),
						..Default::default()
					});

					definitions.push(global_scope_schema_def.definitions);

					let mut required = BTreeSet::new();

					required.insert("identifier".to_string());

					let mut object = ObjectValidation { required, ..Default::default() };

					let mut permission_schemas = Vec::new();

					permission_schemas.push(schema_from(
						key,
						"default",
						manifest.default_permission.as_ref().map(|d| d.description.as_ref()),
					));

					for set in manifest.permission_sets.values() {
						permission_schemas.push(schema_from(
							key,
							&set.identifier,
							Some(&set.description),
						));
					}

					for permission in manifest.permissions.values() {
						permission_schemas.push(schema_from(
							key,
							&permission.identifier,
							permission.description.as_deref(),
						));
					}

					let identifier_schema = Schema::Object(SchemaObject {
						subschemas:Some(Box::new(SubschemaValidation {
							one_of:Some(permission_schemas),
							..Default::default()
						})),
						..Default::default()
					});

					object.properties.insert("identifier".to_string(), identifier_schema);

					object.properties.insert("allow".to_string(), global_scope_schema.clone());

					object.properties.insert("deny".to_string(), global_scope_schema);

					global_scope_one_of.push(Schema::Object(SchemaObject {
						instance_type:Some(InstanceType::Object.into()),
						object:Some(Box::new(object)),
						..Default::default()
					}));
				}
			}

			if !global_scope_one_of.is_empty() {
				scope_extended_schema_obj.object = None;

				scope_extended_schema_obj.subschemas.replace(Box::new(SubschemaValidation {
					one_of:Some(global_scope_one_of),
					..Default::default()
				}));
			};
		}
	}

	for definitions_map in definitions {
		schema.definitions.extend(definitions_map);
	}

	schema
}

pub fn generate_schema(acl_manifests:&BTreeMap<String, Manifest>, target:Target) -> Result<()> {
	let schema = capabilities_schema(acl_manifests);

	let schema_str = serde_json::to_string_pretty(&schema).unwrap();

	let out_dir = PathBuf::from(CAPABILITIES_SCHEMA_FOLDER_PATH);

	create_dir_all(&out_dir).context("unable to create schema output directory")?;

	let schema_path = out_dir.join(format!("{target}-{CAPABILITIES_SCHEMA_FILE_NAME}"));

	if schema_str != read_to_string(&schema_path).unwrap_or_default() {
		write(&schema_path, schema_str)?;

		copy(
			schema_path,
			out_dir.join(format!(
				"{}-{CAPABILITIES_SCHEMA_FILE_NAME}",
				if target.is_desktop() { "desktop" } else { "mobile" }
			)),
		)?;
	}

	Ok(())
}

pub fn save_capabilities(capabilities:&BTreeMap<String, Capability>) -> Result<PathBuf> {
	let capabilities_path =
		PathBuf::from(CAPABILITIES_SCHEMA_FOLDER_PATH).join(CAPABILITIES_FILE_NAME);

	let capabilities_json = serde_json::to_string(&capabilities)?;

	if capabilities_json != read_to_string(&capabilities_path).unwrap_or_default() {
		std::fs::write(&capabilities_path, capabilities_json)?;
	}

	Ok(capabilities_path)
}

pub fn save_acl_manifests(acl_manifests:&BTreeMap<String, Manifest>) -> Result<PathBuf> {
	let acl_manifests_path =
		PathBuf::from(CAPABILITIES_SCHEMA_FOLDER_PATH).join(ACL_MANIFESTS_FILE_NAME);

	let acl_manifests_json = serde_json::to_string(&acl_manifests)?;

	if acl_manifests_json != read_to_string(&acl_manifests_path).unwrap_or_default() {
		std::fs::write(&acl_manifests_path, acl_manifests_json)?;
	}

	Ok(acl_manifests_path)
}

pub fn get_manifests_from_plugins() -> Result<BTreeMap<String, Manifest>> {
	let permission_map =
		tauri_utils::acl::build::read_permissions().context("failed to read plugin permissions")?;

	let mut global_scope_map = tauri_utils::acl::build::read_global_scope_schemas()
		.context("failed to read global scope schemas")?;

	let mut processed = BTreeMap::new();

	for (plugin_name, permission_files) in permission_map {
		let manifest = Manifest::new(permission_files, global_scope_map.remove(&plugin_name));

		processed.insert(plugin_name, manifest);
	}

	Ok(processed)
}

pub fn inline_plugins(
	out_dir:&Path,
	inlined_plugins:HashMap<&'static str, InlinedPlugin>,
) -> Result<BTreeMap<String, Manifest>> {
	let mut acl_manifests = BTreeMap::new();

	for (name, plugin) in inlined_plugins {
		let plugin_out_dir = out_dir.join("plugins").join(name);

		create_dir_all(&plugin_out_dir)?;

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
				let default_permission_toml = format!(
					r###"# Automatically generated - DO NOT EDIT!
[default]
permissions = [{default_permissions}]
"###,
					default_permissions = default_permissions
						.iter()
						.map(|p| format!("\"{p}\""))
						.collect::<Vec<String>>()
						.join(",")
				);

				let default_permission_toml_path = plugin_out_dir.join("default.toml");

				write_if_changed(&default_permission_toml_path, default_permission_toml)
					.unwrap_or_else(|_| {
						panic!("unable to autogenerate {default_permission_toml_path:?}")
					});
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

pub fn app_manifest_permissions(
	out_dir:&Path,
	manifest:AppManifest,
	inlined_plugins:&HashMap<&'static str, InlinedPlugin>,
) -> Result<Manifest> {
	let app_out_dir = out_dir.join("app-manifest");

	create_dir_all(&app_out_dir)?;

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

		let permissions_root = current_dir()?.join("permissions");

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

pub fn validate_capabilities(
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

			if key == "core" && permission_name == "default" {
				continue;
			}

			let permission_exists = acl_manifests
				.get(key)
				.map(|manifest| {
					// the default permission is always treated as valid, the
					// CLI automatically adds it on the `tauri add` command
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
