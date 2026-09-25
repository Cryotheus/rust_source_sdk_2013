//! Source provenance captured while bindgen discovers and emits items.
//!
//! This module is intentionally independent from the SDK-specific generation
//! pipeline.  A collector can be cloned into [`bindgen::Builder::parse_callbacks`]
//! while the retained clone is used to query an immutable [`ProvenanceIndex`]
//! after generation.

use bindgen::callbacks::{DiscoveredItem, DiscoveredItemId, ParseCallbacks, SourceLocation};
use crys_bricks::bindings::ModulePath;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use syn::visit::Visit;
use syn::{File, Item, Type, UseTree};

/// Where the declaration associated with a generated item originated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceOrigin {
	/// A declaration in the configured source repository.
	Source {
		/// Normalized path reported by Clang.
		path: PathBuf,
		/// Normalized path relative to the configured source root.
		relative_path: PathBuf,
	},

	/// A declaration in a synthetic bridge supplied to bindgen.
	Bridge {
		/// Normalized path reported by Clang.
		path: PathBuf,
	},

	/// A declaration outside the source repository and configured bridges.
	External {
		/// Normalized path reported by Clang.
		path: PathBuf,
	},

	/// Bindgen did not report a source location for the item.
	Unknown,
}

impl SourceOrigin {
	pub const fn is_source(&self) -> bool {
		matches!(self, Self::Source { .. })
	}

	pub const fn is_bridge(&self) -> bool {
		matches!(self, Self::Bridge { .. })
	}

	pub const fn is_external(&self) -> bool {
		matches!(self, Self::External { .. })
	}
}

/// The bindgen item category and any relationship needed to resolve ownership.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GeneratedItemKind {
	Struct,
	Union,
	Enum,
	Alias { alias_for: DiscoveredItemId },
	Function,
	Method { parent: DiscoveredItemId },
	Constant,
	Variable,
}

impl GeneratedItemKind {
	const fn is_type(&self) -> bool {
		matches!(
			self,
			Self::Struct | Self::Union | Self::Enum | Self::Alias { .. }
		)
	}
}

/// Provenance for one item reported through `ParseCallbacks::new_item_found`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeneratedItemRecord {
	pub id: DiscoveredItemId,
	pub kind: GeneratedItemKind,

	/// Name bindgen emitted, or the C/C++ alias name for an alias callback.
	pub final_name: String,

	/// Original C/C++ name when bindgen makes one available.
	pub original_name: Option<String>,
	pub origin: SourceOrigin,
}

impl GeneratedItemRecord {
	fn original_type_name(&self) -> Option<&str> {
		if !self.kind.is_type() {
			return None;
		}

		match &self.kind {
			GeneratedItemKind::Struct | GeneratedItemKind::Union => self.original_name.as_deref(),
			// Bindgen does not currently report a separate original name for
			// enums or aliases. Their callback names are the names accepted by
			// bindgen's allowlist/opaque-type matching.
			GeneratedItemKind::Enum | GeneratedItemKind::Alias { .. } => Some(&self.final_name),
			_ => None,
		}
	}
}

#[derive(Debug)]
struct CollectorConfig {
	source_root: PathBuf,
	bridge_files: BTreeSet<PathBuf>,
}

#[derive(Debug, Default)]
struct CollectorState {
	records: BTreeMap<DiscoveredItemId, GeneratedItemRecord>,
	included_files: BTreeSet<PathBuf>,
}

/// Shared bindgen callback which records item names, relationships, and paths.
///
/// Clone this value before moving one clone into `bindgen::Builder`. All clones
/// share the same collection state.
#[derive(Debug, Clone)]
pub struct ProvenanceCollector {
	config: Arc<CollectorConfig>,
	state: Arc<Mutex<CollectorState>>,
}

