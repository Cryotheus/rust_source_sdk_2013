//! Tests of entities' native properties, read through the game's vtables and
//! data description maps, and of teleporting them.

use super::*;
use crate::test_support::entities::{MockEntity, set_collision_group, teleports};

#[test]
fn collision_groups_are_read_from_the_collideable() {
	// `TFCOLLISION_GROUP_ROCKETS`, a group of TF2's own.
	let rockets = sdk_raw::entities::LAST_SHARED_COLLISION_GROUP + 4;
	let mut mock = MockEntity::new(5);

	assert_eq!(mock.entity().collision_group(), Some(COLLISION_GROUP_NONE));

	// Shared groups convert, and games' own are kept as they are.
	set_collision_group(COLLISION_GROUP_DEBRIS);
	assert_eq!(
		mock.entity()
			.collision_group()
			.and_then(CollisionGroup::from_raw),
		Some(CollisionGroup::Debris)
	);

	set_collision_group(rockets);
	assert_eq!(mock.entity().collision_group(), Some(rockets));
	assert_eq!(CollisionGroup::from_raw(rockets), None);
}

#[test]
fn entities_read_native_properties_and_teleport() {
	let mut mock = MockEntity::new(5 | 7 << 16);
	let entity = mock.entity();

	assert_eq!(entity.class_name(), c"tf_player");
	assert_eq!(entity.position(), Some(Vector::new(1.0, 2.0, 3.0)));
	assert_eq!(entity.handle(), EntityHandle::from_raw(5 | 7 << 16));
	assert_eq!(entity.index(), Some(5));
	assert!(!entity.is_marked_for_deletion());

	let slot = TeleportSlot::TeamFortress2;

	// SAFETY: The mock's vtable has `teleport_entity` at TF2's `Teleport` slot.
	unsafe { entity.teleport(slot, Some(Vector::new(4.0, 5.0, 6.0)), None, None) }.unwrap();
	assert_eq!(entity.position(), Some(Vector::new(4.0, 5.0, 6.0)));
	assert_eq!(teleports(), 1);

	mock.set_eflags(EFL_KILLME);
	let entity = mock.entity();

	assert!(entity.is_marked_for_deletion());
	assert_eq!(
		// SAFETY: As above.
		unsafe { entity.teleport(slot, Some(Vector::new(7.0, 8.0, 9.0)), None, None) },
		Err(TeleportError::MarkedForDeletion)
	);
	assert_eq!(teleports(), 1);
}

#[test]
fn hammer_ids_are_read_from_the_member() {
	let mut mock = MockEntity::new(5);

	assert_eq!(mock.entity().hammer_id(), None);

	mock.set_hammer_id(1234);

	assert_eq!(mock.entity().hammer_id(), HammerId::new(1234));
	assert_eq!(mock.entity().hammer_id().map(HammerId::get), Some(1234));
}
