//! Tests of [`crate::cpp_vtable`]: collecting C++ records with Clang and modelling
//! their vtables under each ABI.

use super::*;
use crate::test_support::{TempDir, bindgen_fixture, cxx17_arguments, records, target_arguments};
use std::fs;

/// A class with a user-provided copy constructor, like `CBaseHandle`.
const CALL_LOWERING_FIXTURE: &str = r#"
	struct Handle {
		Handle() {}
		Handle(const Handle &other) : index(other.index) {}
		unsigned index;
	};
	struct Plain { float x, y, z; };
	struct Assigned {
		Assigned &operator=(const Assigned &) { return *this; }
		int value;
	};
	struct Owner {
		~Owner() {}
		int value;
	};
	typedef Handle HandleAlias;
	struct Interface {
		virtual bool accept(const char *name, Handle value, int id) = 0;
		virtual Plain position() const = 0;
		virtual Handle handle() = 0;
		virtual void assigned(Assigned value) = 0;
		virtual void owner(Owner value) = 0;
		virtual void by_reference(const Handle &value) = 0;
		virtual void by_const_alias(const HandleAlias value) = 0;
	};
	"#;

#[test]
fn clang_evaluates_the_special_members_which_decide_call_lowering() {
	let values = ["::Handle", "::Plain", "::Assigned", "::Owner"].map(|spelling| RecordValue {
		canonical_spelling: spelling.to_owned(),
		unqualified_spelling: spelling.to_owned(),
	});

	for target in ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"] {
		let traits = query_record_call_traits(
			"call_traits_fixture.cpp",
			CALL_LOWERING_FIXTURE,
			&target_arguments(target),
			&values,
		)
		.unwrap();

		// Without a move constructor, `T &&` selects the copy constructor.
		assert_eq!(
			traits["::Handle"],
			RecordCallTraits {
				copy_constructible: true,
				trivial_copy_constructor: false,
				move_constructible: true,
				trivially_move_constructible: false,
				trivially_destructible: true,
				size: 4,
			}
		);
		assert_eq!(
			traits["::Plain"],
			RecordCallTraits {
				copy_constructible: true,
				trivial_copy_constructor: true,
				move_constructible: true,
				trivially_move_constructible: true,
				trivially_destructible: true,
				size: 12,
			}
		);
		assert!(!traits["::Handle"].itanium_passes_as_c_struct());
		assert!(!traits["::Handle"].msvc_x64_passes_as_c_struct());
		assert!(traits["::Plain"].itanium_passes_as_c_struct());
		assert!(traits["::Plain"].msvc_x64_passes_as_c_struct());

		// A copy assignment operator never affects how a class is passed.
		assert!(traits["::Assigned"].itanium_passes_as_c_struct());
		assert!(traits["::Assigned"].msvc_x64_passes_as_c_struct());

		// MSVC passes a register-sized class with a destructor directly.
		assert!(traits["::Owner"].trivial_copy_constructor);
		assert!(!traits["::Owner"].trivially_destructible);
		assert!(!traits["::Owner"].itanium_passes_as_c_struct());
		assert!(traits["::Owner"].msvc_x64_passes_as_c_struct());
	}
}

#[test]
fn collects_inline_virtual_method_metadata() {
	let records = records(
		r#"
		namespace fixture {
		class Inline {
		public:
			virtual int value(double amount, ...) const { return static_cast<int>(amount); }
		};
		}
		"#,
	);
	let record = records.record("fixture::Inline").unwrap();
	assert_eq!(record.name, "Inline");
	assert!(
		record
			.source_path
			.as_ref()
			.unwrap()
			.ends_with("cpp_vtable_fixture.cpp")
	);
	assert!(record.bases.is_empty());
	assert_eq!(record.virtual_methods.len(), 1);

	let method = &record.virtual_methods[0];
	assert_eq!(method.name, "value");
	assert_eq!(method.qualified_name, "fixture::Inline::value");
	assert_eq!(method.kind, VirtualMethodKind::Method);
	assert!(method.is_const);
	assert_eq!(method.result_type, "int");
	assert_eq!(
		method.parameters,
		[Parameter {
			name: "amount".to_owned(),
			type_spelling: "double".to_owned(),
			canonical_type: "double".to_owned(),
			record: None,
		}]
	);
	assert!(method.is_variadic);
	assert!(method.overrides.is_empty());
}

#[test]
fn derived_destructor_replaces_the_inherited_destructor_group() {
	let index = records(
		r#"
		struct Base {
			virtual ~Base() = default;
		};
		struct Derived : Base {
			~Derived() override = default;
		};
		"#,
	);
	let derived = index.record("Derived").unwrap();
	assert_eq!(derived.virtual_methods.len(), 1);
	assert_eq!(
		derived.virtual_methods[0].kind,
		VirtualMethodKind::Destructor
	);

	let layout = index.vtable_layout("Derived", CppAbi::Itanium).unwrap();
	assert_eq!(layout.slots.len(), 2);
	assert!(
		layout
			.slots
			.iter()
			.all(|slot| slot.method.qualified_name == "Derived::~Derived")
	);
}