impl ProvenanceCollector {
	pub fn new<Root, Bridges, Bridge>(source_root: Root, bridge_files: Bridges) -> Self
	where
		Root: AsRef<Path>,
		Bridges: IntoIterator<Item = Bridge>,
		Bridge: AsRef<Path>,
	{
		let source_root = normalize_path(source_root.as_ref());
		let bridge_files = bridge_files
			.into_iter()
			.map(|path| normalize_path(path.as_ref()))
			.collect();

		Self {
			config: Arc::new(CollectorConfig {
				source_root,
				bridge_files,
			}),
			state: Arc::new(Mutex::new(CollectorState::default())),
		}
	}

	pub fn source_root(&self) -> &Path {
		&self.config.source_root
	}

	/// Take a deterministic snapshot of all provenance observed so far.
	pub fn index(&self) -> ProvenanceIndex {
		let state = self.lock_state();
		ProvenanceIndex {
			source_root: self.config.source_root.clone(),
			bridge_files: self.config.bridge_files.clone(),
			records: state.records.clone(),
			included_files: state.included_files.clone(),
		}
	}

	fn lock_state(&self) -> MutexGuard<'_, CollectorState> {
		self.state
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	fn classify(&self, source_location: Option<&SourceLocation>) -> SourceOrigin {
		let Some(file_name) = source_location.and_then(|location| location.file_name.as_deref())
		else {
			return SourceOrigin::Unknown;
		};

		let path = normalize_path(Path::new(file_name));

		if self
			.config
			.bridge_files
			.iter()
			.any(|bridge| paths_equal(bridge, &path))
		{
			return SourceOrigin::Bridge { path };
		}

		if let Some(relative_path) = strip_prefix(&path, &self.config.source_root) {
			return SourceOrigin::Source {
				path,
				relative_path,
			};
		}

		SourceOrigin::External { path }
	}
}

impl ParseCallbacks for ProvenanceCollector {
	fn include_file(&self, filename: &str) {
		self.lock_state()
			.included_files
			.insert(normalize_path(Path::new(filename)));
	}

	fn new_item_found(
		&self,
		id: DiscoveredItemId,
		item: DiscoveredItem,
		source_location: Option<&SourceLocation>,
	) {
		let origin = self.classify(source_location);
		let (kind, final_name, original_name) = match item {
			DiscoveredItem::Struct {
				original_name,
				final_name,
			} => (GeneratedItemKind::Struct, final_name, original_name),
			DiscoveredItem::Union {
				original_name,
				final_name,
			} => (GeneratedItemKind::Union, final_name, original_name),
			DiscoveredItem::Alias {
				alias_name,
				alias_for,
			} => (
				GeneratedItemKind::Alias { alias_for },
				alias_name.clone(),
				Some(alias_name),
			),
			DiscoveredItem::Enum { final_name } => (GeneratedItemKind::Enum, final_name, None),
			DiscoveredItem::Function { final_name } => {
				(GeneratedItemKind::Function, final_name, None)
			}
			DiscoveredItem::Method { final_name, parent } => {
				(GeneratedItemKind::Method { parent }, final_name, None)
			}
			DiscoveredItem::Constant { final_name } => {
				(GeneratedItemKind::Constant, final_name, None)
			}
			DiscoveredItem::Variable { final_name } => {
				(GeneratedItemKind::Variable, final_name, None)
			}
		};

		self.lock_state().records.insert(
			id,
			GeneratedItemRecord {
				id,
				kind,
				final_name,
				original_name,
				origin,
			},
		);
	}
}

/// Immutable, deterministic item-provenance lookup built by a collector.
#[derive(Debug, Clone)]
pub struct ProvenanceIndex {
	source_root: PathBuf,
	bridge_files: BTreeSet<PathBuf>,
	records: BTreeMap<DiscoveredItemId, GeneratedItemRecord>,
	included_files: BTreeSet<PathBuf>,
}

impl ProvenanceIndex {
	pub fn source_root(&self) -> &Path {
		&self.source_root
	}

