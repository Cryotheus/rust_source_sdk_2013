//! Utilities for organizing generated Rust bindings into deterministic module trees.

use crate::determinism::ItemOrder;
use proc_macro2::Span;
use quote::format_ident;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};
use syn::{File, Item, LitStr, parse_quote};

/// The file name used for layout assertions extracted from a generated leaf module.
pub const LAYOUT_ASSERTIONS_FILE: &str = "layout_assertions.rs";

/// A Rust module path derived from a repository-relative source header path.
///
/// Directory components become parent modules and the final component has its
/// extension removed. Each component is sanitized into a regular (non-raw)
/// Rust identifier, so the path can also be used safely as an output path.
#[derive(Debug, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModulePath {
	segments: Vec<String>,
}

impl ModulePath {
	/// Converts a repository-relative header path into a Rust module path.
	///
	/// Invalid identifier characters become underscores, identifiers beginning
	/// with a digit are prefixed with an underscore, and Rust keywords receive a
	/// trailing underscore.
	pub fn from_header_path(path: impl AsRef<Path>) -> Result<Self, ModulePathError> {
		let path = path.as_ref();
		let mut source_segments = Vec::new();

		for component in path.components() {
			match component {
				Component::Normal(segment) => source_segments.push(segment),
				Component::CurDir => {}
				Component::ParentDir => {
					return Err(ModulePathError::ParentTraversal {
						path: path.to_owned(),
					});
				}
				Component::Prefix(_) | Component::RootDir => {
					return Err(ModulePathError::Absolute {
						path: path.to_owned(),
					});
				}
			}
		}

		let Some(file_name) = source_segments.pop() else {
			return Err(ModulePathError::MissingFileName {
				path: path.to_owned(),
			});
		};

		let Some(file_stem) = Path::new(file_name).file_stem() else {
			return Err(ModulePathError::MissingFileName {
				path: path.to_owned(),
			});
		};
		source_segments.push(file_stem);

		let segments = source_segments
			.into_iter()
			.map(|segment| {
				segment
					.to_str()
					.map(sanitize_identifier)
					.ok_or_else(|| ModulePathError::NonUtf8 {
						path: path.to_owned(),
					})
			})
			.collect::<Result<Vec<_>, _>>()?;

		Ok(Self { segments })
	}

	/// Returns the sanitized path components.
	pub fn as_segments(&self) -> &[String] {
		&self.segments
	}

	/// Iterates over the sanitized path components.
	pub fn segments(&self) -> impl ExactSizeIterator<Item = &str> {
		self.segments.iter().map(String::as_str)
	}

	/// Returns the relative directory in which this module's `mod.rs` is written.
	pub fn to_relative_dir(&self) -> PathBuf {
		self.segments.iter().collect()
	}
}

impl Display for ModulePath {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let mut segments = self.segments();

		if let Some(first) = segments.next() {
			f.write_str(first)?;
		}

		for segment in segments {
			write!(f, "::{segment}")?;
		}

		Ok(())
	}
}

impl TryFrom<&Path> for ModulePath {
	type Error = ModulePathError;

	fn try_from(path: &Path) -> Result<Self, Self::Error> {
		Self::from_header_path(path)
	}
}

impl TryFrom<PathBuf> for ModulePath {
	type Error = ModulePathError;

	fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
		Self::from_header_path(path)
	}
}

/// Failure to derive a safe Rust module path from a source header path.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum ModulePathError {
	#[error("header path must be repository-relative, got {path:?}")]
	Absolute { path: PathBuf },

	#[error("header path must name a file, got {path:?}")]
	MissingFileName { path: PathBuf },

	#[error("header path is not valid UTF-8: {path:?}")]
	NonUtf8 { path: PathBuf },

	#[error("header path cannot traverse a parent directory: {path:?}")]
	ParentTraversal { path: PathBuf },
}

/// A collection of generated leaf modules that can be rendered or written as
/// a deterministic Rust module tree.
#[derive(Default)]
pub struct ModuleTree {
	leaves: BTreeMap<ModulePath, File>,
}

impl ModuleTree {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn is_empty(&self) -> bool {
		self.leaves.is_empty()
	}

	pub fn len(&self) -> usize {
		self.leaves.len()
	}