#[test]
fn distinguishes_nonpolymorphic_inheritance_from_inherited_virtual_slots() {
	let records = records(
		r#"
		struct PlainLeft {};
		struct PlainRight {};
		struct PlainMultiple : PlainLeft, PlainRight {};
		struct PlainVirtual : virtual PlainLeft {};
		struct PlainChain : PlainMultiple {};
		struct Polymorphic { virtual void call() = 0; };
		struct InheritedPolymorphic : PlainLeft, Polymorphic {};
		"#,
	);

	for record in [
		"PlainLeft",
		"PlainRight",
		"PlainMultiple",
		"PlainVirtual",
		"PlainChain",
	] {
		assert!(
			!records.has_virtual_slots(record).unwrap(),
			"{record} unexpectedly has virtual slots"
		);
	}
	assert!(records.has_virtual_slots("Polymorphic").unwrap());
	assert!(records.has_virtual_slots("InheritedPolymorphic").unwrap());
}

#[test]
fn globally_qualifies_nested_result_and_parameter_types() {
	let fixture = r#"
		struct Outer {
			enum Result { Found, Missing };
			using ResultAlias = Result;
			virtual Result find(ResultAlias previous, const Result *fallback) = 0;
		};
		"#;
	let index = records(fixture);
	let method = &index.record("Outer").unwrap().virtual_methods[0];
	assert_eq!(method.result_type, "::Outer::Result");
	assert_eq!(method.parameters[0].type_spelling, "::Outer::ResultAlias");
	assert_eq!(
		method.parameters[1].type_spelling,
		"const ::Outer::Result *"
	);

	let probe = probe(&index, "Outer", CppAbi::Itanium);
	let _ = records(&format!("{fixture}\n{}", probe.source));
}

#[test]
fn itanium_passes_non_trivial_classes_by_pointer_and_returns_them_before_this() {
	let probe = lowered_probe(
		CALL_LOWERING_FIXTURE,
		"Interface",
		"x86_64-unknown-linux-gnu",
		CppAbi::Itanium,
	);

	assert_eq!(
		slot_alias(&probe, "Interface_accept"),
		"auto (*)(::Interface *, const char *, ::Handle *, int) -> bool"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_position"),
		"auto (*)(const ::Interface *) -> ::Plain"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_handle"),
		"auto (*)(::Handle *, ::Interface *) -> ::Handle *"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_assigned"),
		"auto (*)(::Interface *, ::Assigned) -> void"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_owner"),
		"auto (*)(::Interface *, ::Owner *) -> void"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_by_reference"),
		"auto (*)(::Interface *, const ::Handle &) -> void"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_by_const_alias"),
		"auto (*)(::Interface *, ::HandleAlias *) -> void"
	);
	assert!(probe.source.contains(
		"    /// C++ takes `::Handle` by value. `arg3` points to a temporary copy the caller \
		 makes; the callee may modify it, and the caller destroys it after the call.\n    \
		 __crys_vtable_Interface_slot_000_type Interface_accept;"
	));
	assert!(
		probe.source.contains(
			"    /// C++ returns `::Handle` by value. `arg1` is the hidden result pointer"
		)
	);
}

#[test]
fn itanium_x86_returns_classes_as_c_structs() {
	let probe = lowered_probe(
		CALL_LOWERING_FIXTURE,
		"Interface",
		"i686-unknown-linux-gnu",
		CppAbi::ItaniumX86,
	);

	assert_eq!(
		slot_alias(&probe, "Interface_accept"),
		"auto (*)(::Interface *, const char *, ::Handle *, int) -> bool"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_handle"),
		"auto (*)(::Interface *) -> ::Handle"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_owner"),
		"auto (*)(::Interface *, ::Owner *) -> void"
	);
}

#[test]
fn keys_explicit_specializations_by_their_template_arguments() {
	let records = records(
		r#"
		enum fieldtype_t { FIELD_INTEGER, FIELD_VECTOR, FIELD_POSITION_VECTOR };
		class Vector { public: float x, y, z; };
		class CBaseEntityOutput { public: ~CBaseEntityOutput(); protected: int m_Value; };

		template <class Type, fieldtype_t fieldType>
		class CEntityOutputTemplate : public CBaseEntityOutput {
		public:
			void Init(Type value) {}
			class Iterator { int index; };
		};

		template <>
		class CEntityOutputTemplate<class Vector, FIELD_VECTOR> : public CBaseEntityOutput {
		public:
			void Get(Vector &vec) {}
			class Iterator { virtual void next(); };
		};

		template <>
		class CEntityOutputTemplate<class Vector, FIELD_POSITION_VECTOR> : public CBaseEntityOutput {
		public:
			void Get(Vector &vec) {}
		};

		typedef CEntityOutputTemplate<int, FIELD_INTEGER> COutputInt;
		typedef CEntityOutputTemplate<Vector, FIELD_VECTOR> COutputVector;

		template class CEntityOutputTemplate<float, FIELD_INTEGER>;
		"#,
	);

	let roles = [
		("CEntityOutputTemplate", TemplateRole::Pattern),
		(
			"CEntityOutputTemplate::Iterator",
			TemplateRole::DependentMember,
		),
		(
			"CEntityOutputTemplate<class Vector, FIELD_VECTOR>",
			TemplateRole::ExplicitSpecialization,
		),
		(
			"CEntityOutputTemplate<class Vector, FIELD_VECTOR>::Iterator",
			TemplateRole::None,
		),
		(
			"CEntityOutputTemplate<class Vector, FIELD_POSITION_VECTOR>",
			TemplateRole::ExplicitSpecialization,
		),
		(
			"CEntityOutputTemplate<float, FIELD_INTEGER>",
			TemplateRole::ExplicitInstantiation,
		),
		("CBaseEntityOutput", TemplateRole::None),
	];

	for (record, role) in roles {
		assert_eq!(
			records.record(record).map(|record| record.template),
			Some(role),
			"{record}"
		);
	}

	for record in [
		"CEntityOutputTemplate",
		"CEntityOutputTemplate<class Vector, FIELD_VECTOR>",
		"CBaseEntityOutput",
	] {
		assert_eq!(
			records.vtable_model(record, CppAbi::Itanium).unwrap(),
			VtableModel::NotPolymorphic,
			"{record}"
		);
	}

	// A member of an explicit specialization is a concrete record.
	assert_eq!(
		slot_names(
			&records,
			"CEntityOutputTemplate<class Vector, FIELD_VECTOR>::Iterator",
			CppAbi::Msvc
		),
		["next()"]
	);
}

