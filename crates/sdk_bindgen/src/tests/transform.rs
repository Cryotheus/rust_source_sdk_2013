//! Tests of [`crate::transform`]: installing Clang-ordered vtable probes into bindgen's output.

use super::*;

#[test]
fn installs_typed_probe_and_flattens_primary_base() {
	let mut syntax: File = parse_quote! {
		pub type __crys_vtable_Derived_slot_0 = Option<unsafe extern "C" fn(this: *mut Derived)>;

		pub struct __crys_vtable_Derived {
			pub Derived_call: __crys_vtable_Derived_slot_0,
		}

		pub struct Derived { pub _base: Base }
		pub struct Derived__bindgen_vtable(::std::os::raw::c_void);
	};

	let targets = BTreeMap::from([("Derived".to_owned(), "Derived".to_owned())]);
	let installed = install_vtable_probes(&mut syntax, &targets, &["Derived"]).unwrap();
	assert_eq!(
		installed.keys().cloned().collect::<BTreeSet<_>>(),
		BTreeSet::from(["Derived".to_owned()])
	);

	let expected = BTreeMap::from([("Derived".to_owned(), installed["Derived"].clone())]);
	validate_vtables_against(&syntax, &expected, &BTreeSet::new()).unwrap();

	let rendered: proc_macro2::TokenStream = quote!(#syntax);
	let rendered = rendered.to_string();
	assert!(rendered.contains("pub vtable_ : * const Derived__bindgen_vtable"));
	assert!(rendered.contains("pub Derived_call : unsafe extern \"C\" fn"));
	assert!(!rendered.contains(PROBE_PREFIX));
	assert!(!rendered.contains("Option"));
}

#[test]
fn maps_qualified_probe_names_and_replaces_equal_length_wrong_order_tables() {
	let mut syntax: File = parse_quote! {
		pub type __crys_vtable_fixture__Derived_slot_000_type = Option<unsafe extern "C" fn(this: *mut fixture_Derived)>;
		pub type __crys_vtable_fixture__Derived_slot_001_type = Option<unsafe extern "C" fn(this: *mut fixture_Derived)>;
		pub struct __crys_vtable_fixture__Derived {
			pub fixture__Derived_first: __crys_vtable_fixture__Derived_slot_000_type,
			pub fixture__Derived_second: __crys_vtable_fixture__Derived_slot_001_type,
		}
		pub struct fixture_Derived { pub vtable_: *const fixture_Derived__bindgen_vtable }
		pub struct fixture_Derived__bindgen_vtable {
			pub fixture_Derived_second: unsafe extern "C" fn(this: *mut fixture_Derived),
			pub fixture_Derived_first: unsafe extern "C" fn(this: *mut fixture_Derived),
		}
	};

	let targets = BTreeMap::from([("fixture__Derived".to_owned(), "fixture_Derived".to_owned())]);
	let installed = install_vtable_probes(&mut syntax, &targets, &[]).unwrap();

	assert_eq!(
		installed["fixture__Derived"],
		[
			"fixture_Derived_first".to_owned(),
			"fixture_Derived_second".to_owned()
		]
	);

	let expected = BTreeMap::from([(
		"fixture_Derived".to_owned(),
		installed["fixture__Derived"].clone(),
	)]);
	validate_vtables_against(&syntax, &expected, &BTreeSet::new()).unwrap();
}

#[test]
fn rejects_unreplaced_dummy_vtable() {
	let syntax: File = parse_quote! {
		pub struct Broken__bindgen_vtable(::std::os::raw::c_void);
	};

	assert!(matches!(
		validate_vtables_against(&syntax, &BTreeMap::new(), &BTreeSet::new()),
		Err(VtableTransformError::IncompleteVtable { .. })
	));
}