	pub fn bridge_files(&self) -> impl ExactSizeIterator<Item = &Path> {
		self.bridge_files.iter().map(PathBuf::as_path)
	}

	pub fn records(&self) -> impl ExactSizeIterator<Item = &GeneratedItemRecord> {
		self.records.values()
	}

	/// All files observed by bindgen's include callback, normalized and sorted.
	pub fn included_files(&self) -> impl ExactSizeIterator<Item = &Path> {
		self.included_files.iter().map(PathBuf::as_path)
	}

	pub fn record(&self, id: DiscoveredItemId) -> Option<&GeneratedItemRecord> {
		self.records.get(&id)
	}

	/// Resolve method ownership through its parent and aliases without their own
	/// location through the aliased item.
	pub fn resolved_origin(&self, id: DiscoveredItemId) -> Option<SourceOrigin> {
		let mut current = id;
		let mut visited = BTreeSet::new();

		loop {
			if !visited.insert(current) {
				return None;
			}

			let record = self.records.get(&current)?;
			match &record.kind {
				GeneratedItemKind::Method { parent } => current = *parent,
				GeneratedItemKind::Alias { alias_for }
					if matches!(&record.origin, SourceOrigin::Unknown) =>
				{
					current = *alias_for;
				}
				_ => return Some(record.origin.clone()),
			}
		}
	}

	pub fn module_for_id(&self, id: DiscoveredItemId) -> Option<ModulePath> {
		let SourceOrigin::Source { relative_path, .. } = self.resolved_origin(id)? else {
			return None;
		};

		ModulePath::from_header_path(relative_path).ok()
	}

	/// Find the source module which owns a bindgen-emitted name.
	///
	/// Vtables and custom `__crys_` artifacts inherit the class owner's module.
	pub fn module_for_generated_name(&self, name: &str) -> Option<ModulePath> {
		if let Some(module) = self.direct_module_for_name(name) {
			return Some(module);
		}

		if let Some(module) = self.constant_owner_module(name) {
			return Some(module);
		}

		if let Some(class_name) = name.strip_suffix("__bindgen_vtable") {
			return self.direct_module_for_name(class_name);
		}

		if let Some((class_name, _)) = name.split_once("__crys_")
			&& !class_name.is_empty()
		{
			return self.direct_module_for_name(class_name);
		}

		let artifact = name.strip_prefix("__crys_")?;
		self.records
			.values()
			.filter(|record| record.kind.is_type())
			.filter(|record| artifact_mentions_class(artifact, &record.final_name))
			.max_by_key(|record| record.final_name.len())
			.and_then(|record| self.module_for_id(record.id))
	}

	/// Bindgen-emitted type names which are proven to resolve exclusively to
	/// declarations outside both the source repository and bridge files.
	///
	/// If the same generated name is also reported for a source, bridge, or
	/// unknown declaration, it is deliberately excluded. This lets pruning
	/// passes treat the returned set as proof without risking a local item.
	pub fn external_generated_type_names(&self) -> BTreeSet<String> {
		let mut external = BTreeSet::new();
		let mut not_proven_external = BTreeSet::new();

		for record in self.records.values().filter(|record| record.kind.is_type()) {
			match self.resolved_origin(record.id) {
				Some(origin) if origin.is_external() => {
					external.insert(record.final_name.clone());
				}
				_ => {
					not_proven_external.insert(record.final_name.clone());
				}
			}
		}

		external.difference(&not_proven_external).cloned().collect()
	}

	pub(crate) fn generated_type_names(&self) -> BTreeSet<String> {
		self.records
			.values()
			.filter(|record| record.kind.is_type())
			.map(|record| record.final_name.clone())
			.collect()
	}

