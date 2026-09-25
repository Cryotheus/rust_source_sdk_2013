//! Conservative removal of external types which bindgen emitted but no
//! retained generated item uses.

use crate::provenance::ProvenanceIndex;
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use syn::visit::Visit;
use syn::{File, Item, Type, UseTree};

/// Remove generated external type definitions which are unreachable from the
/// rest of the generated file.
///
/// Only names which provenance proves are exclusively external are eligible.
/// Source, bridge, unknown, or conflicting declarations therefore always stay
/// in the file and seed reachability. External definitions retain their own
/// external dependencies transitively. Associated impls and anonymous layout
/// assertions are removed only when they would refer to a definition removed
/// by this pass.
pub(crate) fn prune_unreferenced_external_types(syntax: &mut File, provenance: &ProvenanceIndex) {
	let mut external_names = provenance.external_generated_type_names();
	if external_names.is_empty() {
		return;
	}

	let mut local_type_names = provenance.generated_type_names();
	let mut definition_counts = BTreeMap::<String, usize>::new();
	for item in &syntax.items {
		if let Some(names) = definition_names(item) {
			for name in names {
				local_type_names.insert(name.clone());
				*definition_counts.entry(name).or_default() += 1;
			}
		}
	}
	// Duplicate definitions cannot be tied back to one provenance record with
	// certainty, so protect their names even if one record was external.
	external_names.retain(|name| definition_counts.get(name).copied().unwrap_or_default() <= 1);

	let mut dependencies = BTreeMap::<String, BTreeSet<String>>::new();
	let mut roots = referenced_attribute_names(&syntax.attrs, &external_names);

	for item in &syntax.items {
		let referenced_external = referenced_names(item, &external_names);
		match item_role(item, provenance, &external_names, &local_type_names) {
			ItemRole::ExternalDefinition(owners) | ItemRole::ExternalAssociated(owners) => {
				for owner in owners {
					dependencies
						.entry(owner)
						.or_default()
						.extend(referenced_external.iter().cloned());
				}
			}
			ItemRole::Retained => roots.extend(referenced_external),
		}
	}

	let retained_external = reachable_external_names(roots, &dependencies);
	let removed_names =
		removed_definition_names(&syntax.items, &external_names, &retained_external);

	syntax.items.retain(|item| {
		if let Some(owners) = external_definition_names(item, &external_names) {
			return owners.iter().any(|owner| retained_external.contains(owner));
		}

		let associated_item = matches!(item, Item::Impl(_) | Item::Const(_));
		if associated_item {
			return referenced_names(item, &removed_names).is_empty();
		}

		true
	});
}

#[derive(Debug)]
enum ItemRole {
	/// A struct, union, enum, type alias, or type re-export proven external.
	ExternalDefinition(BTreeSet<String>),
	/// An impl or layout assertion whose lifetime follows external types.
	ExternalAssociated(BTreeSet<String>),
	/// An item that must remain and consequently roots all of its references.
	Retained,
}

fn item_role(
	item: &Item,
	provenance: &ProvenanceIndex,
	external_names: &BTreeSet<String>,
	generated_type_names: &BTreeSet<String>,
) -> ItemRole {
	if let Some(owners) = external_definition_names(item, external_names) {
		return ItemRole::ExternalDefinition(owners);
	}

	if let Item::Impl(item_impl) = item
		&& let Some(owner) = direct_impl_self_type_name(item_impl)
		&& external_names.contains(&owner)
	{
		return ItemRole::ExternalAssociated(BTreeSet::from([owner]));
	}

	if matches!(item, Item::Const(item) if item.ident == "_") {
		let referenced_generated = referenced_names(item, generated_type_names);
		let external = referenced_generated
			.iter()
			.filter(|name| external_names.contains(*name))
			.cloned()
			.collect::<BTreeSet<_>>();
		let has_non_external = referenced_generated
			.iter()
			.any(|name| !external_names.contains(name));
		if !external.is_empty() && !has_non_external {
			return ItemRole::ExternalAssociated(external);
		}
	}

	if let Item::Const(item_const) = item
		&& item_const.ident != "_"
		&& let Some(owner) =
			provenance.constant_owner_generated_type_name(&item_const.ident.to_string())
		&& external_names.contains(&owner)
	{
		let referenced_generated = referenced_names(item, generated_type_names);
		let has_non_external = referenced_generated
			.iter()
			.any(|name| !external_names.contains(name));
		if !has_non_external && referenced_generated.contains(&owner) {
			return ItemRole::ExternalAssociated(BTreeSet::from([owner]));
		}
	}

	ItemRole::Retained
}

