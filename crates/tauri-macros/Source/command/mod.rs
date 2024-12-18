// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use proc_macro2::Ident;
use syn::{Path, PathSegment};

pub use self::{handler::Handler, wrapper::wrapper};

mod handler;
mod wrapper;

/// The autogenerated wrapper ident.
fn format_command_wrapper(function:&Ident) -> Ident { quote::format_ident!("__cmd__{}", function) }

/// This function will panic if the passed [`syn::Path`] does not have any
/// segments.
fn path_to_command(path:&mut Path) -> &mut PathSegment {
	path.segments.last_mut().expect("parsed syn::Path has no segment")
}
