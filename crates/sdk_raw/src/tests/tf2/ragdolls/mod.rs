//! Tests of the search of a `prop_ragdoll`'s map for its physics objects.

use super::*;
use crate::test_support::entities::{data_map, field};
use std::ffi::CString;
use std::ptr::NonNull;

/// Where the fixture's `m_ragdoll.listCount` lies.
const COUNT: usize = 1024;

/// Where the fixture's element 0 starts.
const LIST: usize = 1040;

/// How far apart the fixture's elements lie: an arbitrary size, a whole number
/// of pointers, which the finder must learn from the map.
const STRIDE: usize = 32;

#[test]
fn a_member_declared_where_element_0s_object_lies_is_found() {
	let sized = |name: &'static CStr, offset: usize, size: usize| {
		let mut field = field(name, sys::_fieldtypes_FIELD_INTEGER, offset);

		field.fieldSize = 1;
		field.fieldSizeInBytes = size as c_int;
		field
	};

	// Members just before and just after it leave it clear.
	let own = [
		ragdoll_fields(),
		vec![
			sized(c"before", object(0) - 8, 8),
			sized(c"after", object(0) + 8, 4),
		],
	]
	.concat();

	assert!(undeclared(own, vec![sized(c"m_iHealth", 64, 4)]));

	// One of the ragdoll's own members on it, whether its size is known or not.
	let own = [ragdoll_fields(), vec![sized(c"on", object(0) + 4, 4)]].concat();

	assert!(!undeclared(own, Vec::new()));

	let own = [
		ragdoll_fields(),
		vec![field(c"on", sys::_fieldtypes_FIELD_CUSTOM, object(0))],
	]
	.concat();

	assert!(!undeclared(own, Vec::new()));

	// A base's member reaching into it from before.
	assert!(!undeclared(
		ragdoll_fields(),
		vec![sized(c"m_pPhysicsObject", object(0) - 4, 8)]
	));
}

#[test]
fn a_missing_or_repeated_member_finds_nothing() {
	for element in 1..RAGDOLL_MAX_ELEMENTS as usize {
		let mut fields = ragdoll_fields();

		fields.retain(|field| !named(field, &object_name(element)));
		assert_eq!(find(fields), None, "without element {element}");
	}

	let mut fields = ragdoll_fields();

	fields.retain(|field| !named(field, c"m_ragdoll.listCount"));
	assert_eq!(find(fields), None);

	let mut fields = ragdoll_fields();

	fields.push(count_field(COUNT - 8));
	assert_eq!(find(fields), None);

	let mut fields = ragdoll_fields();

	fields.push(object_field(5, object(5)));
	assert_eq!(find(fields), None);
}

#[test]
fn an_element_0_outside_the_entity_or_on_the_count_finds_nothing() {
	// Element 1 so near the entity's start that element 0 would lie before it.
	let mut fields = vec![count_field(COUNT)];

	for element in 1..RAGDOLL_MAX_ELEMENTS as usize {
		fields.push(object_field(element, 16 + (element - 1) * STRIDE));
	}

	assert_eq!(find(fields), None);

	// Element 0's object on the entity's first pointer, its table of virtual
	// functions, and one pointer past it.
	for (start, found) in [(0, false), (size_of::<*const ()>(), true)] {
		let mut fields = vec![count_field(COUNT)];

		for element in 1..RAGDOLL_MAX_ELEMENTS as usize {
			fields.push(object_field(element, start + element * STRIDE));
		}

		assert_eq!(find(fields).is_some(), found, "element 0 at {start}");
	}

	// The count where element 0's object lies.
	let mut fields = ragdoll_fields();

	fields[1] = count_field(object(0) + 4);
	assert_eq!(find(fields), None);
}

#[test]
fn another_layouts_elements_find_nothing() {
	// This layout's map leaves element 0's object out.
	let mut fields = ragdoll_fields();

	fields.push(object_field(0, object(0)));
	assert_eq!(find(fields), None);

	// A game whose ragdolls have more elements.
	let mut fields = ragdoll_fields();

	fields.push(object_field(24, object(24)));
	assert_eq!(find(fields), None);
}

/// The member `m_ragdoll.listCount`, declared as the game does.
fn count_field(offset: usize) -> sys::typedescription_t {
	let mut count = field(
		c"m_ragdoll.listCount",
		sys::_fieldtypes_FIELD_INTEGER,
		offset,
	);

	count.fieldSize = 1;
	count.fieldSizeInBytes = size_of::<c_int>() as c_int;
	count
}

#[test]
fn every_object_is_found_in_the_ragdolls_map() {
	let mut fields = ragdoll_fields();

	// The game may declare them in any order.
	fields.reverse();

	let found = find(fields).unwrap();

	assert_eq!(found.count, COUNT);

	// Element 0's too, which the map leaves out, one element before element 1's.
	for (element, &offset) in found.objects.iter().enumerate() {
		assert_eq!(offset, object(element));
	}
}

/// Searches a map of `fields`.
fn find(fields: Vec<sys::typedescription_t>) -> Option<RagdollObjectFields> {
	let map = data_map(RAGDOLL_PROP_CLASS, fields, std::ptr::null_mut());

	// SAFETY: The map is leaked, with its fields and their names.
	find_ragdoll_object_fields(unsafe { DataMap::from_raw(NonNull::new(map).unwrap()) })
}