/// Renders `record`'s probe with the call traits Clang evaluates for
/// `target`, and checks that the probe is still valid C++ there.
fn lowered_probe(source: &str, record: &str, target: &str, abi: CppAbi) -> VtableProbe {
	const FILE: &str = "call_lowering_fixture.cpp";

	let arguments = target_arguments(target);
	let index = collect_records(FILE, source, &arguments).unwrap();
	let layout = index.vtable_layout(record, abi).unwrap();
	let traits =
		query_record_call_traits(FILE, source, &arguments, layout.by_value_records()).unwrap();
	let probe = render_vtable_probe(&index, &layout, &traits).unwrap();

	collect_records(FILE, &format!("{source}\n{}", probe.source), &arguments).unwrap();

	probe
}

/// Expected orders are Clang's own `-fdump-vtable-layouts` output for
/// `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`.
#[test]
fn microsoft_abi_groups_and_reverses_new_overloads() {
	let fixture = r#"
		struct Vector { float x, y, z; };
		struct IServerToolsLike {
			virtual ~IServerToolsLike();
			virtual bool GetKeyValue(const char *field) = 0;
			virtual bool SetKeyValue(const char *field, const char *value) = 0;
			virtual bool SetKeyValue(const char *field, float value) = 0;
			virtual bool SetKeyValue(const char *field, const Vector &value) = 0;
			virtual void Other() = 0;
		};

		struct S {
			void f(double);
			virtual void g();
			virtual void f(int);
			virtual ~S();
			virtual void a();
			int h;
			virtual void h2();
		};

		struct B { virtual void f(int); virtual void z(); };
		struct D : B {
			virtual void f(double);
			virtual void y();
			virtual void f(char);
			void f(int) override;
			using B::z;
			virtual void z(int);
		};
		"#;
	let index = records(fixture);

	assert_eq!(
		slot_names(&index, "IServerToolsLike", CppAbi::Msvc),
		[
			"~scalar",
			"GetKeyValue(const char *)",
			"SetKeyValue(const char *, const Vector &)",
			"SetKeyValue(const char *, float)",
			"SetKeyValue(const char *, const char *)",
			"Other()",
		]
	);
	assert_eq!(
		slot_names(&index, "IServerToolsLike", CppAbi::Itanium),
		[
			"~complete",
			"~deleting",
			"GetKeyValue(const char *)",
			"SetKeyValue(const char *, const char *)",
			"SetKeyValue(const char *, float)",
			"SetKeyValue(const char *, const Vector &)",
			"Other()",
		]
	);

	assert_eq!(
		slot_names(&index, "S", CppAbi::Msvc),
		["f(int)", "g()", "~scalar", "a()", "h2()"]
	);
	assert_eq!(
		slot_names(&index, "S", CppAbi::Itanium),
		["g()", "f(int)", "~complete", "~deleting", "a()", "h2()"]
	);

	assert_eq!(
		slot_names(&index, "D", CppAbi::Msvc),
		["f(int)", "z()", "f(char)", "f(double)", "y()", "z(int)"]
	);
	assert_eq!(
		slot_names(&index, "D", CppAbi::Itanium),
		["f(int)", "z()", "f(double)", "y()", "f(char)", "z(int)"]
	);

	let _ = records(&format!(
		"{fixture}\n{}",
		probe(&index, "D", CppAbi::MsvcX86).source
	));
}

