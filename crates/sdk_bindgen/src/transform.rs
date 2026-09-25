use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Fields, File, GenericArgument, Item, ItemStruct, PathArguments, Type, parse_quote};

const PROBE_PREFIX: &str = "__crys_vtable_";
const VTABLE_SUFFIX: &str = "__bindgen_vtable";

/// Replaces bindgen's incomplete C++ vtables with the typed probe structs
/// emitted by the libclang metadata pass.
///
/// Bindgen represents C/C++ function pointers as `Option<unsafe extern fn>`.
/// A C++ vtable entry cannot be null, so probe aliases are resolved and the
/// `Option` wrapper is deliberately removed while the fields are installed.
pub(crate) fn install_vtable_probes(
	syntax: &mut File,
	probe_targets: &BTreeMap<String, String>,
	flattened_interfaces: &[&str],
) -> Result<BTreeMap<String, Vec<String>>, VtableTransformError> {
	let aliases = syntax
		.items
		.iter()
		.filter_map(|item| match item {
			Item::Type(alias) if alias.ident.to_string().starts_with(PROBE_PREFIX) => {
				Some((alias.ident.to_string(), (*alias.ty).clone()))
			}
			_ => None,
		})
		.collect::<BTreeMap<_, _>>();

	let mut replacements = BTreeMap::<String, (String, ItemStruct, Vec<String>)>::new();
	let mut target_owners = BTreeMap::<String, String>::new();

	for item in &syntax.items {
		let Item::Struct(probe) = item else { continue };
		let probe_name = probe.ident.to_string();
		let Some(probe_key) = probe_name.strip_prefix(PROBE_PREFIX) else {
			continue;
		};

		let Some(class_name) = probe_targets.get(probe_key) else {
			return Err(VtableTransformError::UnmappedProbe { probe: probe_name });
		};

		if let Some(existing) = target_owners.insert(class_name.clone(), probe_key.to_owned()) {
			return Err(VtableTransformError::DuplicateProbeTarget {
				target: class_name.clone(),
				first_probe: existing,
				second_probe: probe_key.to_owned(),
			});
		}

		let Fields::Named(probe_fields) = &probe.fields else {
			return Err(VtableTransformError::UnnamedProbe { probe: probe_name });
		};

		let mut fields = probe_fields.clone();
		let mut field_names = Vec::with_capacity(fields.named.len());

		for field in &mut fields.named {
			field.ty = resolve_function_pointer(&field.ty, &aliases, &mut BTreeSet::new())?;

			if !matches!(field.ty, Type::FnPtr(_)) {
				return Err(VtableTransformError::InvalidProbeField {
					field: field
						.ident
						.as_ref()
						.map(ToString::to_string)
						.unwrap_or_default(),
				});
			}

			let Some(identifier) = field.ident.as_ref() else {
				return Err(VtableTransformError::InvalidProbeField {
					field: String::new(),
				});
			};

			let identifier = identifier.to_string();
			let prefix = format!("{probe_key}_");

			let Some(suffix) = identifier.strip_prefix(&prefix) else {
				return Err(VtableTransformError::InvalidProbeField { field: identifier });
			};

			let renamed = format_ident!("{class_name}_{suffix}");

			field_names.push(renamed.to_string());

			field.ident = Some(renamed);
		}

		let vtable_ident = format_ident!("{class_name}{VTABLE_SUFFIX}");

		let replacement: ItemStruct = parse_quote! {
			#[repr(C)]
			pub struct #vtable_ident #fields
		};

		replacements.insert(
			probe_key.to_owned(),
			(class_name.clone(), replacement, field_names),
		);
	}

	let installed = replacements
		.iter()
		.map(|(probe, (_, _, fields))| (probe.clone(), fields.clone()))
		.collect::<BTreeMap<_, _>>();

	for (_, (class_name, replacement, _)) in replacements {
		let target_name = replacement.ident.to_string();

		if let Some(item) = syntax
			.items
			.iter_mut()
			.find(|item| matches!(item, Item::Struct(item) if item.ident == target_name))
		{
			*item = Item::Struct(replacement);
		} else {
			syntax.items.push(Item::Struct(replacement));
		}

		if flattened_interfaces.contains(&class_name.as_str()) {
			point_interface_at_vtable(syntax, &class_name)?;
		}
	}

	// The probes and their layout tests are implementation details. Their
	// resolved fields now live in the canonical `__bindgen_vtable` structs.
	syntax.items.retain(|item| match item {
		Item::Struct(item) => !item.ident.to_string().starts_with(PROBE_PREFIX),
		Item::Type(item) => !item.ident.to_string().starts_with(PROBE_PREFIX),
		Item::Const(item) => !quote!(#item).to_string().contains(PROBE_PREFIX),
		_ => true,
	});

	Ok(installed)
}

/// Validates that every generated vtable is callable and that probe-backed
/// tables exactly match their metadata-derived ABI slot order.
pub(crate) fn validate_vtables_against(
	syntax: &File,
	expected_fields: &BTreeMap<String, Vec<String>>,
	intentionally_opaque: &BTreeSet<String>,
) -> Result<(), VtableTransformError> {
	let mut found = BTreeSet::new();

	for item in &syntax.items {
		let Item::Struct(vtable) = item else { continue };
		let name = vtable.ident.to_string();
		let Some(class_name) = name.strip_suffix(VTABLE_SUFFIX) else {
			continue;
		};

		if intentionally_opaque.contains(class_name) {
			continue;
		}

		let Fields::Named(fields) = &vtable.fields else {
			return Err(VtableTransformError::IncompleteVtable { vtable: name });
		};

		if fields.named.is_empty() {
			return Err(VtableTransformError::IncompleteVtable { vtable: name });
		}

		if fields
			.named
			.iter()
			.any(|field| !matches!(field.ty, Type::FnPtr(_)))
		{
			return Err(VtableTransformError::IncompleteVtable { vtable: name });
		}

		if let Some(expected) = expected_fields.get(class_name) {
			let actual = fields
				.named
				.iter()
				.map(|field| {
					field
						.ident
						.as_ref()
						.map(ToString::to_string)
						.unwrap_or_default()
				})
				.collect::<Vec<_>>();

			if &actual != expected {
				return Err(VtableTransformError::MismatchedVtableFields {
					vtable: name,
					expected: expected.clone(),
					actual,
				});
			}

			found.insert(class_name.to_owned());
		}
	}

	if let Some(missing) = expected_fields.keys().find(|name| !found.contains(*name)) {
		return Err(VtableTransformError::MissingVtable {
			vtable: format!("{missing}{VTABLE_SUFFIX}"),
		});
	}

	Ok(())
}

/// Documents each placeholder vtable left in place for a record whose table
/// cannot be modeled, so the gap is visible in the generated API.
pub(crate) fn document_opaque_vtables(syntax: &mut File, reasons: &BTreeMap<String, String>) {
	for item in &mut syntax.items {
		let Item::Struct(vtable) = item else { continue };
		let name = vtable.ident.to_string();

		let Some(reason) = name
			.strip_suffix(VTABLE_SUFFIX)
			.and_then(|class_name| reasons.get(class_name))
		else {
			continue;
		};

		let summary = " Opaque placeholder: this vtable's slots are not modeled.".to_owned();
		let detail = format!(" Reason: {reason}.");

		vtable.attrs.insert(0, parse_quote!(#[doc = #detail]));
		vtable.attrs.insert(0, parse_quote!(#[doc = ""]));
		vtable.attrs.insert(0, parse_quote!(#[doc = #summary]));
	}
}

/// Documents each record made opaque because bindgen cannot express its
/// C++ layout, keyed by generated name.
pub(crate) fn document_opaque_records(syntax: &mut File, reasons: &BTreeMap<String, String>) {
	for item in &mut syntax.items {
		let (ident, attributes) = match item {
			Item::Struct(item) => (item.ident.to_string(), &mut item.attrs),
			Item::Union(item) => (item.ident.to_string(), &mut item.attrs),
			_ => continue,
		};

		let Some(reason) = reasons.get(&ident) else {
			continue;
		};
		let summary = " Opaque: bindgen cannot express this record's C++ layout, so only its size and alignment are kept.".to_owned();
		let detail = format!(" Generated as fields, {reason}.");

		attributes.insert(0, parse_quote!(#[doc = #detail]));
		attributes.insert(0, parse_quote!(#[doc = ""]));
		attributes.insert(0, parse_quote!(#[doc = #summary]));
	}
}

fn point_interface_at_vtable(
	syntax: &mut File,
	class_name: &str,
) -> Result<(), VtableTransformError> {
	let class_ident = format_ident!("{class_name}");
	let vtable_ident = format_ident!("{class_name}{VTABLE_SUFFIX}");
	let Some(class) = syntax.items.iter_mut().find_map(|item| match item {
		Item::Struct(item) if item.ident == class_ident => Some(item),
		_ => None,
	}) else {
		return Err(VtableTransformError::MissingInterface {
			interface: class_name.to_owned(),
		});
	};

	let Fields::Named(fields) = &mut class.fields else {
		return Err(VtableTransformError::UnexpectedInterfaceLayout {
			interface: class_name.to_owned(),
		});
	};

	if fields.named.len() != 1 {
		return Err(VtableTransformError::UnexpectedInterfaceLayout {
			interface: class_name.to_owned(),
		});
	}

	let field = fields.named.first_mut().expect("the length was checked");
	let is_vtable = field.ident.as_ref().is_some_and(|ident| ident == "vtable_");
	let is_primary_base = field.ident.as_ref().is_some_and(|ident| ident == "_base");

	if !is_vtable && !is_primary_base {
		return Err(VtableTransformError::UnexpectedInterfaceLayout {
			interface: class_name.to_owned(),
		});
	}

	*field = parse_quote! { pub vtable_: *const #vtable_ident };

	Ok(())
}

fn resolve_function_pointer(
	type_: &Type,
	aliases: &BTreeMap<String, Type>,
	visiting: &mut BTreeSet<String>,
) -> Result<Type, VtableTransformError> {
	let Type::Path(path) = type_ else {
		return Ok(type_.clone());
	};

	if path.qself.is_some() {
		return Ok(type_.clone());
	}

	let segment = path
		.path
		.segments
		.last()
		.expect("a type path has a segment");
	let name = segment.ident.to_string();

	if name == "Option" {
		let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
			return Ok(type_.clone());
		};

		let Some(GenericArgument::Type(inner)) = arguments.args.first() else {
			return Ok(type_.clone());
		};

		return resolve_function_pointer(inner, aliases, visiting);
	}

	let Some(alias) = aliases.get(&name) else {
		return Ok(type_.clone());
	};

	if !visiting.insert(name.clone()) {
		return Err(VtableTransformError::RecursiveProbeAlias { alias: name });
	}

	let resolved = resolve_function_pointer(alias, aliases, visiting);

	visiting.remove(&name);

	resolved
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum VtableTransformError {
	#[error(
		"more than one probe targets generated Rust class {target:?}: {first_probe:?} and {second_probe:?}"
	)]
	DuplicateProbeTarget {
		target: String,
		first_probe: String,
		second_probe: String,
	},

	#[error(
		"generated vtable {vtable:?} is empty or opaque without being explicitly marked opaque"
	)]
	IncompleteVtable { vtable: String },

	#[error("could not find generated interface {interface:?}")]
	MissingInterface { interface: String },

	#[error("could not find generated vtable {vtable:?}")]
	MissingVtable { vtable: String },

	#[error(
		"generated vtable {vtable:?} fields do not match metadata: expected {expected:?}, found {actual:?}"
	)]
	MismatchedVtableFields {
		vtable: String,
		expected: Vec<String>,
		actual: Vec<String>,
	},

	#[error("vtable probe field {field:?} is not a function pointer")]
	InvalidProbeField { field: String },

	#[error("vtable probe alias {alias:?} is recursive")]
	RecursiveProbeAlias { alias: String },

	#[error("interface {interface:?} is not a one-pointer C++ interface")]
	UnexpectedInterfaceLayout { interface: String },

	#[error("vtable probe {probe:?} does not have named fields")]
	UnnamedProbe { probe: String },

	#[error("vtable probe {probe:?} has no C++ to Rust record mapping")]
	UnmappedProbe { probe: String },
}

#[cfg(test)]
mod tests {
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

		let targets =
			BTreeMap::from([("fixture__Derived".to_owned(), "fixture_Derived".to_owned())]);
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
	fn documents_intentionally_opaque_vtables() {
		let mut syntax: File = parse_quote! {
			pub struct Multiple__bindgen_vtable(::std::os::raw::c_void);
			pub struct Other__bindgen_vtable(::std::os::raw::c_void);
		};

		let reasons = BTreeMap::from([(
			"Multiple".to_owned(),
			"`Multiple` has more than one polymorphic base class".to_owned(),
		)]);
		validate_vtables_against(
			&syntax,
			&BTreeMap::new(),
			&BTreeSet::from(["Multiple".to_owned(), "Other".to_owned()]),
		)
		.unwrap();
		document_opaque_vtables(&mut syntax, &reasons);

		let rendered = quote!(#syntax).to_string();
		assert!(
			rendered.contains("more than one polymorphic base class"),
			"{rendered}"
		);
		assert_eq!(rendered.matches("# [doc").count(), 3, "{rendered}");
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
}