	/// Original names of reachable external types, excluding source-owned and
	/// bridge declarations of the same name.
	pub fn external_original_type_names(&self) -> Vec<String> {
		let mut external = BTreeSet::new();
		let mut non_external = BTreeSet::new();

		for record in self.records.values() {
			let Some(name) = record.original_type_name() else {
				continue;
			};
			let Some(origin) = self.resolved_origin(record.id) else {
				continue;
			};

			if origin.is_external() {
				external.insert(name.to_owned());
			} else if origin.is_source() || origin.is_bridge() {
				non_external.insert(name.to_owned());
			}
		}

		external.difference(&non_external).cloned().collect()
	}

	/// Regexes which match exactly the names returned by
	/// [`Self::external_original_type_names`] and can be passed directly to
	/// `bindgen::Builder::opaque_type`.
	pub fn external_opaque_type_patterns(&self) -> Vec<String> {
		self.external_opaque_type_patterns_excluding(&BTreeSet::new())
	}

	/// [`Self::external_opaque_type_patterns`] without the names in `excluded`.
	pub fn external_opaque_type_patterns_excluding(
		&self,
		excluded: &BTreeSet<String>,
	) -> Vec<String> {
		self.external_original_type_names()
			.into_iter()
			.filter(|name| !excluded.contains(name))
			.map(|name| format!("^{}$", escape_regex(&name)))
			.collect()
	}

	fn direct_module_for_name(&self, name: &str) -> Option<ModulePath> {
		let mut owner = None;

		for record in self
			.records
			.values()
			.filter(|record| record.final_name == name)
		{
			let Some(module) = self.module_for_id(record.id) else {
				continue;
			};

			match &owner {
				None => owner = Some(module),
				Some(existing) if existing == &module => {}
				Some(_) => return None,
			}
		}

		owner
	}

	fn constant_owner_module(&self, name: &str) -> Option<ModulePath> {
		let owner_name = self.constant_owner_generated_type_name(name)?;
		let owners = self
			.records
			.values()
			.filter(|record| record.kind.is_type() && record.final_name == owner_name)
			.filter_map(|record| self.module_for_id(record.id))
			.collect::<BTreeSet<_>>();

		if owners.len() == 1 {
			owners.into_iter().next()
		} else {
			None
		}
	}

	pub(crate) fn constant_owner_generated_type_name(&self, name: &str) -> Option<String> {
		let longest_prefix = self
			.records
			.values()
			.filter(|record| record.kind.is_type())
			.filter(|record| {
				name.strip_prefix(&record.final_name)
					.is_some_and(|suffix| suffix.starts_with('_'))
			})
			.map(|record| record.final_name.len())
			.max()?;
		let owners = self
			.records
			.values()
			.filter(|record| record.kind.is_type() && record.final_name.len() == longest_prefix)
			.filter(|record| {
				name.strip_prefix(&record.final_name)
					.is_some_and(|suffix| suffix.starts_with('_'))
			})
			.map(|record| record.final_name.clone())
			.collect::<BTreeSet<_>>();

		if owners.len() == 1 {
			owners.into_iter().next()
		} else {
			None
		}
	}
}

/// Partition a generated syntax tree into deterministic, source-owned modules.
///
/// Items without a source owner (including bridge/external declarations) are
/// placed in `support_module`. An anonymous layout constant follows its unique
/// referenced type owner, and falls back to support when no unambiguous owner
/// can be determined.
pub fn partition_file(
	syntax: File,
	provenance: &ProvenanceIndex,
	support_module: ModulePath,
) -> Vec<(ModulePath, File)> {
	let File {
		shebang,
		frontmatter,
		attrs,
		items,
	} = syntax;

	let mut modules = BTreeMap::new();

	modules.insert(
		support_module.clone(),
		File {
			shebang: shebang.clone(),
			frontmatter: frontmatter.clone(),
			attrs: attrs.clone(),
			items: Vec::new(),
		},
	);

	for item in items {
		let module = item_owner_module(&item, provenance).unwrap_or_else(|| support_module.clone());
		modules
			.entry(module)
			.or_insert_with(|| File {
				shebang: shebang.clone(),
				frontmatter: frontmatter.clone(),
				attrs: attrs.clone(),
				items: Vec::new(),
			})
			.items
			.push(item);
	}

	modules.into_iter().collect()
}