#[test]
fn models_primary_address_point_of_multiple_bases_but_refuses_virtual_bases() {
	let records = records(
		r#"
		struct Left { virtual void left() = 0; };
		struct Right { virtual void right() = 0; };
		struct Multiple : Left, Right {};
		struct Virtual : virtual Left {};
		struct PlainBase { int value; };
		struct DataFirst : PlainBase, Left { virtual void own() = 0; };
		"#,
	);

	for record in ["Virtual"] {
		for abi in [CppAbi::Itanium, CppAbi::Msvc] {
			assert!(
				matches!(
					records.vtable_model(record, abi).unwrap(),
					VtableModel::Unsupported { .. }
				),
				"{record} {abi:?}"
			);
		}
	}

	for abi in [CppAbi::Itanium, CppAbi::Msvc] {
		assert_eq!(slot_names(&records, "Multiple", abi), ["left()"]);
	}

	let virtual_base = records
		.vtable_layout("Virtual", CppAbi::Itanium)
		.unwrap_err();
	assert!(
		matches!(virtual_base, CppVtableError::Unsupported { ref reason, .. } if reason.contains("virtual base"))
	);

	// A single polymorphic base supplies the primary vtable in both ABIs,
	// even after a non-polymorphic base.
	let data_first = records.vtable_layout("DataFirst", CppAbi::Itanium).unwrap();
	assert_eq!(
		data_first
			.slots
			.iter()
			.map(|slot| slot.method.qualified_name.as_str())
			.collect::<Vec<_>>(),
		["Left::left", "DataFirst::own"]
	);
}

#[test]
fn msvc_x64_passes_non_trivial_classes_by_pointer_and_returns_every_class_after_this() {
	let probe = lowered_probe(
		CALL_LOWERING_FIXTURE,
		"Interface",
		"x86_64-pc-windows-msvc",
		CppAbi::Msvc,
	);

	assert_eq!(
		slot_alias(&probe, "Interface_accept"),
		"auto (*)(::Interface *, const char *, ::Handle *, int) -> bool"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_position"),
		"auto (*)(const ::Interface *, ::Plain *) -> ::Plain *"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_handle"),
		"auto (*)(::Interface *, ::Handle *) -> ::Handle *"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_assigned"),
		"auto (*)(::Interface *, ::Assigned) -> void"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_owner"),
		"auto (*)(::Interface *, ::Owner) -> void"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_by_const_alias"),
		"auto (*)(::Interface *, ::HandleAlias *) -> void"
	);
	assert!(probe.source.contains(
		"the callee may modify it, and the callee destroys it.\n    \
		 __crys_vtable_Interface_slot_000_type Interface_accept;"
	));

	let source = format!("{CALL_LOWERING_FIXTURE}\n{}", probe.source);
	let bindings = bindgen_fixture(
		"call_lowering_bindgen_fixture.hpp",
		&source,
		&target_arguments("x86_64-pc-windows-msvc"),
	)
	.allowlist_type("__crys_vtable_Interface")
	.generate()
	.unwrap()
	.to_string()
	.split_whitespace()
	.collect::<String>();

	assert!(
		bindings.contains("arg1:*constInterface,arg2:*mutPlain)->*mutPlain"),
		"generated bindings:\n{bindings}"
	);
	assert!(
		bindings.contains("isthehiddenresultpointer"),
		"generated bindings:\n{bindings}"
	);
}

#[test]
fn msvc_x86_passes_classes_in_place_and_returns_them_after_this() {
	let probe = lowered_probe(
		CALL_LOWERING_FIXTURE,
		"Interface",
		"i686-pc-windows-msvc",
		CppAbi::MsvcX86,
	);

	assert_eq!(
		slot_alias(&probe, "Interface_accept"),
		"auto (__thiscall *)(::Interface *, const char *, ::Handle, int) -> bool"
	);
	assert_eq!(
		slot_alias(&probe, "Interface_position"),
		"auto (__thiscall *)(const ::Interface *, ::Plain *) -> ::Plain *"
	);
}

#[test]
fn overload_field_names_follow_declaration_order_under_every_abi() {
	let records = records(
		r#"
		struct Vector { float x, y, z; };
		struct Tools {
			virtual bool SetKeyValue(const char *field, const char *value) = 0;
			virtual bool SetKeyValue(const char *field, float value) = 0;
			virtual bool SetKeyValue(const char *field, const Vector &value) = 0;
		};
		struct MoreTools : Tools { virtual bool SetKeyValue(const char *field, int value) = 0; };
		"#,
	);

	let named = |abi| {
		let probe = probe(&records, "MoreTools", abi);
		probe
			.field_names
			.iter()
			.cloned()
			.zip(
				probe
					.layout
					.slots
					.iter()
					.map(|slot| slot.method.display_name.clone()),
			)
			.collect::<BTreeMap<_, _>>()
	};

	let itanium = named(CppAbi::Itanium);
	assert_eq!(itanium, named(CppAbi::Msvc));
	assert_eq!(
		itanium["MoreTools_SetKeyValue"],
		"SetKeyValue(const char *, const char *)"
	);
	assert_eq!(
		itanium["MoreTools_SetKeyValue2"],
		"SetKeyValue(const char *, const Vector &)"
	);
	assert_eq!(
		itanium["MoreTools_SetKeyValue3"],
		"SetKeyValue(const char *, int)"
	);

	assert_eq!(
		probe(&records, "MoreTools", CppAbi::Msvc).field_names,
		[
			"MoreTools_SetKeyValue2",
			"MoreTools_SetKeyValue1",
			"MoreTools_SetKeyValue",
			"MoreTools_SetKeyValue3"
		]
	);
}

fn probe(records: &RecordIndex, record: &str, abi: CppAbi) -> VtableProbe {
	render_vtable_probe(
		records,
		&records.vtable_layout(record, abi).unwrap(),
		&RecordCallTraitIndex::new(),
	)
	.unwrap()
}