#[test]
fn members_of_the_wrong_kind_or_place_find_nothing() {
	let change = |element: usize, change: &dyn Fn(&mut sys::typedescription_t)| {
		let mut fields = ragdoll_fields();
		let name = object_name(element);

		change(fields.iter_mut().find(|field| named(field, &name)).unwrap());

		find(fields)
	};

	assert_eq!(
		change(3, &|field| field.fieldType = sys::_fieldtypes_FIELD_INTEGER),
		None
	);

	assert_eq!(change(3, &|field| field.fieldSize = 2), None);

	// Misaligned for a pointer.
	assert_eq!(change(3, &|field| field.fieldOffset[0] += 4), None);

	// Out of step with the other elements.
	assert_eq!(change(3, &|field| field.fieldOffset[0] += 8), None);

	assert_eq!(change(3, &|field| field.fieldOffset[0] = -8), None);

	// The count must be an aligned `int`, clear of the objects.
	let mut fields = ragdoll_fields();

	fields[1] = count_field(COUNT + 2);
	assert_eq!(find(fields), None);

	let mut fields = ragdoll_fields();

	fields[1].fieldSizeInBytes = 8;
	assert_eq!(find(fields), None);

	let mut fields = ragdoll_fields();

	fields[1] = count_field(object(4) + 4);
	assert_eq!(find(fields), None);
}

#[test]
fn members_past_the_limit_find_nothing() {
	let far = RAGDOLL_PROP_FIELD_OFFSET_LIMIT - 16 * STRIDE;
	let mut fields = vec![count_field(COUNT)];

	for element in 1..RAGDOLL_MAX_ELEMENTS as usize {
		fields.push(object_field(element, far + element * STRIDE));
	}

	assert_eq!(find(fields), None);

	// The same layout, short of the limit, is found.
	let near = RAGDOLL_PROP_FIELD_OFFSET_LIMIT - 24 * STRIDE;
	let mut fields = vec![count_field(COUNT)];

	for element in 1..RAGDOLL_MAX_ELEMENTS as usize {
		fields.push(object_field(element, near + element * STRIDE));
	}

	assert!(find(fields).is_some());
}

/// Whether `field` is named `name`.
fn named(field: &sys::typedescription_t, name: &CStr) -> bool {
	// SAFETY: The fixture's names are leaked strings.
	!field.fieldName.is_null() && unsafe { CStr::from_ptr(field.fieldName) } == name
}

/// Where the fixture's object of `element` lies, 8 bytes into it.
fn object(element: usize) -> usize {
	LIST + element * STRIDE + 8
}

/// The member `m_ragdoll.list[element].pObject`, declared as the game does.
fn object_field(element: usize, offset: usize) -> sys::typedescription_t {
	let mut object = field(
		Box::leak(object_name(element).into_boxed_c_str()),
		sys::_fieldtypes_FIELD_CUSTOM,
		offset,
	);

	object.fieldSize = 1;
	object
}

/// The name of element `element`'s object.
fn object_name(element: usize) -> CString {
	CString::new(format!("m_ragdoll.list[{element}].pObject")).unwrap()
}

#[test]
fn only_canonical_member_names_name_an_element() {
	assert_eq!(
		ragdoll_object_element(b"m_ragdoll.list[7].pObject"),
		Some(7)
	);
	assert_eq!(
		ragdoll_object_element(b"m_ragdoll.list[23].pObject"),
		Some(23)
	);
	assert_eq!(
		ragdoll_object_element(b"m_ragdoll.list[0].pObject"),
		Some(0)
	);
	assert_eq!(ragdoll_object_element(b"m_ragdoll.list[07].pObject"), None);
	assert_eq!(ragdoll_object_element(b"m_ragdoll.list[].pObject"), None);
	assert_eq!(ragdoll_object_element(b"m_ragdoll.list[+7].pObject"), None);
	assert_eq!(
		ragdoll_object_element(b"m_ragdoll.list[7].pConstraint"),
		None
	);
	assert_eq!(ragdoll_object_element(b"m_ragdoll.listCount"), None);

	assert_eq!(
		ragdoll_object_element(b"m_ragdoll.list[99999999999999999999999].pObject"),
		None
	);
}

/// The fields of a ragdoll's map, with others around them, as the game
/// declares them.
fn ragdoll_fields() -> Vec<sys::typedescription_t> {
	let mut fields = vec![field(c"m_flBlendWeight", sys::_fieldtypes_FIELD_FLOAT, 8)];

	fields.push(count_field(COUNT));

	for element in 1..RAGDOLL_MAX_ELEMENTS as usize {
		fields.push(object_field(element, object(element)));
	}

	fields
}

/// Whether [`ragdoll_element_0_undeclared`] finds element 0's object clear in a
/// ragdoll's map of `own`, whose base's map declares `base`.
fn undeclared(own: Vec<sys::typedescription_t>, base: Vec<sys::typedescription_t>) -> bool {
	let base = data_map(c"CBaseEntity", base, std::ptr::null_mut());
	let own = data_map(RAGDOLL_PROP_CLASS, own, base);

	// SAFETY: The maps are leaked, with their fields and their names.
	let (map, maps) = unsafe {
		(
			DataMap::from_raw(NonNull::new(own).unwrap()),
			DataMaps::new(own),
		)
	};

	ragdoll_element_0_undeclared(maps, &find_ragdoll_object_fields(map).unwrap())
}