fn item_owner_module(item: &Item, provenance: &ProvenanceIndex) -> Option<ModulePath> {
	if matches!(item, Item::Const(item) if item.ident == "_") {
		return layout_assertion_owner(item, provenance);
	}

	if let Item::Use(item) = item {
		return use_tree_owner(&item.tree, provenance);
	}

	item_owner_name(item).and_then(|name| provenance.module_for_generated_name(&name))
}

fn use_tree_owner(tree: &UseTree, provenance: &ProvenanceIndex) -> Option<ModulePath> {
	fn collect_source_names(tree: &UseTree, names: &mut BTreeSet<String>) -> bool {
		match tree {
			UseTree::Path(path) => collect_source_names(&path.tree, names),

			UseTree::Name(name) => {
				names.insert(name.ident.to_string());
				true
			}

			UseTree::Rename(rename) => {
				names.insert(rename.ident.to_string());
				true
			}

			UseTree::Group(group) => group
				.items
				.iter()
				.all(|tree| collect_source_names(tree, names)),
			UseTree::Glob(_) => false,
		}
	}

	let mut names = BTreeSet::new();

	if !collect_source_names(tree, &mut names) || names.is_empty() {
		return None;
	}

	let owners = names
		.iter()
		.map(|name| provenance.module_for_generated_name(name))
		.collect::<Option<BTreeSet<_>>>()?;

	if owners.len() == 1 {
		owners.into_iter().next()
	} else {
		None
	}
}

fn item_owner_name(item: &Item) -> Option<String> {
	match item {
		Item::Struct(item) => Some(item.ident.to_string()),
		Item::Union(item) => Some(item.ident.to_string()),
		Item::Enum(item) => Some(item.ident.to_string()),
		Item::Type(item) => Some(item.ident.to_string()),
		Item::Const(item) if item.ident != "_" => Some(item.ident.to_string()),
		Item::Static(item) => Some(item.ident.to_string()),
		Item::Fn(item) => Some(item.sig.ident.to_string()),
		Item::Impl(item) => self_type_name(&item.self_ty),
		_ => None,
	}
}

fn self_type_name(ty: &Type) -> Option<String> {
	match ty {
		Type::Path(path) => path
			.path
			.segments
			.last()
			.map(|segment| segment.ident.to_string()),
		Type::Reference(reference) => self_type_name(&reference.elem),
		Type::Paren(paren) => self_type_name(&paren.elem),
		Type::Group(group) => self_type_name(&group.elem),
		_ => None,
	}
}

fn layout_assertion_owner(item: &Item, provenance: &ProvenanceIndex) -> Option<ModulePath> {
	#[derive(Default)]
	struct TypePathCollector {
		names: BTreeSet<String>,
	}

	impl<'ast> Visit<'ast> for TypePathCollector {
		fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
			if let Some(segment) = path.path.segments.last() {
				self.names.insert(segment.ident.to_string());
			}

			syn::visit::visit_type_path(self, path);
		}
	}

	let mut references = TypePathCollector::default();

	references.visit_item(item);

	let mut owners = BTreeSet::new();
	let mut has_unknown_owner = false;

	for name in references.names {
		if let Some(owner) = provenance.module_for_generated_name(&name) {
			owners.insert(owner);
		} else {
			has_unknown_owner = true;
		}
	}

	if !has_unknown_owner && owners.len() == 1 {
		owners.into_iter().next()
	} else {
		None
	}
}

fn artifact_mentions_class(artifact: &str, class_name: &str) -> bool {
	artifact.match_indices(class_name).any(|(start, _)| {
		let end = start + class_name.len();
		let starts_on_boundary = start == 0 || artifact.as_bytes()[start - 1] == b'_';
		let ends_on_boundary = end == artifact.len() || artifact.as_bytes()[end] == b'_';
		starts_on_boundary && ends_on_boundary
	})
}