#[test]
fn records_typedef_names_of_unnamed_records() {
	let records = records(
		r#"
		typedef union { char size[40]; long align; } pthread_mutex_t;
		typedef struct { int x; } plain_anon_t;
		typedef struct named_s { int y; } named_t;
		namespace ns { typedef struct { int z; } nested_t; }
		"#,
	);

	assert_eq!(
		records
			.typedef_named_anonymous_records()
			.iter()
			.map(String::as_str)
			.collect::<Vec<_>>(),
		["nested_t", "plain_anon_t", "pthread_mutex_t"]
	);
}

#[test]
fn rendered_probe_does_not_pollute_global_type_lookup() {
	let fixture = r#"
		using int64 = long long;
		namespace fixture {
		using int64 = long;
		struct Typed {
			virtual int64 exchange(int64 value) = 0;
		};
		}
		"#;
	let index = records(fixture);
	let probe = probe(&index, "fixture::Typed", CppAbi::Itanium);
	assert!(probe.source.contains("::fixture::int64"));
	let _ = records(&format!(
		"{fixture}\n{}\nstruct AfterProbe {{ int64 value; }};",
		probe.source
	));
}

#[test]
fn rendered_probe_keeps_user_type_spellings_valid_at_global_scope() {
	let fixture = r#"
		namespace fixture {
		struct Argument {};
		struct Typed {
			virtual Argument * exchange(const Argument & value) = 0;
		};
		}
		"#;
	let index = records(fixture);
	let method = &index.record("fixture::Typed").unwrap().virtual_methods[0];
	assert_eq!(method.result_type, "::fixture::Argument *");
	assert_eq!(
		method.parameters[0].type_spelling,
		"const ::fixture::Argument &"
	);

	let probe = probe(&index, "fixture::Typed", CppAbi::Itanium);
	assert!(
		probe
			.source
			.starts_with("namespace __crys_vtable_fixture__Typed_scope {")
	);
	let _ = records(&format!("{fixture}\n{}", probe.source));
}

#[test]
fn rendering_a_by_value_class_without_its_call_traits_fails() {
	let index = records(CALL_LOWERING_FIXTURE);
	let layout = index.vtable_layout("Interface", CppAbi::Itanium).unwrap();

	assert!(matches!(
		render_vtable_probe(&index, &layout, &RecordCallTraitIndex::new()),
		Err(CppVtableError::MissingCallTraits { ref record }) if record == "::Handle"
	));
}

#[test]
fn renders_msvc_x86_slots_with_thiscall() {
	let fixture = r#"
		struct Win32Interface {
			virtual int call(double value) const = 0;
			virtual void log(const char *message, ...) = 0;
			virtual ~Win32Interface() = default;
		};
		"#;
	let index = records(fixture);
	let probe = probe(&index, "Win32Interface", CppAbi::MsvcX86);

	assert!(
		probe
			.source
			.contains("auto (__thiscall *)(const ::Win32Interface *, double) -> int;")
	);
	assert!(
		probe
			.source
			.contains("auto (__cdecl *)(::Win32Interface *, const char *, ...) -> void;")
	);
	assert!(
		probe
			.source
			.contains("auto (__thiscall *)(::Win32Interface *, unsigned int) -> void *;")
	);
	assert_eq!(
		probe.field_names,
		[
			"Win32Interface_call",
			"Win32Interface_log",
			"Win32Interface_destructor"
		]
	);

	let source = format!("{fixture}\n{}", probe.source);
	let mut arguments = target_arguments("i686-pc-windows-msvc");

	arguments.push("-fms-extensions".to_owned());

	let bindings = bindgen_fixture("cpp_vtable_win32_fixture.hpp", &source, &arguments)
		.allowlist_type("__crys_vtable_Win32Interface")
		.generate()
		.unwrap()
		.to_string();

	assert!(
		bindings.contains("extern \"thiscall\" fn"),
		"generated bindings:\n{bindings}"
	);
	assert!(
		bindings.contains("extern \"C\" fn"),
		"generated bindings:\n{bindings}"
	);
}

#[test]
fn renders_typed_destructor_slots_for_both_abis() {
	let fixture = r#"
		struct Destroyed {
			virtual ~Destroyed() = default;
		};
		"#;
	let index = records(fixture);
	assert_eq!(vtable_probe_stem("Destroyed"), "__crys_vtable_Destroyed");

	let itanium = probe(&index, "Destroyed", CppAbi::Itanium);
	assert!(itanium.source.contains(
		"using __crys_vtable_Destroyed_slot_000_type = auto (*)(::Destroyed *) -> void;"
	));
	assert!(itanium.source.contains("Destroyed_complete_destructor"));
	assert!(itanium.source.contains("Destroyed_deleting_destructor"));

	let msvc = probe(&index, "Destroyed", CppAbi::Msvc);
	assert!(msvc.source.contains(
		"using __crys_vtable_Destroyed_slot_000_type = auto (*)(::Destroyed *, unsigned int) -> void *;"
	));
	assert!(msvc.source.contains("Destroyed_destructor"));

	// The probe is C++ source, rather than only a diagnostic description.
	// Have libclang parse both renderings to guard their function-pointer syntax.
	let _ = records(&format!("{fixture}\n{}", itanium.source));
	let _ = records(&format!("{fixture}\n{}", msvc.source));
}