	/// Adds a generated Rust file at an already-derived module path.
	pub fn insert(&mut self, module: ModulePath, file: File) -> Result<(), ModuleTreeError> {
		if file.shebang.is_some() {
			return Err(ModuleTreeError::Shebang { module });
		}

		// Generated trees are commonly committed on Windows and compiled on
		// Linux. Reject case-only collisions everywhere so generation never
		// depends on the host filesystem's case-sensitivity.
		if self
			.leaves
			.keys()
			.any(|existing| module_paths_equal_portably(existing, &module))
		{
			return Err(ModuleTreeError::DuplicateModulePath { module });
		}
		if self
			.leaves
			.keys()
			.any(|existing| module_paths_have_case_collision(existing, &module))
		{
			return Err(ModuleTreeError::PortablePathCollision { module });
		}

		self.validate_child_module_names(&module, &file)?;

		self.leaves.insert(module, file);
		Ok(())
	}

	fn validate_child_module_names(
		&self,
		module: &ModulePath,
		file: &File,
	) -> Result<(), ModuleTreeError> {
		let item_names = type_namespace_item_names(file);

		// If this leaf already has descendants, rendering would prepend a module
		// declaration for each immediate child to this file.
		for existing in self.leaves.keys() {
			if let Some(child) = immediate_descendant(module, existing)
				&& item_names.contains(child)
			{
				return Err(ModuleTreeError::ChildModuleItemCollision {
					module: module.clone(),
					name: child.to_owned(),
				});
			}
		}

		// Conversely, this insertion may introduce a child declaration into an
		// existing ancestor leaf. Validate that leaf before changing the tree.
		for (existing, existing_file) in &self.leaves {
			if let Some(child) = immediate_descendant(existing, module)
				&& type_namespace_item_names(existing_file).contains(child)
			{
				return Err(ModuleTreeError::ChildModuleItemCollision {
					module: existing.clone(),
					name: child.to_owned(),
				});
			}
		}

		Ok(())
	}

	/// Derives a module path from `header` and adds its generated Rust file.
	pub fn insert_header(
		&mut self,
		header: impl AsRef<Path>,
		file: File,
	) -> Result<ModulePath, ModuleTreeError> {
		let module = ModulePath::from_header_path(header)?;
		self.insert(module.clone(), file)?;
		Ok(module)
	}

	/// Renders all files using paths relative to a future output directory.
	///
	/// Every source leaf is represented by a directory containing `mod.rs`.
	/// Anonymous const items, as emitted by bindgen's layout tests, are moved to
	/// a sibling `layout_assertions.rs` and included from the leaf module.
	pub fn render(&self) -> BTreeMap<PathBuf, String> {
		let mut root = OutputNode::default();

		for (module, file) in &self.leaves {
			root.insert(module.as_segments(), file);
		}

		let mut rendered = BTreeMap::new();
		root.render(Path::new(""), &mut rendered);
		rendered
	}

	/// Writes the complete module tree below `output_dir`.
	///
	/// Existing generated files are replaced, but files which are no longer in
	/// the tree are not removed.
	pub fn write_to(&self, output_dir: impl AsRef<Path>) -> Result<(), ModuleWriteError> {
		let output_dir = output_dir.as_ref();

		for (relative_path, contents) in self.render() {
			let path = output_dir.join(relative_path);
			let parent = path.parent().unwrap_or(output_dir);

			fs::create_dir_all(parent).map_err(|source| ModuleWriteError::CreateDirectory {
				path: parent.to_owned(),
				source,
			})?;
			fs::write(&path, contents)
				.map_err(|source| ModuleWriteError::WriteFile { path, source })?;
		}

		Ok(())
	}
}

/// Failure to add a generated leaf to a [`ModuleTree`].
#[derive(Debug, thiserror::Error)]
pub enum ModuleTreeError {
	#[error(
		"generated leaf module {module} defines type/module item {name}, which conflicts with its generated child module"
	)]
	ChildModuleItemCollision { module: ModulePath, name: String },

	#[error("more than one source path maps to Rust module {module}")]
	DuplicateModulePath { module: ModulePath },

	#[error("generated module {module} has a case-only path conflict with another module")]
	PortablePathCollision { module: ModulePath },

	#[error(transparent)]
	InvalidModulePath(#[from] ModulePathError),

	#[error(
		"generated leaf module {module} contains a shebang, which is invalid in a nested Rust module"
	)]
	Shebang { module: ModulePath },
}

/// Failure to write one of the rendered module-tree files.
#[derive(Debug, thiserror::Error)]
pub enum ModuleWriteError {
	#[error("could not create generated binding directory {path:?}: {source}")]
	CreateDirectory {
		path: PathBuf,
		source: std::io::Error,
	},

	#[error("could not write generated binding file {path:?}: {source}")]
	WriteFile {
		path: PathBuf,
		source: std::io::Error,
	},
}

