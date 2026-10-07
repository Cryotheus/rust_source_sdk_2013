//! Tests of creating entities: the key values a spawn gives, and what it does
//! when the class is unknown, a value is not finite, a key is refused, or the
//! entity removes itself.

use super::*;

use crate::test_support::entities::{
	MockEntity, activations, set_activation_removes, set_datamap, state_maps,
};

use crate::test_support::server_tools::MockTools;

#[test]
fn entities_are_created_given_their_keys_spawned_and_activated() {
	let mut created = MockEntity::new(7 | 3 << 16);
	let mocks = mock_tools(&mut created);
	let spawn = EntitySpawn::new(c"info_target").name(c"marker");

	// SAFETY: The mocks' constructor, `Spawn` and `Activate` free nothing.
	let entity = unsafe { spawn.spawn(mocks.tools()) }.unwrap();

	assert_eq!(entity.as_ptr(), created.as_ptr());
	assert_eq!(entity.name(), Some(c"marker"));
	assert_eq!(mocks.take_classes(), [c"info_target".to_owned()]);
	assert_eq!(
		mocks.take_keys(),
		[(
			created.as_ptr(),
			c"targetname".to_owned(),
			c"marker".to_owned()
		)]
	);
	assert_eq!(mocks.take_spawned(), [created.as_ptr()]);
	assert_eq!(activations(), [created.as_ptr()]);

	// SAFETY: As above.
	unsafe { spawn.activate(false).spawn(mocks.tools()) }.unwrap();
	assert_eq!(mocks.take_spawned(), [created.as_ptr()]);
	assert_eq!(activations(), [created.as_ptr()], "not activated again");
}

#[test]
fn entities_that_remove_themselves_are_reported() {
	let mut created = MockEntity::new(7 | 3 << 16);
	let mocks = mock_tools(&mut created);

	mocks.set_spawn_removes(true);

	// SAFETY: The mocks' constructor, `Spawn` and `Activate` free nothing.
	let spawned = unsafe { EntitySpawn::new(c"info_target").spawn(mocks.tools()) };

	assert_eq!(spawned, Err(SpawnError::RemovedItself));
	assert!(activations().is_empty(), "not activated once removed");

	let mut created = MockEntity::new(7 | 3 << 16);
	let mocks = mock_tools(&mut created);

	set_activation_removes(true);

	// SAFETY: As above.
	let activated = unsafe { EntitySpawn::new(c"info_target").spawn(mocks.tools()) };

	assert_eq!(activated, Err(SpawnError::RemovedItself));
	assert_eq!(activations(), [created.as_ptr()]);
}

#[test]
fn keys_are_kept_in_order_with_vectors_formatted() {
	let spawn = EntitySpawn::new(c"prop_dynamic")
		.name(c"locker")
		.model(c"models/props_gameplay/resupply_locker.mdl")
		.origin(Vector::new(1.0, -2.5, 64.0))
		.angles(QAngle {
			pitch: 0.0,
			yaw: 90.0,
			roll: 0.125,
		})
		.key(c"solid", c"6")
		.key(c"solid", c"0");

	assert_eq!(spawn.class(), c"prop_dynamic");
	assert_eq!(
		spawn.keys().collect::<Vec<_>>(),
		[
			(c"targetname", c"locker"),
			(c"model", c"models/props_gameplay/resupply_locker.mdl"),
			(c"origin", c"1 -2.5 64"),
			(c"angles", c"0 90 0.125"),
			(c"solid", c"6"),
			(c"solid", c"0"),
		]
	);
}

/// Mock tools that create `created`, with a world.
fn mock_tools(created: &mut MockEntity) -> MockTools {
	let mut world = MockEntity::new(0);
	let tools = MockTools::new(world.as_ptr());

	set_datamap(state_maps(vec![(c"CTestEntity", vec![])]));
	tools.set_created(created.as_ptr());
	tools
}

#[test]
fn refused_keys_remove_the_entity() {
	let mut created = MockEntity::new(7 | 3 << 16);
	let mocks = mock_tools(&mut created);
	let spawn = EntitySpawn::new(c"prop_dynamic")
		.key(c"solid", c"6")
		.key(c"skin", c"1");

	mocks.set_refused(c"solid");

	// SAFETY: The mocks' constructor, `Spawn` and `Activate` free nothing.
	let refused = unsafe { spawn.spawn(mocks.tools()) };

	assert_eq!(
		refused,
		Err(SpawnError::KeyRejected {
			key: c"solid".to_owned()
		})
	);
	assert_eq!(mocks.take_keys().len(), 1, "no later key is set");
	assert_eq!(mocks.take_removed(), [created.as_ptr()]);
	assert!(mocks.take_spawned().is_empty());

	// The key values `ServerTools::set_key_value` refuses never reach the game.
	let mut created = MockEntity::new(7 | 3 << 16);
	let mocks = mock_tools(&mut created);

	// SAFETY: As above.
	let refused = unsafe {
		EntitySpawn::new(c"prop_dynamic")
			.key(c"max_health", c"0")
			.spawn(mocks.tools())
	};

	assert_eq!(
		refused,
		Err(SpawnError::KeyRejected {
			key: c"max_health".to_owned()
		})
	);
	assert!(mocks.take_keys().is_empty());
	assert_eq!(mocks.take_removed(), [created.as_ptr()]);
}

#[test]
fn unknown_classes_and_values_that_are_not_finite_create_nothing() {
	let mut created = MockEntity::new(7 | 3 << 16);
	let mocks = mock_tools(&mut created);

	// SAFETY: The mocks' constructor, `Spawn` and `Activate` free nothing.
	let not_finite = unsafe {
		EntitySpawn::new(c"info_target")
			.origin(Vector::new(0.0, f32::NAN, 0.0))
			.name(c"marker")
			.spawn(mocks.tools())
	};

	assert_eq!(
		not_finite,
		Err(SpawnError::NonFinite {
			key: c"origin".to_owned()
		})
	);
	assert!(mocks.take_classes().is_empty());

	// SAFETY: As above.
	let angles = unsafe {
		EntitySpawn::new(c"info_target")
			.angles(QAngle {
				pitch: f32::INFINITY,
				yaw: 0.0,
				roll: 0.0,
			})
			.spawn(mocks.tools())
	};

	assert_eq!(
		angles,
		Err(SpawnError::NonFinite {
			key: c"angles".to_owned()
		})
	);

	mocks.set_created(std::ptr::null_mut());

	// SAFETY: As above.
	let unknown = unsafe { EntitySpawn::new(c"no_such_class").spawn(mocks.tools()) };

	assert_eq!(
		unknown,
		Err(SpawnError::UnknownClass {
			class: c"no_such_class".to_owned()
		})
	);
	assert!(mocks.take_spawned().is_empty());
}