#[test]
fn replaces_primary_base_slot_for_override_and_appends_new_virtual() {
	let records = records(
		r#"
		namespace fixture {
		struct Base {
			virtual int keep(double value) = 0;
			virtual int replace(int value) = 0;
		};
		struct Derived : Base {
			int replace(int value) override { return value; }
			virtual void added() = 0;
		};
		}
		"#,
	);
	let derived = records.record("fixture::Derived").unwrap();
	assert_eq!(
		derived.bases,
		[BaseRecord {
			qualified_name: "fixture::Base".to_owned(),
			type_spelling: "Base".to_owned(),
			is_virtual: false,
			target: BaseTarget::Record,
		}]
	);
	assert_eq!(derived.virtual_methods[0].name, "replace");
	assert_eq!(derived.virtual_methods[0].overrides.len(), 1);
	assert_eq!(
		derived.virtual_methods[0].overrides[0].qualified_name,
		"fixture::Base::replace"
	);

	let layout = records
		.vtable_layout("fixture::Derived", CppAbi::Itanium)
		.unwrap();
	assert_eq!(
		layout
			.slots
			.iter()
			.map(|slot| slot.method.qualified_name.as_str())
			.collect::<Vec<_>>(),
		[
			"fixture::Base::keep",
			"fixture::Derived::replace",
			"fixture::Derived::added"
		]
	);

	let probe = probe(&records, "fixture::Derived", CppAbi::Itanium);
	assert_eq!(probe.stem, "__crys_vtable_fixture__Derived");
	assert!(probe.source.contains("::fixture::Derived *"));
	assert!(probe.source.contains("fixture__Derived_keep"));
	assert!(probe.source.contains("fixture__Derived_replace"));
	assert!(probe.source.contains("fixture__Derived_added"));
}

#[test]
fn reports_covariant_overriders_as_unsupported() {
	let records = records(
		r#"
		struct Result { int value; };
		struct Derived : Result { virtual void other(); };
		struct Base { virtual Result *get(); };
		struct Covariant : Base { Derived *get() override; };
		"#,
	);

	assert!(matches!(
		records.vtable_model("Covariant", CppAbi::Itanium).unwrap(),
		VtableModel::Unsupported { ref reason } if reason.contains("covariant")
	));
}

#[test]
fn reports_template_dependent_vtables_as_unsupported() {
	let records = records(
		r#"
		struct Plain { int value; };
		template <class T> struct Factory { virtual T *create() = 0; };
		struct UsesFactory : Factory<Plain> { Plain *create() override; };

		template <class Base> struct Mixin : Base { };
		struct UsesMixin : Mixin<Plain> { virtual void run(); };

		template <class T> struct Outer { struct Inner { virtual void run(); T value; }; };
		"#,
	);

	for record in ["UsesFactory", "Factory", "Outer::Inner"] {
		assert!(
			matches!(
				records.vtable_model(record, CppAbi::Itanium).unwrap(),
				VtableModel::Unsupported { .. }
			),
			"{record}"
		);
	}
	assert_eq!(
		slot_names(&records, "UsesMixin", CppAbi::Itanium),
		["run()"]
	);

	assert_eq!(
		records.vtable_model("Outer", CppAbi::Itanium).unwrap(),
		VtableModel::NotPolymorphic
	);
}

#[test]
fn resolves_bases_through_non_polymorphic_template_instantiations() {
	let records = records(
		r#"
		struct IFace {
			virtual ~IFace() {}
			virtual void run() = 0;
		};

		// Source's `CNetworkVarBase` shape: a non-polymorphic CRTP template.
		template <class Type, class Changer> class NetworkVarBase { protected: Type m_Value; };

		// A template adding data, but no virtual functions, to a concrete base.
		template <class T> class Holder : public IFace { public: T value; };

		class Entity : public IFace {
		public:
			class NetworkVar_health : public NetworkVarBase<int, NetworkVar_health> {
			public:
				virtual void changed();
			};

			void run() override;
			virtual void think();
		};

		class Held : public Holder<int> {
		public:
			void run() override;
			virtual void extra();
		};

		template class Holder<float>;
		class HeldExplicit : public Holder<float> { public: virtual void extra(); };
		"#,
	);

	assert_eq!(
		slot_names(&records, "Entity::NetworkVar_health", CppAbi::Itanium),
		["changed()"]
	);
	assert_eq!(
		slot_names(&records, "Held", CppAbi::Itanium),
		["~complete", "~deleting", "run()", "extra()"]
	);
	assert_eq!(
		slot_names(&records, "Held", CppAbi::Msvc),
		["~scalar", "run()", "extra()"]
	);
	assert_eq!(
		slot_names(&records, "HeldExplicit", CppAbi::Msvc),
		["~scalar", "run()", "extra()"]
	);
	assert_eq!(
		records
			.vtable_layout("Held", CppAbi::Itanium)
			.unwrap()
			.slots[2]
			.method
			.qualified_name,
		"Held::run"
	);

	// The template itself cannot have a probe: its layout is generic.
	assert!(matches!(
		records.vtable_model("Holder", CppAbi::Itanium).unwrap(),
		VtableModel::Unsupported { .. }
	));
}