fn external_definition_names(
	item: &Item,
	external_names: &BTreeSet<String>,
) -> Option<BTreeSet<String>> {
	let names = definition_names(item)?;
	(!names.is_empty() && names.iter().all(|name| external_names.contains(name))).then_some(names)
}

fn definition_names(item: &Item) -> Option<BTreeSet<String>> {
	let direct_name = match item {
		Item::Struct(item) => Some(item.ident.to_string()),
		Item::Union(item) => Some(item.ident.to_string()),
		Item::Enum(item) => Some(item.ident.to_string()),
		Item::Type(item) => Some(item.ident.to_string()),
		_ => None,
	};
	if let Some(name) = direct_name {
		return Some(BTreeSet::from([name]));
	}

	let Item::Use(item_use) = item else {
		return None;
	};
	exported_use_names(&item_use.tree)
}

fn exported_use_names(tree: &UseTree) -> Option<BTreeSet<String>> {
	fn collect(tree: &UseTree, names: &mut BTreeSet<String>) -> bool {
		match tree {
			UseTree::Path(path) => collect(&path.tree, names),
			UseTree::Name(name) => {
				names.insert(name.ident.to_string());
				true
			}
			UseTree::Rename(rename) => {
				names.insert(rename.rename.to_string());
				true
			}
			UseTree::Group(group) => group.items.iter().all(|tree| collect(tree, names)),
			UseTree::Glob(_) => false,
		}
	}

	let mut names = BTreeSet::new();
	collect(tree, &mut names).then_some(names)
}

fn direct_impl_self_type_name(item: &syn::ItemImpl) -> Option<String> {
	let owner = self_type_name(&item.self_ty)?;
	let shadowed = item.generics.params.iter().any(
		|parameter| matches!(parameter, syn::GenericParam::Type(parameter) if parameter.ident == owner),
	);
	(!shadowed).then_some(owner)
}

fn self_type_name(type_: &Type) -> Option<String> {
	match type_ {
		Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => path
			.path
			.segments
			.first()
			.map(|segment| segment.ident.to_string()),
		Type::Path(_) => None,
		Type::Reference(reference) => self_type_name(&reference.elem),
		Type::Paren(paren) => self_type_name(&paren.elem),
		Type::Group(group) => self_type_name(&group.elem),
		_ => None,
	}
}

fn referenced_names(item: &Item, interesting_names: &BTreeSet<String>) -> BTreeSet<String> {
	let mut collector = ReferenceCollector {
		interesting_names,
		references: BTreeSet::new(),
	};
	collector.visit_item(item);
	collector.references
}

fn referenced_attribute_names(
	attributes: &[syn::Attribute],
	interesting_names: &BTreeSet<String>,
) -> BTreeSet<String> {
	let mut collector = ReferenceCollector {
		interesting_names,
		references: BTreeSet::new(),
	};
	for attribute in attributes {
		collector.visit_attribute(attribute);
	}
	collector.references
}

struct ReferenceCollector<'a> {
	interesting_names: &'a BTreeSet<String>,
	references: BTreeSet<String>,
}

