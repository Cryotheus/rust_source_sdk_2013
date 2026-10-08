//! Tests of the crate root: bindgen output guards, C++ name mapping and generated-file safety.

use super::*;

use crate::test_support::{
	TempDir, bindgen_fixture, cxx17_arguments, generated_type_names, has_type, records_in,
};

#[test]
fn full_player_headers_cannot_regress_to_opaque_records() {
	let complete: File = parse_quote! {
		pub struct CTFPlayer { pub m_Shared: CTFPlayerShared }
		pub struct CTFPlayerShared { pub m_nPlayerCond: i32, pub m_pOuter: *mut CTFPlayer }
	};
	assert!(validate_player_records(&complete).is_ok());
	let opaque: File = parse_quote! {
		pub struct CTFPlayer { pub _bindgen_opaque_blob: [u8; 128] }
		pub struct CTFPlayerShared { pub m_nPlayerCond: i32, pub m_pOuter: *mut CTFPlayer }
	};
	assert!(
		matches!(validate_player_records(&opaque), Err(BindgenError::IncompletePlayerRecord { record, .. }) if record == "CTFPlayer")
	);
}

#[test]
fn generated_manifest_paths_cannot_escape_the_output_root() {
	for invalid in [
		"",
		"/absolute.rs",
		"../outside.rs",
		"nested/../outside.rs",
		r"nested\outside.rs",
		"C:/outside.rs",
	] {
		assert!(
			generated_path_from_key(invalid).is_err(),
			"accepted {invalid:?}"
		);
	}

	assert_eq!(
		generated_path_from_key("headers/public/mod.rs").unwrap(),
		PathBuf::from("headers/public/mod.rs")
	);
}

#[test]
fn generated_write_removes_only_stale_manifest_owned_files() {
	let directory = TempDir::new();
	let root = directory.path();

	let first = GeneratedBindings {
		files: BTreeMap::from([
			(PathBuf::from("mod.rs"), "pub mod stale;\n".to_owned()),
			(
				PathBuf::from("stale/mod.rs"),
				"pub struct Stale;\n".to_owned(),
			),
		]),
		included_files: Vec::new(),
	};

	first.write(root).unwrap();
	fs::write(root.join("user-owned.txt"), "keep").unwrap();

	let second = GeneratedBindings {
		files: BTreeMap::from([(PathBuf::from("mod.rs"), "pub struct Current;\n".to_owned())]),
		included_files: Vec::new(),
	};

	second.write(root).unwrap();

	assert!(!root.join("stale/mod.rs").exists());
	assert_eq!(
		fs::read_to_string(root.join("user-owned.txt")).unwrap(),
		"keep"
	);
	assert_eq!(
		fs::read_to_string(root.join("mod.rs")).unwrap(),
		"pub struct Current;\n"
	);

	assert_eq!(
		fs::read_to_string(root.join(GENERATED_FILES_MANIFEST)).unwrap(),
		format!("{GENERATED_MANIFEST_HEADER}mod.rs\n")
	);
}

#[test]
fn item_generation_types_do_not_link_nonvirtual_engine_symbols() {
	let header = r#"
		struct CBaseEntity {};
		struct Vector {};
		struct QAngle {};
		struct CItemSelectionCriteria { int level; };
		struct baseitemcriteria_t { int iClass; int iSlot; };
		typedef int entityquality_t;
		class CAutoGameSystem { public: virtual ~CAutoGameSystem(); virtual bool Init(); };
		class CItemGeneration : public CAutoGameSystem {
		public:
			CBaseEntity *GenerateItemFromDefIndex(int, const Vector &, const QAngle &);
			CBaseEntity *GenerateBaseItem(baseitemcriteria_t *);
		};
		extern CItemGeneration *ItemGeneration();
	"#;
	let arguments = cxx17_arguments();
	let provenance = ProvenanceCollector::new(Path::new("/sdk"), [BRIDGE_FILE]);
	let bindings = binding_builder(header, &arguments, provenance, &[], false)
		.generate()
		.unwrap();
	let syntax = syn::parse_file(&bindings.to_string()).unwrap();
	assert!(reject_directly_linked_symbols(&syntax).is_ok());
	for name in [
		"CItemGeneration",
		"CItemSelectionCriteria",
		"baseitemcriteria_t",
		"entityquality_t",
	] {
		assert!(has_type(&syntax, name), "missing {name}");
	}
}

#[test]
fn maps_qualified_and_nested_cpp_records_to_bindgen_names() {
	let source = r#"
		struct Widget { virtual void global_call() = 0; };
		namespace alpha {
		struct Widget { virtual void call() = 0; };
		struct Outer {
			struct Inner { virtual int value() const = 0; };
			virtual Inner * inner() = 0;
		};
		}
		"#;

	let provenance = ProvenanceCollector::new(Path::new("."), std::iter::empty::<&Path>());

	let bindings = bindgen_fixture("qualified_record_fixture.hpp", source, &cxx17_arguments())
		.parse_callbacks(Box::new(provenance.clone()))
		.allowlist_type("Widget")
		.allowlist_type("alpha::Widget")
		.allowlist_type("alpha::Outer.*")
		.allowlist_recursively(true)
		.vtable_generation(true)
		.generate()
		.unwrap();

	let syntax = syn::parse_file(&bindings.to_string()).unwrap();
	let generated_types = generated_type_names(&syntax);

	let records = records_in("qualified_record_fixture.hpp", source);

	let provenance = provenance.index();
	let mapped = bindgen_record_name_map(&records, &provenance, &generated_types).unwrap();

	assert_eq!(mapped.get("Widget").map(String::as_str), Some("Widget"));
	assert_eq!(
		mapped.get("alpha::Widget").map(String::as_str),
		Some("alpha_Widget")
	);
	assert_eq!(
		mapped.get("alpha::Outer").map(String::as_str),
		Some("alpha_Outer")
	);
	assert_eq!(
		mapped.get("alpha::Outer::Inner").map(String::as_str),
		Some("alpha_Outer_Inner")
	);
}