#[test]
fn scoped_aliases_resolve_relative_names_and_keep_probe_transform_stable() {
	let fixture = r#"
		using int64 = long long;
		template <class T> struct Vector {};
		namespace fixture {
		using int64 = long;
		namespace io { struct Stream {}; }
		struct Field {};
		struct Typed {
			virtual void parse(io::Stream *, Vector<Field *> *) = 0;
		};
		}
		"#;
	let index = records(fixture);
	let probe = probe(&index, "fixture::Typed", CppAbi::Itanium);
	let source = format!(
		"{fixture}\n{}\nstruct AfterProbe {{ int64 value; }};",
		probe.source
	);
	let _ = records(&source);
	let bindings = bindgen_fixture(
		"scoped_vtable_alias_fixture.hpp",
		&source,
		&cxx17_arguments(),
	)
	.allowlist_type("__crys_vtable_fixture__Typed")
	.generate()
	.unwrap()
	.to_string();
	let mut syntax = syn::parse_file(&bindings).unwrap();
	let targets = BTreeMap::from([("fixture__Typed".to_owned(), "fixture_Typed".to_owned())]);
	let installed = crate::transform::install_vtable_probes(&mut syntax, &targets, &[]).unwrap();
	assert_eq!(installed["fixture__Typed"], ["fixture_Typed_parse"]);
	assert!(
		!quote::quote!(#syntax)
			.to_string()
			.contains("__crys_vtable_")
	);
}

#[test]
fn secondary_overrides_and_implicit_destructors_follow_clang_abi_layouts() {
	// Verified with clang -Xclang -fdump-vtable-layouts on both targets.
	let records = records(
		r#"
	struct Left { virtual void left(); };
	struct Right { virtual ~Right(); virtual void right(); };
	struct Both : Left, Right { virtual void own(); };
	struct Over : Left, Right { void right() override; virtual void own(); };
	struct Last : Over { void right() override; };
	struct Explicit : Left, Right { ~Explicit() override; virtual void own(); };
	"#,
	);
	assert_eq!(
		slot_names(&records, "Both", CppAbi::Msvc),
		["left()", "own()"]
	);
	assert_eq!(
		slot_names(&records, "Both", CppAbi::Itanium),
		["left()", "own()", "~complete", "~deleting"]
	);
	assert_eq!(
		slot_names(&records, "Over", CppAbi::Msvc),
		["left()", "own()"]
	);
	assert_eq!(
		slot_names(&records, "Over", CppAbi::Itanium),
		["left()", "right()", "own()", "~complete", "~deleting"]
	);
	assert_eq!(
		slot_names(&records, "Last", CppAbi::Msvc),
		["left()", "own()"]
	);
	assert_eq!(
		slot_names(&records, "Last", CppAbi::Itanium),
		["left()", "right()", "own()", "~complete", "~deleting"]
	);
	assert_eq!(
		slot_names(&records, "Explicit", CppAbi::Msvc),
		["left()", "own()"]
	);
	assert_eq!(
		slot_names(&records, "Explicit", CppAbi::Itanium),
		["left()", "~complete", "~deleting", "own()"]
	);
}

/// The alias of the slot named `field`, from `using` to `;`.
fn slot_alias<'a>(probe: &'a VtableProbe, field: &str) -> &'a str {
	let position = probe
		.field_names
		.iter()
		.position(|name| name == field)
		.unwrap_or_else(|| panic!("no slot {field:?} in {:?}", probe.field_names));
	let alias = format!("using {}_slot_{position:03}_type = ", probe.stem);
	let start = probe.source.find(&alias).unwrap() + alias.len();
	let end = start + probe.source[start..].find(";\n").unwrap();

	&probe.source[start..end]
}

fn slot_names(records: &RecordIndex, record: &str, abi: CppAbi) -> Vec<String> {
	records
		.vtable_layout(record, abi)
		.unwrap()
		.slots
		.iter()
		.map(|slot| match slot.kind {
			VtableSlotKind::Method => slot.method.display_name.clone(),
			VtableSlotKind::ItaniumCompleteDestructor => "~complete".to_owned(),
			VtableSlotKind::ItaniumDeletingDestructor => "~deleting".to_owned(),
			VtableSlotKind::MsvcScalarDeletingDestructor => "~scalar".to_owned(),
		})
		.collect()
}

