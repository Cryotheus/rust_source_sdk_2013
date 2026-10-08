//! Conservative removal of external types which bindgen emitted but no
//! retained generated item uses.

#[cfg(test)]
#[path = "tests/prune.rs"]
mod tests;

use crate::provenance::ProvenanceIndex;
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use syn::visit::Visit;
use syn::{File, Item, Type, UseTree};

#[derive(Debug)]
enum ItemRole {
	/// A struct, union, enum, type alias, or type re-export proven external.
	ExternalDefinition(BTreeSet<String>),
	/// An impl or layout assertion whose lifetime follows external types.
	ExternalAssociated(BTreeSet<String>),
	/// An item that must remain and consequently roots all of its references.
	Retained,
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
	fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
		for segment in &path.path.segments {
			self.record(&segment.ident);
		}
		syn::visit::visit_expr_path(self, path);
	}

	fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
		self.visit_use_source(&item.tree);
	}

	fn visit_macro(&mut self, macro_: &'ast syn::Macro) {
		self.visit_macro_tokens(&macro_.tokens);
	}

	fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
		for segment in &path.path.segments {
			self.record(&segment.ident);
		}
		syn::visit::visit_type_path(self, path);
	}
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

fn direct_impl_self_type_name(item: &syn::ItemImpl) -> Option<String> {
	let owner = self_type_name(&item.self_ty)?;
	let shadowed = item.generics.params.iter().any(
		|parameter| matches!(parameter, syn::GenericParam::Type(parameter) if parameter.ident == owner),
	);
	(!shadowed).then_some(owner)
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

fn external_definition_names(
	item: &Item,
	external_names: &BTreeSet<String>,
) -> Option<BTreeSet<String>> {
	let names = definition_names(item)?;
	(!names.is_empty() && names.iter().all(|name| external_names.contains(name))).then_some(names)
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

fn referenced_names(item: &Item, interesting_names: &BTreeSet<String>) -> BTreeSet<String> {
	let mut collector = ReferenceCollector {
		interesting_names,
		references: BTreeSet::new(),
	};
	collector.visit_item(item);
	collector.references
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
