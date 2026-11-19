//! Tests of how a `prop_ragdoll`'s physics objects are found and switched.

use super::*;
use crate::entities::movement::PhysicsObjectError;
use crate::test_support::entities::{MockEntity, MockPhysicsObject, set_datamap, state_maps};
use sdk_raw::test_support::entities::field;
use std::alloc::Layout;
use std::ffi::{CString, c_void};
use std::ptr::null_mut;

/// Where mock ragdolls store `m_ragdoll.listCount`, past the words every mock
/// entity uses.
const COUNT: usize = 1024;

/// Where mock ragdolls' element 0 starts.
const LIST: usize = 1040;

/// How far apart mock ragdolls' elements lie: an arbitrary size, a whole number
/// of pointers, which the finder must learn from the map.
const STRIDE: usize = 32;

#[test]
fn a_count_beyond_the_most_finds_every_element_once() {
	let (mut mock, objects) = full_ragdoll();

	set_count(mock.as_ptr(), 1000);

	for object in RagdollPhysicsObjects::new(mock.entity()).unwrap() {
		object.set_collisions_enabled(false).unwrap();
	}

	assert!(objects.iter().all(|object| object.switches() == 1));
}

#[test]
fn a_ragdoll_whose_map_lacks_a_member_switches_nothing() {
	for missing in [1, 12, 23] {
		let mut mock = mock_ragdoll(c"CRagdollProp", ragdoll_fields(Some(missing)));
		let objects: Vec<_> = (0..RAGDOLL_MAX_ELEMENTS)
			.map(|_| MockPhysicsObject::leaked())
			.collect();

		for (element, object) in objects.iter().enumerate() {
			set_object(mock.as_ptr(), element, object.as_ptr());
		}

		set_count(mock.as_ptr(), RAGDOLL_MAX_ELEMENTS);

		assert_eq!(
			RagdollPhysicsObjects::new(mock.entity()).err(),
			Some(RagdollObjectsError::UnexpectedFields)
		);

		assert!(objects.iter().all(|object| object.switches() == 0));
	}
}

#[test]
fn an_entity_that_is_not_a_ragdoll_has_no_ragdoll_objects() {
	let mut mock = mock_ragdoll(c"CDynamicProp", ragdoll_fields(None));
	let object = MockPhysicsObject::leaked();

	mock.state().physics_object = object.as_ptr();

	assert_eq!(
		RagdollPhysicsObjects::new(mock.entity()).err(),
		Some(RagdollObjectsError::NotRagdoll)
	);
}

#[test]
fn every_object_is_switched_once_whatever_the_entitys_own_points_at() {
	for own in [5, RAGDOLL_MAX_ELEMENTS as usize - 1] {
		let (mut mock, objects) = full_ragdoll();

		// As the game points the entity's own object at a part a bullet hit.
		mock.state().physics_object = objects[own].as_ptr();

		let found: Vec<_> = RagdollPhysicsObjects::new(mock.entity()).unwrap().collect();

		assert_eq!(found.len(), objects.len());

		for object in found {
			assert_eq!(object.set_collisions_enabled(false), Ok(()));
		}

		let switched: Vec<_> = objects.iter().map(|object| object.switches()).collect();

		assert!(
			switched.iter().all(|&switches| switches == 1),
			"own {own}: {switched:?}"
		);
	}

	// An entity's own object that is none of the list's is never switched.
	let (mut mock, objects) = full_ragdoll();
	let stray = MockPhysicsObject::leaked();

	mock.state().physics_object = stray.as_ptr();

	for object in RagdollPhysicsObjects::new(mock.entity()).unwrap() {
		object.set_collisions_enabled(false).unwrap();
	}

	assert_eq!(stray.switches(), 0);
	assert!(objects.iter().all(|object| object.switches() == 1));
}

#[test]
fn every_object_of_a_ragdoll_is_found_and_switched() {
	let (mut mock, objects) = full_ragdoll();
	let found = RagdollPhysicsObjects::new(mock.entity()).unwrap();

	for object in found {
		assert_eq!(object.set_collisions_enabled(false), Ok(()));
		assert_eq!(object.collisions_enabled(), Ok(false));
	}

	for object in &objects {
		assert_eq!((object.enabled(), object.switches()), (false, 1));
	}

	// The objects come in the order of their elements, from element 0's.
	let mut found = 0;

	for (element, object) in RagdollPhysicsObjects::new(mock.entity())
		.unwrap()
		.enumerate()
	{
		assert_eq!(object.set_collisions_enabled(true), Ok(()));

		let (switched, left) = objects.split_at(element + 1);

		assert!(switched.iter().all(|object| object.enabled()));
		assert!(left.iter().all(|object| !object.enabled()));
		found += 1;
	}

	assert_eq!(found, objects.len());
}

