//! Tests of entities' classes, names, parents' names, read from the members their datamaps declare.

use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use std::ptr::null;

#[test]
fn classes_are_those_of_the_entitys_datamaps() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![
		(c"CTFPlayer", vec![]),
		(c"CBaseAnimating", vec![]),
	]));

	let entity = mock.entity();

	assert!(entity.is_a(c"CTFPlayer"));
	assert!(entity.is_a(c"CBaseAnimating"));
	assert!(entity.is_a(c"CBaseEntity"));
	assert!(!entity.is_a(c"CObjectSentrygun"));
}

#[test]
fn names_are_read_from_their_pooled_strings() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![]));
	assert_eq!(mock.entity().name(), None);
	assert_eq!(mock.entity().parent_name(), Ok(None));

	mock.set_name(sys::string_t {
		pszValue: c"door".as_ptr(),
	});
	mock.state().parent_name = sys::string_t {
		pszValue: c"train".as_ptr(),
	};
	assert_eq!(mock.entity().name(), Some(c"door"));
	assert_eq!(mock.entity().parent_name(), Ok(Some(c"train")));

	// Empty names are no names.
	mock.set_name(sys::string_t {
		pszValue: c"".as_ptr(),
	});
	mock.state().parent_name = sys::string_t { pszValue: null() };
	assert_eq!(mock.entity().name(), None);
	assert_eq!(mock.entity().parent_name(), Ok(None));
}
