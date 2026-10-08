//! Tests of [`crate::prune`]: removing unreferenced external types from bindgen's output.

use super::*;
use crate::test_support::{found_enum, found_struct, has_type, sdk_collector};
use bindgen::callbacks::{DiscoveredItem, DiscoveredItemId, ParseCallbacks};
use syn::parse_quote;

/// A header outside the SDK root, such as the toolchain's.
const EXTERNAL_HEADER: &str = "toolchain/include/external.h";

/// A header under the SDK root.
const SOURCE_HEADER: &str = "sdk/public/source.h";

#[test]
fn never_removes_unused_source_types() {
	let collector = sdk_collector();
	for (id, name) in [
		(1, "characterset_t"),
		(2, "CKeyValuesGrowableStringTable"),
		(3, "IBaseFileSystem"),
	] {
		found_struct(&collector, id, name, SOURCE_HEADER);
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
fn removes_an_unused_external_alias_like_va_list() {
	let collector = sdk_collector();
	found_struct(&collector, 1, "__va_list_tag", EXTERNAL_HEADER);
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
	let collector = sdk_collector();
	found_enum(&collector, 1, "ExternalEnum", EXTERNAL_HEADER);
	let mut syntax: File = parse_quote! {
		pub type ExternalEnum = ::std::os::raw::c_uint;
		pub const ExternalEnum_Value: ExternalEnum = 1;
	};

	prune_unreferenced_external_types(&mut syntax, &collector.index());

	assert!(syntax.items.is_empty());
}

#[test]
fn removes_impls_and_layout_assertions_for_pruned_types() {
	let collector = sdk_collector();
	found_struct(&collector, 1, "ExternalUnused", EXTERNAL_HEADER);
	let mut syntax: File = parse_quote! {
		pub struct ExternalUnused { pub value: u32 }
		impl ExternalUnused { pub fn value(&self) -> u32 { self.value } }
		const _: () = { ::std::mem::size_of::<ExternalUnused>(); };
	};

	prune_unreferenced_external_types(&mut syntax, &collector.index());

	assert!(syntax.items.is_empty());
}

#[test]
fn retains_an_external_enum_when_its_constant_also_references_a_source_type() {
	let collector = sdk_collector();
	found_enum(&collector, 1, "ExternalEnum", EXTERNAL_HEADER);
	found_struct(&collector, 2, "SourceOwner", SOURCE_HEADER);
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
fn retains_an_external_opaque_type_referenced_by_a_source_vtable() {
	let collector = sdk_collector();
	found_struct(&collector, 1, "bf_read", EXTERNAL_HEADER);
	found_struct(&collector, 2, "IHandler__bindgen_vtable", SOURCE_HEADER);
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
	let collector = sdk_collector();
	found_struct(&collector, 1, "SourceOwner", SOURCE_HEADER);
	found_struct(&collector, 2, "ExternalFirst", EXTERNAL_HEADER);
	found_struct(&collector, 3, "ExternalSecond", EXTERNAL_HEADER);
	found_struct(&collector, 4, "ExternalUnused", EXTERNAL_HEADER);
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