fn normalize_path(path: &Path) -> PathBuf {
	let mut normalized = PathBuf::new();

	for component in path.components() {
		match component {
			Component::CurDir => {}

			Component::ParentDir => {
				if !normalized.pop() {
					normalized.push(component.as_os_str());
				}
			}

			_ => normalized.push(component.as_os_str()),
		}
	}

	normalized
}

fn strip_prefix(path: &Path, base: &Path) -> Option<PathBuf> {
	let path_components = path.components().collect::<Vec<_>>();
	let base_components = base.components().collect::<Vec<_>>();

	if base_components.len() > path_components.len()
		|| !base_components
			.iter()
			.zip(&path_components)
			.all(|(base, path)| os_str_equal(base.as_os_str(), path.as_os_str()))
	{
		return None;
	}

	let mut relative = PathBuf::new();

	for component in &path_components[base_components.len()..] {
		relative.push(component.as_os_str());
	}

	Some(relative)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
	let left = left.components().collect::<Vec<_>>();
	let right = right.components().collect::<Vec<_>>();

	left.len() == right.len()
		&& left
			.iter()
			.zip(right)
			.all(|(left, right)| os_str_equal(left.as_os_str(), right.as_os_str()))
}

fn os_str_equal(left: &OsStr, right: &OsStr) -> bool {
	if cfg!(windows) {
		left.to_string_lossy()
			.eq_ignore_ascii_case(&right.to_string_lossy())
	} else {
		left == right
	}
}

pub(crate) fn escape_regex(name: &str) -> String {
	let mut escaped = String::with_capacity(name.len());

	for character in name.chars() {
		if matches!(
			character,
			'.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
		) {
			escaped.push('\\');
		}
		escaped.push(character);
	}

	escaped
}

#[cfg(test)]
mod tests {
	use super::*;
	use syn::parse_quote;

	fn location(path: &str) -> SourceLocation {
		SourceLocation {
			line: 1,
			col: 1,
			byte_offset: 0,
			file_name: Some(path.to_owned()),
		}
	}

	fn module(path: &str) -> ModulePath {
		ModulePath::from_header_path(path).unwrap()
	}

	#[test]
	fn classifies_and_normalizes_source_bridge_and_external_paths() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		collector.new_item_found(
			DiscoveredItemId::new(1),
			DiscoveredItem::Struct {
				original_name: Some("Owned".into()),
				final_name: "Owned".into(),
			},
			Some(&location("sdk/public/../public/owned.hpp")),
		);

		collector.new_item_found(
			DiscoveredItemId::new(2),
			DiscoveredItem::Struct {
				original_name: Some("Bridge".into()),
				final_name: "Bridge".into(),
			},
			Some(&location("bridge.hpp")),
		);

		collector.new_item_found(
			DiscoveredItemId::new(3),
			DiscoveredItem::Struct {
				original_name: Some("External".into()),
				final_name: "External".into(),
			},
			Some(&location("toolchain/include/external.hpp")),
		);

		let index = collector.index();

		assert!(matches!(
			&index.record(DiscoveredItemId::new(1)).unwrap().origin,
			SourceOrigin::Source { relative_path, .. } if relative_path == Path::new("public/owned.hpp")
		));

