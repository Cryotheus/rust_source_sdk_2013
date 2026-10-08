//! Tests of finding entities by class, by model and near a point, and of
//! matching names as the game does.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use crate::test_support::server_tools::{MockTools, Search};
use std::ptr::null_mut;

#[test]
fn names_match_as_the_game_matches_them() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![(c"CTestEntity", vec![])]));

	let mut matches = |name: Option<&'static CStr>, query: &CStr| {
		mock.set_name(sys::string_t {
			pszValue: name.map_or(ptr::null(), CStr::as_ptr),
		});
		mock.entity().name_matches(query)
	};

	for (name, query, matched) in [
		(c"door_1", c"door_1", true),
		(c"Door_1", c"dOOR_1", true),
		(c"door_1", c"door", false),
		(c"door", c"door_1", false),
		(c"door_1", c"door*", true),
		(c"door_1", c"*", true),
		(c"door_1", c"", false),
		(c"", c"", true),
		(c"", c"*door", true),
		// Past the characters that match, a `*` matches the rest.
		(c"door_1", c"d*r", true),
		(c"door_1", c"door_1*", true),
		// The game compares characters promoted to `int`s, so those below `A`
		// also match the one 32 above them, and those below `a` the one 32
		// below them.
		(c"1", c"Q", true),
		(c"Q", c"1", true),
		(c"q", c"1", false),
		(c"[", c";", true),
	] {
		assert_eq!(
			matches(Some(name), query),
			matched,
			"{name:?} against {query:?}"
		);
	}

	assert!(matches(None, c""));
	assert!(matches(None, c"*door"));
	assert!(!matches(None, c"door"));
	assert!(!matches(None, c"!player"), "it is not a player");

	set_datamap(state_maps(vec![
		(c"CTFPlayer", vec![]),
		(c"CBasePlayer", vec![]),
	]));
	assert!(mock.entity().name_matches(c"!PLAYER"));
}

/// The addresses of the entities a search finds.
fn pointers<'s>(entities: impl Iterator<Item = Entity<'s>>) -> Vec<*mut sys::CBaseEntity> {
	entities.map(Entity::as_ptr).collect()
}

#[test]
fn searches_continue_after_each_entity_found() {
	let mut world = MockEntity::new(0);
	let mut first = MockEntity::new(1);
	let mut second = MockEntity::new(2);
	let mocks = MockTools::new(world.as_ptr());
	let tools = mocks.tools();
	let found = [first.as_ptr(), second.as_ptr()];

	assert!(pointers(tools.entities_by_class(c"prop_*")).is_empty());
	mocks.set_found(found.to_vec());
	assert_eq!(pointers(tools.entities_by_class(c"prop_*")), found);
	assert_eq!(pointers(tools.entities_by_model(c"*1")), found);
	assert_eq!(
		pointers(tools.entities_in_sphere(Vector::new(1.0, 2.0, 3.0), 64.0)),
		found
	);

	let class = || c"prop_*".to_owned();
	let model = || c"*1".to_owned();
	let center = [1.0, 2.0, 3.0];

	assert_eq!(
		mocks.take_searches(),
		[
			Search::Class(null_mut(), class()),
			Search::Class(null_mut(), class()),
			Search::Class(found[0], class()),
			Search::Class(found[1], class()),
			Search::Model(null_mut(), model()),
			Search::Model(found[0], model()),
			Search::Model(found[1], model()),
			Search::Sphere(null_mut(), center, 64.0),
			Search::Sphere(found[0], center, 64.0),
			Search::Sphere(found[1], center, 64.0),
		]
	);
}
