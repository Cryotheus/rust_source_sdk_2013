//! Tests of dissolving entities through a dissolver spawned for each.

use super::*;

use crate::test_support::entities::{
	MockEntity, activations, input, set_accepts, set_datamap, state_maps, take_inputs,
};

use crate::test_support::server_tools::MockTools;
use std::ffi::CStr;
use std::ptr::{NonNull, null_mut};

struct Mocks {
	tools: MockTools,
	target: MockEntity,
	dissolver: MockEntity,
}

impl Mocks {
	/// A mock entity, for as long as the mocks live.
	fn target(&mut self) -> Entity<'static> {
		// SAFETY: Mock entities are leaked, and their vtables answer what
		// dissolving calls of a `CBaseEntity`.
		unsafe { Entity::from_raw(NonNull::new(self.target.as_ptr()).unwrap()) }
	}
}

#[test]
fn dissolve_types_have_their_game_values() {
	assert_eq!(DissolveType::default(), DissolveType::Normal);
	assert_eq!(
		[
			DissolveType::Normal,
			DissolveType::Electrical,
			DissolveType::ElectricalLight,
			DissolveType::Core,
		]
		.map(DissolveType::to_raw),
		[0, 1, 2, 3]
	);
}

#[test]
fn entities_are_dissolved_by_a_dissolver_removed_after_its_input() {
	let mut mocks = setup(true);
	let target = mocks.target();
	let dissolver = mocks.dissolver.as_ptr();

	mocks
		.tools
		.tools()
		.dissolve(target, DissolveType::Electrical, 250)
		.unwrap();

	assert_eq!(
		mocks.tools.take_classes(),
		[c"env_entity_dissolver".to_owned()]
	);
	assert_eq!(
		mocks.tools.take_keys(),
		[
			(dissolver, c"dissolvetype".to_owned(), c"1".to_owned()),
			(dissolver, c"magnitude".to_owned(), c"250".to_owned()),
		]
	);
	assert_eq!(mocks.tools.take_spawned(), [dissolver]);
	assert!(activations().is_empty(), "the dissolver is not activated");

	let inputs = take_inputs();

	assert_eq!(inputs.len(), 1);
	assert_eq!(inputs[0].target, dissolver);
	assert_eq!(inputs[0].name.as_c_str(), c"Dissolve");
	assert_eq!(
		(inputs[0].activator, inputs[0].caller),
		(target.as_ptr(), target.as_ptr())
	);
	// SAFETY: The value was pooled, and stays allocated for the rest of the
	// thread.
	assert_eq!(unsafe { CStr::from_ptr(inputs[0].string) }, c"!activator");
	assert_eq!(mocks.tools.take_removed(), [dissolver]);
}

#[test]
fn refused_inputs_still_remove_the_dissolver() {
	let mut mocks = setup(true);
	let target = mocks.target();

	set_accepts(false);
	assert_eq!(
		mocks
			.tools
			.tools()
			.dissolve(target, DissolveType::Normal, 0),
		Err(DissolveError::Input(InputError::Rejected))
	);
	assert_eq!(mocks.tools.take_removed(), [mocks.dissolver.as_ptr()]);
}

/// A target, and a dissolver the tools create, whose classes declare the
/// `Dissolve` input, and derive from `CBaseAnimating` if `animating`.
fn setup(animating: bool) -> Mocks {
	let mut world = MockEntity::new(0);
	let mut mocks = Mocks {
		tools: MockTools::new(world.as_ptr()),
		target: MockEntity::new(7 | 3 << 16),
		dissolver: MockEntity::new(9 | 1 << 16),
	};

	let mut classes = vec![(
		c"CTestEntity",
		vec![input(c"Dissolve", sys::_fieldtypes_FIELD_STRING)],
	)];

	if animating {
		classes.push((c"CBaseAnimating", vec![]));
	}

	set_datamap(state_maps(classes));
	mocks.tools.set_created(mocks.dissolver.as_ptr());
	set_accepts(true);
	take_inputs();
	mocks
}

#[test]
fn targets_that_cannot_dissolve_are_refused() {
	let mut mocks = setup(false);
	let target = mocks.target();
	let tools = mocks.tools.tools();

	assert_eq!(
		tools.dissolve(target, DissolveType::Core, 250),
		Err(DissolveError::NotAnimating)
	);

	let mut mocks = setup(true);
	let target = mocks.target();
	let tools = mocks.tools.tools();

	mocks.target.state().flags = EntityFlags::DISSOLVING.bits();
	assert_eq!(
		tools.dissolve(target, DissolveType::Core, 250),
		Err(DissolveError::AlreadyDissolving)
	);
	assert!(mocks.tools.take_classes().is_empty());

	mocks.target.state().flags = 0;
	mocks.tools.set_created(null_mut());
	assert_eq!(
		tools.dissolve(target, DissolveType::Core, 250),
		Err(DissolveError::Spawn(SpawnError::UnknownClass {
			class: c"env_entity_dissolver".to_owned()
		}))
	);
	assert!(take_inputs().is_empty());
}