impl ReferenceCollector<'_> {
	fn record(&mut self, name: impl ToString) {
		let name = name.to_string();
		if self.interesting_names.contains(&name) {
			self.references.insert(name);
		}
	}

	fn visit_macro_tokens(&mut self, tokens: &TokenStream) {
		for token in tokens.clone() {
			match token {
				TokenTree::Group(group) => self.visit_macro_tokens(&group.stream()),
				TokenTree::Ident(ident) => self.record(ident),
				TokenTree::Punct(_) | TokenTree::Literal(_) => {}
			}
		}
	}

	fn visit_use_source(&mut self, tree: &UseTree) {
		match tree {
			UseTree::Path(path) => {
				self.record(&path.ident);
				self.visit_use_source(&path.tree);
			}
			UseTree::Name(name) => self.record(&name.ident),
			UseTree::Rename(rename) => self.record(&rename.ident),
			UseTree::Group(group) => {
				for tree in &group.items {
					self.visit_use_source(tree);
				}
			}
			UseTree::Glob(_) => {}
		}
	}
}

impl<'ast> Visit<'ast> for ReferenceCollector<'_> {
	fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
		for segment in &path.path.segments {
			self.record(&segment.ident);
		}
		syn::visit::visit_type_path(self, path);
	}

	fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
		for segment in &path.path.segments {
			self.record(&segment.ident);
		}
		syn::visit::visit_expr_path(self, path);
	}

	fn visit_macro(&mut self, macro_: &'ast syn::Macro) {
		self.visit_macro_tokens(&macro_.tokens);
	}

	fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
		self.visit_use_source(&item.tree);
	}
}

fn reachable_external_names(
	roots: BTreeSet<String>,
	dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
	let mut retained = roots;
	let mut pending = retained.iter().cloned().collect::<VecDeque<_>>();

	while let Some(name) = pending.pop_front() {
		let Some(referenced) = dependencies.get(&name) else {
			continue;
		};
		for dependency in referenced {
			if retained.insert(dependency.clone()) {
				pending.push_back(dependency.clone());
			}
		}
	}

	retained
}

