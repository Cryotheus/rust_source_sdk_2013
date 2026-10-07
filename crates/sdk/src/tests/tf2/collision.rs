//! Tests of TF2's collision group numbers.

use super::*;

use sdk_raw::entities::{
	COLLISION_GROUP_NONE, COLLISION_GROUP_PROJECTILE, LAST_SHARED_COLLISION_GROUP,
};

#[test]
fn numbers_without_a_group_are_refused() {
	let past = LAST_SHARED_COLLISION_GROUP + c_int::try_from(TfCollisionGroup::OWN.len()).unwrap();

	for raw in [-1, past, past + 1, c_int::MAX] {
		assert_eq!(TfCollisionGroup::from_raw(raw), None);
	}
}

#[test]
fn own_groups_follow_the_shared_ones_in_order() {
	for (offset, group) in TfCollisionGroup::OWN.into_iter().enumerate() {
		let raw = LAST_SHARED_COLLISION_GROUP + c_int::try_from(offset).unwrap();

		assert_eq!(group.to_raw(), raw);
		assert_eq!(TfCollisionGroup::from_raw(raw), Some(group));
		assert_eq!(group.shared(), None);
	}

	assert_eq!(
		TfCollisionGroup::Rockets.to_raw(),
		LAST_SHARED_COLLISION_GROUP + 4
	);
}

#[test]
fn shared_groups_convert_both_ways() {
	for raw in COLLISION_GROUP_NONE..LAST_SHARED_COLLISION_GROUP {
		let shared = CollisionGroup::from_raw(raw).unwrap();
		let group = TfCollisionGroup::from_raw(raw).unwrap();

		assert_eq!(group, TfCollisionGroup::Shared(shared));
		assert_eq!(group, TfCollisionGroup::from(shared));
		assert_eq!(group.shared(), Some(shared));
		assert_eq!(group.to_raw(), raw);
	}

	assert_eq!(
		TfCollisionGroup::from_raw(COLLISION_GROUP_PROJECTILE),
		Some(TfCollisionGroup::Shared(CollisionGroup::Projectile))
	);
}