#[test]
fn source_root_filter_excludes_external_records_but_keeps_sdk_conflicts_visible() {
	let fixture = TempDir::new();
	let sdk_root = fixture.path().join("sdk").join("src");
	let external_root = fixture.path().join("toolchain");
	fs::create_dir_all(&sdk_root).unwrap();
	fs::create_dir_all(&external_root).unwrap();
	fs::write(
		external_root.join("external.hpp"),
		r#"
		template <typename T> struct Specialized {};
		template <> struct Specialized<int> { virtual void one() = 0; };
		template <> struct Specialized<double> { virtual int two() = 0; };
		struct ExternalInterface { virtual void ignored() = 0; };
		"#,
	)
	.unwrap();

	let file_name = sdk_root.join("fixture.cpp");
	let mut clang_args = cxx17_arguments();

	clang_args.push(format!("-I{}", external_root.display()));

	let source = r#"
		#include "external.hpp"
		struct SdkInterface { virtual void kept() = 0; };
		"#;

	let unfiltered = collect_records(&file_name, source, &clang_args).unwrap();
	assert_eq!(
		unfiltered.record("Specialized<int>").unwrap().template,
		TemplateRole::ExplicitSpecialization
	);
	assert_eq!(
		unfiltered.record("Specialized<double>").unwrap().template,
		TemplateRole::ExplicitSpecialization
	);
	assert!(unfiltered.record("ExternalInterface").is_some());

	let filtered = collect_records_under_root(&file_name, source, &clang_args, &sdk_root).unwrap();
	assert!(filtered.record("SdkInterface").is_some());
	assert!(filtered.record("ExternalInterface").is_none());
	assert!(filtered.record("Specialized").is_none());
	assert!(filtered.record("Specialized<int>").is_none());

	// Declarations in an anonymous namespace share the enclosing scope's
	// names, so these genuinely conflicting definitions still collide.
	let conflicting_sdk = r#"
		namespace { struct Collision { virtual void one() = 0; }; }
		struct Collision { virtual int two() = 0; };
		"#;
	assert!(matches!(
		collect_records_under_root(&file_name, conflicting_sdk, &clang_args, &sdk_root),
		Err(CppVtableError::DuplicateRecord(ref name)) if name == "Collision"
	));
}

#[test]
fn specialized_implicit_virtual_declarations_keep_their_original_order() {
	let records = records(
		r#"
	struct Left { virtual void left(); };
	struct Right { virtual ~Right(); virtual void right(); };
	template<class Base> struct Host : Left, Base { ~Host(); void right(); virtual void own(); };
	struct Concrete : Host<Right> {};
	"#,
	);
	assert_eq!(
		slot_names(&records, "Concrete", CppAbi::Msvc),
		["left()", "own()"]
	);
	assert_eq!(
		slot_names(&records, "Concrete", CppAbi::Itanium),
		["left()", "~complete", "~deleting", "right()", "own()"]
	);
}

#[test]
fn specialized_secondary_overrides_match_instantiated_clang_references() {
	let index = records(
		r#"
		struct Left { virtual void left(); };
		struct Right { virtual void right(); };
		template <class Base> struct Host : Base { void right(); };
		struct Combined : Left, Host<Right> { void right() override; virtual void own(); };
		struct Last : Combined { void right() override; };
		"#,
	);
	for record in ["Combined", "Last"] {
		assert_eq!(
			slot_names(&index, record, CppAbi::Msvc),
			["left()", "own()"]
		);
		assert_eq!(
			slot_names(&index, record, CppAbi::Itanium),
			["left()", "right()", "own()"]
		);
	}
}

#[test]
fn specializes_dependent_primary_bases_and_reuses_concrete_virtual_signatures() {
	let records = records(
		r#"
	struct Player { virtual ~Player(); virtual void criteria(int); virtual int response(); virtual bool can_speak(); };
	struct Sink { virtual void sink(); };
	template<class Base> struct Host : Base, Sink {
		virtual void speak(float);
		virtual void criteria(int);
		virtual int response();
		bool can_speak();
	};
	struct Multiplayer : Host<Player> { void criteria(int) override; bool can_speak() override; virtual void extra(); };
	struct Attributes { virtual void attribute(); };
	struct TfPlayer : Multiplayer, Attributes { void attribute() override; virtual void give(); };
	"#,
	);
	assert_eq!(
		slot_names(&records, "TfPlayer", CppAbi::Msvc),
		[
			"~scalar",
			"criteria(int)",
			"response()",
			"can_speak()",
			"speak(float)",
			"extra()",
			"give()"
		]
	);
	assert_eq!(
		slot_names(&records, "TfPlayer", CppAbi::Itanium),
		[
			"~complete",
			"~deleting",
			"criteria(int)",
			"response()",
			"can_speak()",
			"speak(float)",
			"extra()",
			"attribute()",
			"give()"
		]
	);
	let layout = records.vtable_layout("TfPlayer", CppAbi::Itanium).unwrap();
	assert_eq!(
		layout.slots[2].method.qualified_name,
		"Multiplayer::criteria"
	);
	assert!(
		layout.slots[3]
			.method
			.qualified_name
			.contains("Host<Player>::response")
	);
}

#[test]
fn template_override_matching_preserves_cv_and_ref_qualifiers() {
	let index = records(
		r#"
		struct Base {
			virtual void same(int);
			virtual void qualified() volatile;
			virtual void reference() &;
		};
		template <class Parent> struct Host : Parent {
			void same(const int);
			void qualified();
			void reference() &&;
		};
		struct Concrete : Host<Base> {};
		"#,
	);
	for abi in [CppAbi::Msvc, CppAbi::Itanium] {
		let layout = index.vtable_layout("Concrete", abi).unwrap();
		assert_eq!(layout.slots.len(), 3);
		assert_eq!(layout.slots[0].method.qualified_name, "Host<Base>::same");
		assert_eq!(layout.slots[1].method.qualified_name, "Base::qualified");
		assert_eq!(layout.slots[2].method.qualified_name, "Base::reference");
	}
}
