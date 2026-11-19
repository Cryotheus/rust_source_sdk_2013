//! Tests of entities' native properties, read through the game's vtables and
//! data description maps, and of teleporting them.

use super::*;
use crate::test_support::edicts::set_change_accessor;
use crate::test_support::edicts::set_shared_change_info;

use crate::test_support::entities::{
	MOCK_STATE_OFFSET, MockEntity, MockState, owners_set, set_collision_group, set_datamap,
	set_networking, state_maps, teleports,
};

use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::edicts::FL_EDICT_CHANGED;
use sdk_raw::test_support::edicts::mock_edict;
use std::mem::offset_of;
use std::mem::zeroed;
use std::panic::AssertUnwindSafe;
use std::ptr::null_mut;

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
	assert_eq!(
		entity.bounds(),
		Some((Vector::new(0.0, 1.0, 2.0), Vector::new(2.0, 3.0, 4.0)))
	);
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

#[test]
fn owners_without_physics_objects_are_cleared_at_the_member() {
	let mut owner = MockEntity::new(9 | 3 << 16);
	let mut other = MockEntity::new(11 | 4 << 16);
	let mut mock = MockEntity::new(5 | 7 << 16);
	let engine = change_tracking_engine();
	// SAFETY: Zero is valid for every field of `CSharedEdictChangeInfo`.
	let mut shared = Box::new(unsafe { zeroed::<sys::CSharedEdictChangeInfo>() });
	let mut accessor = sys::IChangeInfoAccessor {
		m_iChangeInfo: 0,
		m_iChangeInfoSerialNumber: 0,
	};
	let mut edict = mock_edict(5, false);
	let member = u16::try_from(MOCK_STATE_OFFSET + offset_of!(MockState, owner)).unwrap();
	let owner_handle = owner.entity().handle();
	let mut physics_object = 0_u8;

	shared.m_iSerialNumber = 7;
	set_change_accessor(&raw mut accessor);
	set_shared_change_info(&raw mut *shared);
	set_networking(null_mut(), &raw mut edict);
	set_datamap(state_maps(vec![]));

	// An owner without a physics object is cleared at the member `owner`
	// reads, and the change recorded there, without the game's call.
	mock.state().owner = owner_handle.to_raw();
	assert_eq!(mock.entity().owner(), Ok(Some(owner_handle)));

	mock.entity().clear_owner(engine, owner.entity());

	assert_eq!(mock.state().owner, INVALID_EHANDLE_INDEX);
	assert_eq!(mock.entity().owner(), Ok(None));
	assert!(owners_set().is_empty());
	assert_ne!(edict._base.m_fStateFlags & FL_EDICT_CHANGED, 0);
	assert_eq!(shared.m_ChangeInfos[0].m_nChangeOffsets, 1);
	assert_eq!(shared.m_ChangeInfos[0].m_ChangeOffsets[0], member);

	// No owner: nothing is written, recorded or called.
	edict._base.m_fStateFlags = 0;
	mock.entity().clear_owner(engine, owner.entity());

	assert_eq!(edict._base.m_fStateFlags, 0);
	assert!(owners_set().is_empty());

	// An owner with a physics object, or another entity than the one named,
	// takes the game's way.
	owner.state().physics_object = (&raw mut physics_object).cast();
	mock.state().owner = owner_handle.to_raw();
	mock.entity().clear_owner(engine, owner.entity());

	mock.state().owner = owner_handle.to_raw();
	mock.entity().clear_owner(engine, other.entity());

	assert_eq!(owners_set(), [(mock.as_ptr(), null_mut()); 2]);
	assert_eq!(mock.state().owner, INVALID_EHANDLE_INDEX);
	assert_eq!(edict._base.m_fStateFlags, 0);

	// Within a VPhysics callback's scope, nothing changes, as with
	// `set_owner`, and a debug build panics.
	owner.state().physics_object = null_mut();
	mock.state().owner = owner_handle.to_raw();

	let scope = PhysicsCallbackScope::enter();
	let entity = mock.entity();
	let former = owner.entity();
	let refused = std::panic::catch_unwind(AssertUnwindSafe(|| entity.clear_owner(engine, former)));

	drop(scope);
	assert_eq!(refused.is_err(), cfg!(debug_assertions));
	assert_eq!(mock.state().owner, owner_handle.to_raw());
	assert_eq!(owners_set().len(), 2);
	assert_eq!(edict._base.m_fStateFlags, 0);

	set_change_accessor(null_mut());
	set_shared_change_info(null_mut());
	set_networking(null_mut(), null_mut());
}