#[test]
fn maps_template_members_and_union_scopes_only_to_their_own_generated_names() {
	let source = r#"
		template <class T, class I = int> struct CUtlMemory {
			struct Iterator_t { I index; };
			virtual Iterator_t First() const;
			T *m_pMemory;
		};
		template <class T, int SIZE, class I = int> struct CUtlMemoryFixed {
			struct Iterator_t { I index; };
			virtual Iterator_t First() const;
			char m_Memory[SIZE * sizeof(T)];
		};
		class CSteamID {
			union SteamID_t {
				struct SteamIDComponent_t { unsigned int m_unAccountID; } m_comp;
				unsigned long long m_unAll64Bits;
			} m_steamid;
		};
		struct Uses {
			CUtlMemory<int> memory;
			CUtlMemory<int>::Iterator_t iterator;
			CSteamID steam;
		};
		"#;

	let provenance = ProvenanceCollector::new(Path::new("."), std::iter::empty::<&Path>());
	let bindings = bindgen_fixture("template_member_fixture.hpp", source, &cxx17_arguments())
		.parse_callbacks(Box::new(provenance.clone()))
		.allowlist_type("Uses")
		.allowlist_recursively(true)
		.generate()
		.unwrap();

	let syntax = syn::parse_file(&bindings.to_string()).unwrap();
	let records = records_in("template_member_fixture.hpp", source);
	let mapped = bindgen_record_name_map(
		&records,
		&provenance.index(),
		&generated_type_names(&syntax),
	)
	.unwrap();

	assert_eq!(
		mapped.get("CUtlMemory::Iterator_t").map(String::as_str),
		Some("CUtlMemory_Iterator_t")
	);
	assert_eq!(
		mapped
			.get("CSteamID::SteamID_t::SteamIDComponent_t")
			.map(String::as_str),
		Some("CSteamID_SteamID_t_SteamIDComponent_t")
	);

	// Another template's same-named member must never borrow that name.
	assert_eq!(mapped.get("CUtlMemoryFixed::Iterator_t"), None);
}

#[test]
fn nested_typedefs_are_blamed_on_their_enclosing_cpp_record() {
	let records = records_in(
		"nested_typedef_fixture.hpp",
		r#"
		template <class T> class CUtlLinkedList {
		public:
			template <class List_t> class _CUtlLinkedList_constiterator_t { const List_t *m_list; };
			typedef _CUtlLinkedList_constiterator_t<CUtlLinkedList<T>> const_iterator;
		};
		class CUtlLinked { int head; };
		"#,
	);

	assert_eq!(
		enclosing_cpp_record(&records, "CUtlLinkedList_const_iterator").as_deref(),
		Some("CUtlLinkedList")
	);
	assert_eq!(
		enclosing_cpp_record(
			&records,
			"CUtlLinkedList__CUtlLinkedList_constiterator_t_Base"
		)
		.as_deref(),
		Some("CUtlLinkedList::_CUtlLinkedList_constiterator_t")
	);
	assert_eq!(enclosing_cpp_record(&records, "Unrelated_t"), None);
}

#[test]
fn one_pointer_interfaces_need_a_base_chain_without_data() {
	let syntax: File = parse_quote! {
		pub struct IHandleEntity { pub vtable_: *const IHandleEntity__bindgen_vtable }
		pub struct IServerUnknown { pub _base: IHandleEntity }
		pub struct IServerEntity { pub _base: IServerUnknown }
		pub struct fogparams_t { pub vtable_: *const fogparams_t__bindgen_vtable, pub start: f32 }
		pub struct CFogController_NetworkVar_m_fog { pub _base: fogparams_t }
		pub struct Generic<T> { pub _base: IHandleEntity, pub value: ::std::marker::PhantomData<T> }
		pub struct UsesGeneric { pub _base: Generic<u8> }
	};

	for interface in ["IHandleEntity", "IServerUnknown", "IServerEntity"] {
		assert!(is_one_pointer_interface(&syntax, interface), "{interface}");
	}

	for record in [
		"fogparams_t",
		"CFogController_NetworkVar_m_fog",
		"Generic",
		"UsesGeneric",
		"Missing",
	] {
		assert!(!is_one_pointer_interface(&syntax, record), "{record}");
	}
}

#[test]
fn rejects_any_direct_foreign_function_or_static() {
	let function: File = parse_quote! { unsafe extern "C" { pub fn linked(); } };
	assert!(matches!(
		reject_directly_linked_symbols(&function),
		Err(BindgenError::DirectlyLinkedSymbol)
	));

	let type_only: File = parse_quote! { unsafe extern "C" { pub type Opaque; } };
	assert!(reject_directly_linked_symbols(&type_only).is_ok());
}