		assert!(
			index
				.record(DiscoveredItemId::new(2))
				.unwrap()
				.origin
				.is_bridge()
		);
		assert!(
			index
				.record(DiscoveredItemId::new(3))
				.unwrap()
				.origin
				.is_external()
		);
	}

	#[test]
	fn records_normalized_include_paths_in_deterministic_order() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		collector.include_file("toolchain/include/external.hpp");
		collector.include_file("sdk/public/../public/owned.hpp");
		collector.include_file("bridge.hpp");
		collector.include_file("sdk/public/owned.hpp");

		assert_eq!(
			collector.index().included_files().collect::<Vec<_>>(),
			[
				Path::new("bridge.hpp"),
				Path::new("sdk/public/owned.hpp"),
				Path::new("toolchain/include/external.hpp"),
			]
		);
	}

	#[test]
	fn resolves_alias_and_method_ownership_through_item_ids() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		collector.new_item_found(
			DiscoveredItemId::new(1),
			DiscoveredItem::Struct {
				original_name: Some("Owner".into()),
				final_name: "Owner".into(),
			},
			Some(&location("sdk/public/owner.h")),
		);

		collector.new_item_found(
			DiscoveredItemId::new(2),
			DiscoveredItem::Alias {
				alias_name: "OwnerAlias".into(),
				alias_for: DiscoveredItemId::new(1),
			},
			None,
		);

		collector.new_item_found(
			DiscoveredItemId::new(3),
			DiscoveredItem::Method {
				final_name: "Owner_method".into(),
				parent: DiscoveredItemId::new(1),
			},
			Some(&location("toolchain/generated.hpp")),
		);

		let index = collector.index();

		assert_eq!(
			index.module_for_id(DiscoveredItemId::new(2)),
			Some(module("public/owner.h"))
		);
		assert_eq!(
			index.module_for_id(DiscoveredItemId::new(3)),
			Some(module("public/owner.h"))
		);
	}

	#[test]
	fn external_filter_only_returns_original_reachable_type_names() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		for (id, (path, original, final_name)) in [
			("sdk/public/owned.h", "Owned", "Owned"),
			("bridge.hpp", "BridgeType", "BridgeType"),
			("toolchain/external.h", "External.Type", "ExternalType"),
			("sdk/public/redeclared.h", "Redeclared", "Redeclared"),
			("toolchain/external.h", "Redeclared", "RedeclaredExternal"),
		]
		.into_iter()
		.enumerate()
		{
			collector.new_item_found(
				DiscoveredItemId::new(id),
				DiscoveredItem::Struct {
					original_name: Some(original.into()),
					final_name: final_name.into(),
				},
				Some(&location(path)),
			);
		}

		collector.new_item_found(
			DiscoveredItemId::new(6),
			DiscoveredItem::Function {
				final_name: "external_function".into(),
			},
			Some(&location("toolchain/external.h")),
		);

		let index = collector.index();

		assert_eq!(
			index.external_generated_type_names(),
			BTreeSet::from(["ExternalType".to_owned(), "RedeclaredExternal".to_owned()])
		);

		assert_eq!(index.external_original_type_names(), ["External.Type"]);
		assert_eq!(index.external_opaque_type_patterns(), [r"^External\.Type$"]);
	}

	#[test]
	fn partitions_types_impls_vtables_artifacts_and_helpers() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		for (id, item) in [
			DiscoveredItem::Struct {
				original_name: Some("Class".into()),
				final_name: "Class".into(),
			},
			DiscoveredItem::Alias {
				alias_name: "ClassAlias".into(),
				alias_for: DiscoveredItemId::new(1),
			},
			DiscoveredItem::Constant {
				final_name: "CLASS_VALUE".into(),
			},
		]
		.into_iter()
		.enumerate()
		{
			collector.new_item_found(
				DiscoveredItemId::new(id),
				item,
				Some(&location("sdk/public/class.hpp")),
			);
		}

		let syntax: File = parse_quote! {
			pub struct Class;
			pub struct Class__bindgen_vtable;
			pub struct Class__crys_destructor_slots;
			pub struct __crys_vtable_Class;
			pub type __crys_vtable_Class_slot_0 = unsafe extern "C" fn(*mut Class);
			pub type ClassAlias = Class;
			pub const CLASS_VALUE: u32 = 1;
			impl Class {}
			pub struct __BindgenOpaqueArray8<T>(T);
			const _: () = {};
		};

		let partitions = partition_file(syntax, &collector.index(), module("support.rs"));
		let partitions = partitions.into_iter().collect::<BTreeMap<_, _>>();
		let class_items = &partitions[&module("public/class.hpp")].items;
		let support_items = &partitions[&module("support.rs")].items;

		assert_eq!(class_items.len(), 8);
		assert_eq!(support_items.len(), 2);
	}

	#[test]
	fn assigns_unreported_constants_to_the_longest_type_name_prefix() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		for (id, name, path) in [
			(1, "IVEngineServer", "sdk/public/engine.h"),
			(
				2,
				"IVEngineServer_eFindMapResult",
				"sdk/public/engine_result.h",
			),
		] {
			collector.new_item_found(
				DiscoveredItemId::new(id),
				DiscoveredItem::Enum {
					final_name: name.into(),
				},
				Some(&location(path)),
			);
		}

		assert_eq!(
			collector
				.index()
				.module_for_generated_name("IVEngineServer_eFindMapResult_Found"),
			Some(module("public/engine_result.h"))
		);
	}

	#[test]
	fn assigns_a_renamed_use_to_the_source_items_module() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		collector.new_item_found(
			DiscoveredItemId::new(1),
			DiscoveredItem::Enum {
				final_name: "_fieldtypes".into(),
			},
			Some(&location("sdk/public/datamap.h")),
		);

		let syntax: File = parse_quote! {
			pub enum _fieldtypes {}
			pub use self::_fieldtypes as fieldtype_t;
		};

		let partitions = partition_file(syntax, &collector.index(), module("support.rs"));
		let partitions = partitions.into_iter().collect::<BTreeMap<_, _>>();

		assert_eq!(partitions[&module("public/datamap.h")].items.len(), 2);
		assert!(partitions[&module("support.rs")].items.is_empty());
	}

	#[test]
	fn only_assigns_use_groups_with_one_unambiguous_owner() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		for (id, name, path) in [
			(1, "First", "sdk/public/datamap.h"),
			(2, "Second", "sdk/public/datamap.h"),
			(3, "Elsewhere", "sdk/public/elsewhere.h"),
		] {
			collector.new_item_found(
				DiscoveredItemId::new(id),
				DiscoveredItem::Struct {
					original_name: Some(name.into()),
					final_name: name.into(),
				},
				Some(&location(path)),
			);
		}

		let syntax: File = parse_quote! {
			pub use self::{First, Second as SecondAlias};
			pub use self::{First, Elsewhere};
			pub use self::*;
		};

		let partitions = partition_file(syntax, &collector.index(), module("support.rs"));
		let partitions = partitions.into_iter().collect::<BTreeMap<_, _>>();

		assert_eq!(partitions[&module("public/datamap.h")].items.len(), 1);
		assert_eq!(partitions[&module("support.rs")].items.len(), 2);
	}

	#[test]
	fn assigns_layout_assertions_to_their_unique_referenced_type_owner() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);

		for (id, name, path) in [
			(1, "First", "sdk/public/first.h"),
			(2, "Second", "sdk/public/second.h"),
		] {
			collector.new_item_found(
				DiscoveredItemId::new(id),
				DiscoveredItem::Struct {
					original_name: Some(name.into()),
					final_name: name.into(),
				},
				Some(&location(path)),
			);
		}

		let syntax: File = parse_quote! {
			const _: () = {
				::std::mem::size_of::<First>();
			};
			const _: () = {
				::std::mem::size_of::<First>();
				::std::mem::align_of::<Second>();
			};
			const _: () = {};
		};

		let partitions = partition_file(syntax, &collector.index(), module("support.rs"));
		let partitions = partitions.into_iter().collect::<BTreeMap<_, _>>();

		assert_eq!(partitions[&module("public/first.h")].items.len(), 1);
		assert_eq!(partitions[&module("support.rs")].items.len(), 2);
		assert!(!partitions.contains_key(&module("public/second.h")));
	}
}
