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
fn another_layouts_elements_find_nothing() {
	// Element 0's object is the entity's own, which the map does not declare.
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

	for (element, &offset) in (1..).zip(&found.objects) {
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