/// A mock ragdoll with an object for each of its elements, all in use.
fn full_ragdoll() -> (MockEntity, Vec<&'static MockPhysicsObject>) {
	let mut mock = mock_ragdoll(c"CRagdollProp", ragdoll_fields(None));
	let objects: Vec<_> = (0..RAGDOLL_MAX_ELEMENTS)
		.map(|_| MockPhysicsObject::leaked())
		.collect();

	for (element, object) in objects.iter().enumerate() {
		set_object(mock.as_ptr(), element, object.as_ptr());
	}

	// As the game leaves the entity's own as it makes the ragdoll.
	mock.state().physics_object = objects[0].as_ptr();
	set_count(mock.as_ptr(), RAGDOLL_MAX_ELEMENTS);
	(mock, objects)
}

/// A mock entity large enough for a ragdoll's members, whose class's map is
/// `class`, declaring `fields`, deriving from `CBaseEntity`'s.
fn mock_ragdoll(class: &'static CStr, fields: Vec<sys::typedescription_t>) -> MockEntity {
	let mock = MockEntity::with_layout(7, Layout::new::<[usize; 256]>());

	set_datamap(state_maps(vec![(class, fields)]));
	mock
}

/// Where a mock ragdoll stores the object of `element`, 8 bytes into it.
fn object_offset(element: usize) -> usize {
	LIST + element * STRIDE + 8
}

#[test]
fn objects_of_elements_no_longer_in_use_are_not_called() {
	let (mut mock, objects) = full_ragdoll();
	let ragdoll = mock.as_ptr();
	let found: Vec<_> = RagdollPhysicsObjects::new(mock.entity()).unwrap().collect();

	// As the game destroys the ragdoll's objects.
	set_count(ragdoll, 2);

	let results: Vec<_> = found
		.iter()
		.map(|object| object.set_collisions_enabled(false))
		.collect();

	assert_eq!(results[..2], [Ok(()), Ok(())]);
	assert!(
		results[2..]
			.iter()
			.all(|&result| result == Err(PhysicsObjectError::Gone))
	);
	assert!(objects[2..].iter().all(|object| object.switches() == 0));

	// The count is read again as the objects are gone over.
	let mut remaining = RagdollPhysicsObjects::new(mock.entity()).unwrap();

	assert!(remaining.next().is_some());
	set_count(ragdoll, 1);
	assert!(remaining.next().is_none());
}

#[test]
fn only_the_elements_in_use_with_objects_are_found() {
	let (mut mock, objects) = full_ragdoll();
	let ragdoll = mock.as_ptr();

	set_object(ragdoll, 0, null_mut());
	set_object(ragdoll, 4, null_mut());
	set_count(ragdoll, 6);

	for object in RagdollPhysicsObjects::new(mock.entity()).unwrap() {
		object.set_collisions_enabled(false).unwrap();
	}

	let switched: Vec<_> = objects.iter().map(|object| object.switches()).collect();

	assert_eq!(switched[..6], [0, 1, 1, 1, 0, 1]);
	assert!(switched[6..].iter().all(|&switches| switches == 0));

	// A negative count has none in use.
	set_count(ragdoll, -3);
	assert_eq!(
		RagdollPhysicsObjects::new(mock.entity()).unwrap().count(),
		0
	);
}

/// The members of a ragdoll's class's map, as the game declares them, but
/// for the object of `missing`.
fn ragdoll_fields(missing: Option<usize>) -> Vec<sys::typedescription_t> {
	let mut count = field(
		c"m_ragdoll.listCount",
		sys::_fieldtypes_FIELD_INTEGER,
		COUNT,
	);

	count.fieldSize = 1;
	count.fieldSizeInBytes = size_of::<c_int>() as c_int;

	let mut fields = vec![count];

	for element in (1..RAGDOLL_MAX_ELEMENTS as usize).filter(|&element| Some(element) != missing) {
		let name = CString::new(format!("m_ragdoll.list[{element}].pObject")).unwrap();
		let mut object = field(
			Box::leak(name.into_boxed_c_str()),
			sys::_fieldtypes_FIELD_CUSTOM,
			object_offset(element),
		);

		object.fieldSize = 1;
		fields.push(object);
	}

	fields
}

/// Sets how many of the elements of `ragdoll`, a mock ragdoll, are in use,
/// as the game would while the wrappers hold it.
fn set_count(ragdoll: *mut sys::CBaseEntity, count: c_int) {
	// SAFETY: The mock's storage holds 256 words, past the aligned count.
	unsafe { ragdoll.byte_add(COUNT).cast::<c_int>().write(count) };
}

/// Gives `ragdoll`, a mock ragdoll, an object for `element`, or none for
/// null, as the game would while the wrappers hold it.
fn set_object(ragdoll: *mut sys::CBaseEntity, element: usize, object: *mut c_void) {
	// SAFETY: The mock's storage holds 256 words, past the aligned member.
	unsafe {
		ragdoll
			.byte_add(object_offset(element))
			.cast::<*mut c_void>()
			.write(object)
	};
}