/// Separates bindgen-style anonymous layout assertion constants from the rest
/// of a generated Rust file.
pub fn split_layout_assertions(file: &File) -> (File, File) {
	let mut bindings: File = parse_quote! {};
	bindings.attrs = file.attrs.clone();
	bindings.items = Vec::with_capacity(file.items.len());
	let mut assertions: File = parse_quote! {};

	for item in file.items.iter().cloned() {
		if is_layout_assertion(&item) {
			assertions.items.push(item);
		} else {
			bindings.items.push(item);
		}
	}

	bindings
		.items
		.sort_by_cached_key(ItemOrder::module_item_order);
	assertions
		.items
		.sort_by_cached_key(ItemOrder::module_item_order);

	(bindings, assertions)
}

/// Returns whether `item` has bindgen's layout-assertion shape.
pub fn is_layout_assertion(item: &Item) -> bool {
	matches!(item, Item::Const(item) if item.ident == "_")
}

#[derive(Default)]
struct OutputNode<'file> {
	leaf: Option<&'file File>,
	children: BTreeMap<String, OutputNode<'file>>,
}

impl<'file> OutputNode<'file> {
	fn insert(&mut self, segments: &[String], file: &'file File) {
		let Some((first, rest)) = segments.split_first() else {
			self.leaf = Some(file);
			return;
		};

		self.children
			.entry(first.clone())
			.or_default()
			.insert(rest, file);
	}

