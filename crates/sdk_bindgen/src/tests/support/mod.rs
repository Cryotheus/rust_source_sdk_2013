//! Helpers shared by the generator's unit tests: Clang and bindgen fixtures,
//! bindgen's provenance callbacks, queries of generated syntax, and temporary
//! directories.

use crate::cpp_vtable::{RecordIndex, collect_records};
use crate::provenance::ProvenanceCollector;
use bindgen::callbacks::{DiscoveredItem, DiscoveredItemId, ParseCallbacks, SourceLocation};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::{File, Ident, Item};

/// The file name [`records`] gives its in-memory C++ fixture.
pub(crate) const FIXTURE_FILE: &str = "cpp_vtable_fixture.cpp";

/// A uniquely named directory under the system's temporary directory, removed
/// with its contents on drop.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
	pub(crate) fn new() -> Self {
		static NEXT: AtomicUsize = AtomicUsize::new(0);

		loop {
			let path = std::env::temp_dir().join(format!(
				"source-sdk-bindgen-test-{}-{}",
				std::process::id(),
				NEXT.fetch_add(1, Ordering::Relaxed)
			));

			match fs::create_dir(&path) {
				Ok(()) => return Self(path),
				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
				Err(error) => panic!("cannot create the temporary directory {path:?}: {error}"),
			}
		}
	}

	pub(crate) fn path(&self) -> &Path {
		&self.0
	}
}

impl Drop for TempDir {
	fn drop(&mut self) {
		// Only `TempDir::new` created this directory, so nothing else owns its contents.
		let _ = fs::remove_dir_all(&self.0);
	}
}

/// A bindgen builder for the in-memory header `file_name`, parsed with `arguments`.
pub(crate) fn bindgen_fixture(
	file_name: &str,
	source: &str,
	arguments: &[String],
) -> bindgen::Builder {
	bindgen::builder()
		.header_contents(file_name, source)
		.clang_args(arguments)
}

/// Clang arguments that parse a fixture as C++17 for the host target.
pub(crate) fn cxx17_arguments() -> Vec<String> {
	["-x", "c++", "-std=c++17"].map(str::to_owned).to_vec()
}

/// Reports to `collector`, as bindgen would, the enum `name` declared in `path`.
pub(crate) fn found_enum(collector: &ProvenanceCollector, id: usize, name: &str, path: &str) {
	collector.new_item_found(
		DiscoveredItemId::new(id),
		DiscoveredItem::Enum {
			final_name: name.into(),
		},
		Some(&location(path)),
	);
}

/// Reports to `collector`, as bindgen would, the struct `name` declared in
/// `path` under the same C++ name.
pub(crate) fn found_struct(collector: &ProvenanceCollector, id: usize, name: &str, path: &str) {
	collector.new_item_found(
		DiscoveredItemId::new(id),
		DiscoveredItem::Struct {
			original_name: Some(name.into()),
			final_name: name.into(),
		},
		Some(&location(path)),
	);
}

/// The names of the structs, unions and enums declared at the top level of `syntax`.
pub(crate) fn generated_type_names(syntax: &File) -> BTreeSet<String> {
	syntax
		.items
		.iter()
		.filter_map(record_name)
		.map(ToString::to_string)
		.collect()
}

/// Whether `syntax` declares a struct, union, enum or type alias named `name`
/// at its top level.
pub(crate) fn has_type(syntax: &File, name: &str) -> bool {
	syntax.items.iter().any(|item| match item {
		Item::Type(item) => item.ident == name,
		item => record_name(item).is_some_and(|ident| ident == name),
	})
}

/// The start of `path`, as bindgen reports an item's location.
pub(crate) fn location(path: &str) -> SourceLocation {
	SourceLocation {
		line: 1,
		col: 1,
		byte_offset: 0,
		file_name: Some(path.to_owned()),
	}
}

fn record_name(item: &Item) -> Option<&Ident> {
	match item {
		Item::Struct(item) => Some(&item.ident),
		Item::Union(item) => Some(&item.ident),
		Item::Enum(item) => Some(&item.ident),
		_ => None,
	}
}

/// The records Clang collects from `source`, parsed as [`FIXTURE_FILE`].
pub(crate) fn records(source: &str) -> RecordIndex {
	records_in(FIXTURE_FILE, source)
}

/// The records Clang collects from `source`, parsed as the C++17 file `file_name`.
pub(crate) fn records_in(file_name: &str, source: &str) -> RecordIndex {
	collect_records(file_name, source, &cxx17_arguments()).unwrap()
}

/// A provenance collector for the SDK root `sdk`, with the bridge header `bridge.hpp`.
pub(crate) fn sdk_collector() -> ProvenanceCollector {
	ProvenanceCollector::new("sdk", ["bridge.hpp"])
}

/// [`cxx17_arguments`] for the target triple `target`.
pub(crate) fn target_arguments(target: &str) -> Vec<String> {
	let mut arguments = cxx17_arguments();

	arguments.push(format!("--target={target}"));
	arguments
}