fn removed_definition_names(
	items: &[Item],
	external_names: &BTreeSet<String>,
	retained_names: &BTreeSet<String>,
) -> BTreeSet<String> {
	let mut removed = BTreeSet::new();
	for item in items {
		let Some(owners) = external_definition_names(item, external_names) else {
			continue;
		};
		if owners.iter().all(|owner| !retained_names.contains(owner)) {
			removed.extend(owners);
		}
	}
	removed
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::provenance::ProvenanceCollector;
	use bindgen::callbacks::{DiscoveredItem, DiscoveredItemId, ParseCallbacks, SourceLocation};
	use syn::parse_quote;

	fn location(path: &str) -> SourceLocation {
		SourceLocation {
			line: 1,
			col: 1,
			byte_offset: 0,
			file_name: Some(path.to_owned()),
		}
	}

	fn external_struct(collector: &ProvenanceCollector, id: usize, name: &str) {
		collector.new_item_found(
			DiscoveredItemId::new(id),
			DiscoveredItem::Struct {
				original_name: Some(name.into()),
				final_name: name.into(),
			},
			Some(&location("toolchain/include/external.h")),
		);
	}

	fn source_struct(collector: &ProvenanceCollector, id: usize, name: &str) {
		collector.new_item_found(
			DiscoveredItemId::new(id),
			DiscoveredItem::Struct {
				original_name: Some(name.into()),
				final_name: name.into(),
			},
			Some(&location("sdk/public/source.h")),
		);
	}

	fn external_enum(collector: &ProvenanceCollector, id: usize, name: &str) {
		collector.new_item_found(
			DiscoveredItemId::new(id),
			DiscoveredItem::Enum {
				final_name: name.into(),
			},
			Some(&location("toolchain/include/external.h")),
		);
	}

	fn has_type(syntax: &File, name: &str) -> bool {
		syntax.items.iter().any(|item| match item {
			Item::Struct(item) => item.ident == name,
			Item::Union(item) => item.ident == name,
			Item::Enum(item) => item.ident == name,
			Item::Type(item) => item.ident == name,
			_ => false,
		})
	}

	#[test]
	fn removes_an_unused_external_alias_like_va_list() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		external_struct(&collector, 1, "__va_list_tag");
		collector.new_item_found(
			DiscoveredItemId::new(2),
			DiscoveredItem::Alias {
				alias_name: "va_list".into(),
				alias_for: DiscoveredItemId::new(1),
			},
			None,
		);
		let mut syntax: File = parse_quote! {
			pub type va_list = *mut __va_list_tag;
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(!has_type(&syntax, "va_list"));
	}

	#[test]
	fn removes_an_unused_external_enum_with_its_associated_constant() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		external_enum(&collector, 1, "ExternalEnum");
		let mut syntax: File = parse_quote! {
			pub type ExternalEnum = ::std::os::raw::c_uint;
			pub const ExternalEnum_Value: ExternalEnum = 1;
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(syntax.items.is_empty());
	}

	#[test]
	fn retains_an_external_enum_when_its_constant_also_references_a_source_type() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		external_enum(&collector, 1, "ExternalEnum");
		source_struct(&collector, 2, "SourceOwner");
		let mut syntax: File = parse_quote! {
			pub type ExternalEnum = ::std::os::raw::c_uint;
			pub struct SourceOwner;
			pub const ExternalEnum_Value: ExternalEnum = {
				let _ = ::std::mem::size_of::<SourceOwner>();
				1
			};
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(has_type(&syntax, "ExternalEnum"));
		assert_eq!(syntax.items.len(), 3);
	}

	#[test]
	fn never_removes_unused_source_types() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		for (id, name) in [
			(1, "characterset_t"),
			(2, "CKeyValuesGrowableStringTable"),
			(3, "IBaseFileSystem"),
		] {
			source_struct(&collector, id, name);
		}
		let mut syntax: File = parse_quote! {
			pub type characterset_t = u64;
			pub struct CKeyValuesGrowableStringTable { pub _unused: [u8; 0] }
			pub struct IBaseFileSystem { pub _unused: [u8; 0] }
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(has_type(&syntax, "characterset_t"));
		assert!(has_type(&syntax, "CKeyValuesGrowableStringTable"));
		assert!(has_type(&syntax, "IBaseFileSystem"));
	}

	#[test]
	fn retains_an_external_opaque_type_referenced_by_a_source_vtable() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		external_struct(&collector, 1, "bf_read");
		source_struct(&collector, 2, "IHandler__bindgen_vtable");
		let mut syntax: File = parse_quote! {
			#[repr(C)]
			pub struct bf_read { pub _unused: [u8; 0] }
			pub struct IHandler__bindgen_vtable {
				pub read: unsafe extern "C" fn(this: *mut IHandler, input: *mut bf_read),
			}
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(has_type(&syntax, "bf_read"));
	}

	#[test]
	fn retains_external_dependencies_transitively() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		source_struct(&collector, 1, "SourceOwner");
		external_struct(&collector, 2, "ExternalFirst");
		external_struct(&collector, 3, "ExternalSecond");
		external_struct(&collector, 4, "ExternalUnused");
		let mut syntax: File = parse_quote! {
			pub struct SourceOwner { pub first: *mut ExternalFirst }
			pub struct ExternalFirst { pub second: *mut ExternalSecond }
			pub struct ExternalSecond { pub value: u32 }
			pub struct ExternalUnused { pub value: u32 }
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(has_type(&syntax, "SourceOwner"));
		assert!(has_type(&syntax, "ExternalFirst"));
		assert!(has_type(&syntax, "ExternalSecond"));
		assert!(!has_type(&syntax, "ExternalUnused"));
	}

	#[test]
	fn removes_impls_and_layout_assertions_for_pruned_types() {
		let collector = ProvenanceCollector::new("sdk", ["bridge.hpp"]);
		external_struct(&collector, 1, "ExternalUnused");
		let mut syntax: File = parse_quote! {
			pub struct ExternalUnused { pub value: u32 }
			impl ExternalUnused { pub fn value(&self) -> u32 { self.value } }
			const _: () = { ::std::mem::size_of::<ExternalUnused>(); };
		};

		prune_unreferenced_external_types(&mut syntax, &collector.index());

		assert!(syntax.items.is_empty());
	}
}
