//! Tests of [`crate::provenance`]: assigning bindgen's output to the headers that declare it.

use super::*;
use crate::test_support::{found_enum, found_struct, location, sdk_collector};
use syn::parse_quote;

#[test]
fn assigns_a_renamed_use_to_the_source_items_module() {
	let collector = sdk_collector();

	found_enum(&collector, 1, "_fieldtypes", "sdk/public/datamap.h");

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
fn assigns_layout_assertions_to_their_unique_referenced_type_owner() {
	let collector = sdk_collector();

	for (id, name, path) in [
		(1, "First", "sdk/public/first.h"),
		(2, "Second", "sdk/public/second.h"),
	] {
		found_struct(&collector, id, name, path);
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

#[test]
fn assigns_unreported_constants_to_the_longest_type_name_prefix() {
	let collector = sdk_collector();

	for (id, name, path) in [
		(1, "IVEngineServer", "sdk/public/engine.h"),
		(
			2,
			"IVEngineServer_eFindMapResult",
			"sdk/public/engine_result.h",
		),
	] {
		found_enum(&collector, id, name, path);
	}

	assert_eq!(
		collector
			.index()
			.module_for_generated_name("IVEngineServer_eFindMapResult_Found"),
		Some(module("public/engine_result.h"))
	);
}

#[test]
fn external_filter_only_returns_original_reachable_type_names() {
	let collector = sdk_collector();

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

fn module(path: &str) -> ModulePath {
	ModulePath::from_header_path(path).unwrap()
}

#[test]
fn only_assigns_use_groups_with_one_unambiguous_owner() {
	let collector = sdk_collector();

	for (id, name, path) in [
		(1, "First", "sdk/public/datamap.h"),
		(2, "Second", "sdk/public/datamap.h"),
		(3, "Elsewhere", "sdk/public/elsewhere.h"),
	] {
		found_struct(&collector, id, name, path);
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
fn partitions_types_impls_vtables_artifacts_and_helpers() {
	let collector = sdk_collector();

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
fn resolves_alias_and_method_ownership_through_item_ids() {
	let collector = sdk_collector();

	found_struct(&collector, 1, "Owner", "sdk/public/owner.h");

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