	fn render(&self, directory: &Path, rendered: &mut BTreeMap<PathBuf, String>) {
		let (mut module, assertions) = self
			.leaf
			.map(split_layout_assertions)
			.unwrap_or_else(|| (parse_quote! {}, parse_quote! {}));
		let mut declarations = self
			.children
			.keys()
			.map(|name| {
				let ident = format_ident!("{name}");
				parse_quote! { pub mod #ident; }
			})
			.collect::<Vec<Item>>();

		declarations.append(&mut module.items);
		module.items = declarations;

		if !assertions.items.is_empty() {
			let assertions_file = LitStr::new(LAYOUT_ASSERTIONS_FILE, Span::call_site());
			module
				.items
				.push(parse_quote! { include!(#assertions_file); });
			rendered.insert(
				directory.join(LAYOUT_ASSERTIONS_FILE),
				prettyplease::unparse(&assertions),
			);
		}

		rendered.insert(directory.join("mod.rs"), prettyplease::unparse(&module));

		for (name, child) in &self.children {
			child.render(&directory.join(name), rendered);
		}
	}
}

fn sanitize_identifier(source: &str) -> String {
	let mut sanitized = String::with_capacity(source.len());

	for character in source.chars() {
		if character.is_ascii_alphanumeric() || character == '_' {
			sanitized.push(character);
		} else {
			sanitized.push('_');
		}
	}

	if sanitized.as_bytes().first().is_some_and(u8::is_ascii_digit) {
		sanitized.insert(0, '_');
	}

	if sanitized.is_empty() || sanitized == "_" {
		sanitized.push_str("module");
	}

	if is_rust_keyword(&sanitized) {
		sanitized.push('_');
	}

	sanitized
}

fn module_paths_equal_portably(left: &ModulePath, right: &ModulePath) -> bool {
	left.segments.len() == right.segments.len()
		&& left
			.segments
			.iter()
			.zip(&right.segments)
			.all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn module_paths_have_case_collision(left: &ModulePath, right: &ModulePath) -> bool {
	for (left, right) in left.segments.iter().zip(&right.segments) {
		if left == right {
			continue;
		}
		return left.eq_ignore_ascii_case(right);
	}
	false
}

fn immediate_descendant<'path>(
	ancestor: &ModulePath,
	descendant: &'path ModulePath,
) -> Option<&'path str> {
	(descendant.segments.len() > ancestor.segments.len()
		&& descendant.segments[..ancestor.segments.len()] == ancestor.segments[..])
		.then(|| descendant.segments[ancestor.segments.len()].as_str())
}

fn type_namespace_item_names(file: &File) -> BTreeSet<String> {
	let mut names = BTreeSet::new();

	for item in &file.items {
		let ident = match item {
			Item::Enum(item) => Some(&item.ident),
			Item::ExternCrate(item) => Some(
				item.rename
					.as_ref()
					.map_or(&item.ident, |(_, rename)| rename),
			),
			Item::Mod(item) => Some(&item.ident),
			Item::Struct(item) => Some(&item.ident),
			Item::Trait(item) => Some(&item.ident),
			Item::TraitAlias(item) => Some(&item.ident),
			Item::Type(item) => Some(&item.ident),
			Item::Union(item) => Some(&item.ident),
			_ => None,
		};
		if let Some(ident) = ident {
			names.insert(ident.to_string());
		}

		match item {
			Item::ForeignMod(item) => {
				for item in &item.items {
					if let syn::ForeignItem::Type(item) = item {
						names.insert(item.ident.to_string());
					}
				}
			}
			// A use target's namespace cannot be determined from syntax alone.
			// Conservatively reserve explicit bindings; glob imports are the one
			// case for which no local names are available to validate.
			Item::Use(item) => collect_use_binding_names(&item.tree, None, &mut names),
			_ => {}
		}
	}

	names
}

fn collect_use_binding_names(
	tree: &syn::UseTree,
	parent: Option<&str>,
	names: &mut BTreeSet<String>,
) {
	match tree {
		syn::UseTree::Path(path) => {
			let ident = path.ident.to_string();
			collect_use_binding_names(&path.tree, Some(&ident), names);
		}
		syn::UseTree::Name(name) if name.ident == "self" => {
			if let Some(parent) = parent {
				names.insert(parent.to_owned());
			}
		}
		syn::UseTree::Name(name) => {
			names.insert(name.ident.to_string());
		}
		syn::UseTree::Rename(rename) => {
			names.insert(rename.rename.to_string());
		}
		syn::UseTree::Group(group) => {
			for tree in &group.items {
				collect_use_binding_names(tree, parent, names);
			}
		}
		syn::UseTree::Glob(_) => {}
	}
}

fn is_rust_keyword(identifier: &str) -> bool {
	matches!(
		identifier,
		"Self"
			| "abstract"
			| "as"
			| "async"
			| "await"
			| "become"
			| "box"
			| "break"
			| "const"
			| "continue"
			| "crate"
			| "do"
			| "dyn"
			| "else"
			| "enum"
			| "extern"
			| "false"
			| "final"
			| "fn"
			| "for"
			| "gen"
			| "if"
			| "impl"
			| "in"
			| "let"
			| "loop"
			| "macro"
			| "match"
			| "mod"
			| "move"
			| "mut"
			| "override"
			| "priv"
			| "pub"
			| "ref"
			| "return"
			| "self"
			| "static"
			| "struct"
			| "super"
			| "trait"
			| "true"
			| "try"
			| "type"
			| "typeof"
			| "union"
			| "unsafe"
			| "unsized"
			| "use"
			| "virtual"
			| "where"
			| "while"
			| "yield"
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sanitizes_header_paths_into_rust_modules() {
		let path = ModulePath::from_header_path("Public API/9-lives/type.generated.hpp").unwrap();

		assert_eq!(
			path.as_segments(),
			["Public_API", "_9_lives", "type_generated"]
		);
		assert_eq!(path.to_string(), "Public_API::_9_lives::type_generated");
		assert_eq!(
			path.to_relative_dir(),
			PathBuf::from("Public_API")
				.join("_9_lives")
				.join("type_generated")
		);

		let keyword = ModulePath::from_header_path("async/mod.h").unwrap();
		assert_eq!(keyword.as_segments(), ["async_", "mod_"]);
	}

	#[test]
	fn rejects_paths_outside_the_source_root() {
		assert!(matches!(
			ModulePath::from_header_path("../outside.h"),
			Err(ModulePathError::ParentTraversal { .. })
		));
		assert!(matches!(
			ModulePath::from_header_path(Path::new("/absolute/header.h")),
			Err(ModulePathError::Absolute { .. })
		));
	}

	#[test]
	fn rejects_duplicate_sanitized_module_paths() {
		let mut tree = ModuleTree::new();
		tree.insert_header("api/foo-bar.hpp", parse_quote! {})
			.unwrap();

		let error = tree
			.insert_header("api/foo_bar.h", parse_quote! {})
			.unwrap_err();

		assert!(matches!(
			error,
			ModuleTreeError::DuplicateModulePath { module } if module.to_string() == "api::foo_bar"
		));
	}

	#[test]
	fn rejects_case_only_module_collisions_on_every_host() {
		let mut tree = ModuleTree::new();
		tree.insert_header("Public/API.h", parse_quote! {}).unwrap();

		let error = tree
			.insert_header("public/api.hpp", parse_quote! {})
			.unwrap_err();

		assert!(matches!(error, ModuleTreeError::DuplicateModulePath { .. }));
	}

	#[test]
	fn rejects_case_only_directory_prefix_collisions_on_every_host() {
		let mut tree = ModuleTree::new();
		tree.insert_header("Public/alpha.h", parse_quote! {})
			.unwrap();

		let error = tree
			.insert_header("public/beta.h", parse_quote! {})
			.unwrap_err();

		assert!(matches!(
			error,
			ModuleTreeError::PortablePathCollision { .. }
		));
	}

	#[test]
	fn rejects_type_item_that_conflicts_with_existing_child_module() {
		let mut tree = ModuleTree::new();
		tree.insert_header("api/detail/types.h", parse_quote! {})
			.unwrap();

		let error = tree
			.insert_header("api.h", parse_quote! { pub struct detail; })
			.unwrap_err();

		assert!(matches!(
			error,
			ModuleTreeError::ChildModuleItemCollision { module, name }
				if module.to_string() == "api" && name == "detail"
		));
	}

	#[test]
	fn rejects_new_child_module_that_conflicts_with_existing_type_item() {
		let mut tree = ModuleTree::new();
		tree.insert_header("api.h", parse_quote! { pub type detail = usize; })
			.unwrap();

		let error = tree
			.insert_header("api/detail.h", parse_quote! {})
			.unwrap_err();

		assert!(matches!(
			error,
			ModuleTreeError::ChildModuleItemCollision { module, name }
				if module.to_string() == "api" && name == "detail"
		));
	}

	#[test]
	fn rejects_child_module_that_conflicts_with_explicit_use_rename() {
		let mut tree = ModuleTree::new();
		tree.insert_header(
			"api.h",
			parse_quote! { pub use upstream::Interface as detail; },
		)
		.unwrap();

		let error = tree
			.insert_header("api/detail.h", parse_quote! {})
			.unwrap_err();

		assert!(matches!(
			error,
			ModuleTreeError::ChildModuleItemCollision { module, name }
				if module.to_string() == "api" && name == "detail"
		));
	}

	#[test]
	fn allows_value_item_to_share_a_name_with_child_module() {
		let mut tree = ModuleTree::new();
		tree.insert_header("api.h", parse_quote! { pub fn detail() {} })
			.unwrap();
		tree.insert_header("api/detail.h", parse_quote! {}).unwrap();

		let rendered = tree.render();
		let api = rendered.get(&PathBuf::from("api").join("mod.rs")).unwrap();
		assert!(api.contains("pub mod detail;"), "{api}");
		assert!(api.contains("pub fn detail()"), "{api}");
	}

	#[test]
	fn renders_nested_modules_and_per_leaf_layout_assertions() {
		let mut tree = ModuleTree::new();
		tree.insert_header(
			"public/tier1/interface.hpp",
			parse_quote! {
				pub struct Interface;

				#[allow(clippy::identity_op)]
				const _: () = {
					assert!(::std::mem::size_of::<Interface>() == 0);
				};
			},
		)
		.unwrap();
		tree.insert_header(
			"public/engine/server.h",
			parse_quote! { pub struct Server; },
		)
		.unwrap();

		let rendered = tree.render();

		assert_eq!(
			rendered.get(Path::new("mod.rs")).unwrap(),
			"pub mod public;\n"
		);
		assert_eq!(
			rendered
				.get(&PathBuf::from("public").join("mod.rs"))
				.unwrap(),
			"pub mod engine;\npub mod tier1;\n"
		);

		let leaf = rendered
			.get(
				&PathBuf::from("public")
					.join("tier1")
					.join("interface")
					.join("mod.rs"),
			)
			.unwrap();
		assert!(leaf.contains("pub struct Interface;"));
		assert!(leaf.contains("include!(\"layout_assertions.rs\");"));
		assert!(!leaf.contains("size_of"));

		let assertions = rendered
			.get(
				&PathBuf::from("public")
					.join("tier1")
					.join("interface")
					.join(LAYOUT_ASSERTIONS_FILE),
			)
			.unwrap();
		assert!(assertions.contains("const _: ()"), "{assertions}");
		assert!(assertions.contains("size_of"), "{assertions}");
	}

	#[test]
	fn rendering_is_independent_of_leaf_insertion_order() {
		let mut forward = ModuleTree::new();
		forward
			.insert_header("zeta/z.h", parse_quote! { pub struct Z; })
			.unwrap();
		forward
			.insert_header("alpha/a.h", parse_quote! { pub struct A; })
			.unwrap();

		let mut reverse = ModuleTree::new();
		reverse
			.insert_header("alpha/a.h", parse_quote! { pub struct A; })
			.unwrap();
		reverse
			.insert_header("zeta/z.h", parse_quote! { pub struct Z; })
			.unwrap();

		assert_eq!(forward.render(), reverse.render());
	}
}
