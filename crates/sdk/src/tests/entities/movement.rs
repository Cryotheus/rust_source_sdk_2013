//! Tests of how entities move: their flags, move types, velocities, gravity,
//! friction and move parents, read from the members their datamaps declare,
//! and the move types the game sets.

use super::*;
use crate::test_support::entities::{
	MOCK_EFLAGS_OFFSET, MOCK_STATE_OFFSET, MockEntity, MockPhysicsObject, MockState, set_datamap,
	state_maps,
};
use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server_tools::MockTools;
use std::mem::offset_of;

#[test]
fn absolute_angles_are_only_read_once_computed() {
	let mut mock = MockEntity::new(5);
	let state = mock.state();

	let abs = QAngle {
		pitch: 10.0,
		yaw: 90.0,
		roll: 0.0,
	};

	let local = QAngle {
		pitch: 0.0,
		yaw: 45.0,
		roll: 5.0,
	};

	set_datamap(state_maps(vec![]));
	state.abs_rotation = abs.into();
	state.rotation = local.into();
	state.move_parent = EntityHandle::INVALID.to_raw();

	assert_eq!(mock.entity().abs_angles(), Ok(Some(abs)));

	// Without a parent, the game computes them as the local angles.
	mock.set_eflags(EFL_DIRTY_ABSTRANSFORM);
	assert_eq!(mock.entity().abs_angles(), Ok(Some(local)));

	// With one, they are yet to be computed from the parent's.
	mock.state().move_parent = 9 | 3 << 16;
	assert_eq!(mock.entity().abs_angles(), Ok(None));

	// The velocity's flag leaves them computed.
	mock.set_eflags(EFL_DIRTY_ABSVELOCITY);
	assert_eq!(mock.entity().abs_angles(), Ok(Some(abs)));
}

#[test]
fn absolute_velocities_are_only_read_once_computed() {
	let mut mock = MockEntity::new(5);
	let state = mock.state();

	set_datamap(state_maps(vec![]));
	state.abs_velocity = Vector::new(1.0, 2.0, 3.0).into();
	state.velocity = Vector::new(4.0, 5.0, 6.0).into();
	state.move_parent = EntityHandle::INVALID.to_raw();

	assert_eq!(
		mock.entity().abs_velocity(),
		Ok(Some(Vector::new(1.0, 2.0, 3.0)))
	);
	assert_eq!(
		mock.entity().local_velocity(),
		Ok(Vector::new(4.0, 5.0, 6.0))
	);

	// Without a parent, the game computes it as the local velocity.
	mock.set_eflags(EFL_DIRTY_ABSVELOCITY);
	assert_eq!(
		mock.entity().abs_velocity(),
		Ok(Some(Vector::new(4.0, 5.0, 6.0)))
	);

	// With one, it is yet to be computed from the parent's.
	mock.state().move_parent = 9 | 3 << 16;
	assert_eq!(mock.entity().abs_velocity(), Ok(None));

	mock.set_eflags(0);
	assert_eq!(
		mock.entity().abs_velocity(),
		Ok(Some(Vector::new(1.0, 2.0, 3.0)))
	);
}

#[test]
fn engine_flags_are_read() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![]));
	mock.set_eflags(EFL_KILLME | EFL_DIRTY_ABSTRANSFORM | EFL_NO_DAMAGE_FORCES);

	assert_eq!(
		mock.entity().engine_flags(),
		Ok(EngineFlags::KILL_ME | EngineFlags::DIRTY_ABS_TRANSFORM | EngineFlags::NO_DAMAGE_FORCES)
	);

	// Two names share a bit, which shows as the first.
	mock.set_eflags(EFL_KEEP_ON_RECREATE_ENTITIES);

	let flags = mock.entity().engine_flags().unwrap();

	assert_eq!(flags, EngineFlags::HAS_PLAYER_CHILD);
	assert_eq!(format!("{flags:?}"), "EngineFlags(HAS_PLAYER_CHILD)");

	// Every bit has a name.
	assert_eq!(EngineFlags::all().bits(), !0);
}

#[test]
fn flags_and_move_types_are_read() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![]));
	mock.state().flags = FL_ONGROUND | FL_CLIENT | 1 << 30;
	mock.state().move_type = sys::MoveType_t_MOVETYPE_WALK as u8;

	assert_eq!(
		mock.entity().flags(),
		Ok(EntityFlags::ON_GROUND | EntityFlags::CLIENT | EntityFlags::from_bits_retain(1 << 30))
	);
	assert_eq!(mock.entity().move_type(), Ok(Some(MoveType::Walk)));

	mock.state().move_type = sys::MoveType_t_MOVETYPE_LAST as u8 + 1;
	assert_eq!(mock.entity().move_type(), Ok(None));
}

#[test]
fn flags_are_written() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();

	set_datamap(state_maps(vec![]));
	mock.state().flags = FL_ONGROUND | FL_CLIENT;

	let flags = mock.entity().flags().unwrap() | EntityFlags::FROZEN | EntityFlags::GOD_MODE;

	mock.entity().set_flags(engine, flags).unwrap();
	assert_eq!(
		mock.state().flags,
		FL_ONGROUND | FL_CLIENT | FL_FROZEN | FL_GODMODE
	);

	mock.entity()
		.set_flags(engine, flags - EntityFlags::GOD_MODE)
		.unwrap();
	assert_eq!(
		mock.entity().flags(),
		Ok(EntityFlags::ON_GROUND | EntityFlags::CLIENT | EntityFlags::FROZEN)
	);
}

#[test]
fn move_collide_types_are_read_and_convert_both_ways() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![]));
	mock.state().move_collide = sys::MoveCollide_t_MOVECOLLIDE_FLY_BOUNCE as u8;
	assert_eq!(
		mock.entity().move_collide(),
		Ok(Some(MoveCollide::FlyBounce))
	);

	mock.state().move_collide = sys::MoveCollide_t_MOVECOLLIDE_COUNT as u8;
	assert_eq!(mock.entity().move_collide(), Ok(None));

	for raw in 0..=u8::MAX {
		if let Some(collide) = MoveCollide::from_raw(raw) {
			assert_eq!(collide.to_raw(), raw);
		}
	}

	assert_eq!(MoveCollide::default().to_raw(), 0);
	assert_eq!(
		MoveCollide::from_raw(sys::MoveCollide_t_MOVECOLLIDE_FLY_SLIDE as u8),
		Some(MoveCollide::FlySlide)
	);
}

#[test]
fn move_parents_are_valid_handles() {
	let mut mock = MockEntity::new(5);

	set_datamap(state_maps(vec![]));
	mock.state().move_parent = EntityHandle::INVALID.to_raw();
	assert_eq!(mock.entity().move_parent(), Ok(None));

	mock.state().move_parent = 9 | 3 << 16;
	assert_eq!(
		mock.entity().move_parent(),
		Ok(Some(EntityHandle::from_raw(9 | 3 << 16)))
	);
}

#[test]
fn move_types_are_set_through_the_game() {
	let mut world = MockEntity::new(0);
	let mut mock = MockEntity::new(5);
	let mocks = MockTools::new(world.as_ptr());
	let tools = mocks.tools();

	tools.set_move_type(mock.entity(), MoveType::Fly);
	tools.set_move_type_and_collide(mock.entity(), MoveType::FlyGravity, MoveCollide::FlyCustom);

	assert_eq!(
		mocks.take_move_types(),
		[
			(mock.as_ptr(), sys::MoveType_t_MOVETYPE_FLY as c_int, None),
			(
				mock.as_ptr(),
				sys::MoveType_t_MOVETYPE_FLYGRAVITY as c_int,
				Some(sys::MoveCollide_t_MOVECOLLIDE_FLY_CUSTOM as c_int)
			),
		]
	);
}

#[test]
fn move_types_convert_both_ways() {
	for raw in 0..=u8::MAX {
		if let Some(move_type) = MoveType::from_raw(raw) {
			assert_eq!(move_type.to_raw(), raw);
		}
	}

	assert_eq!(
		MoveType::from_raw(sys::MoveType_t_MOVETYPE_CUSTOM as u8),
		Some(MoveType::Custom)
	);
}

#[test]
fn physics_objects_are_found_through_the_datamap() {
	let mut mock = MockEntity::new(5);
	let mut object = 0_u8;

	set_datamap(state_maps(vec![]));
	assert_eq!(mock.entity().has_physics_object(), Ok(false));

	mock.state().physics_object = (&raw mut object).cast();
	assert_eq!(mock.entity().has_physics_object(), Ok(true));
}

#[test]
fn a_physics_object_is_only_used_while_its_entity_holds_it() {
	let mut mock = MockEntity::new(5);
	let entity = mock.as_ptr();
	let object = MockPhysicsObject::leaked();
	let other = MockPhysicsObject::leaked();

	// Sets the entity's object, as the game would while the wrappers hold it.
	let set = |object: *mut c_void| {
		// SAFETY: The mock stores the aligned member at its offset, within its 64
		// words.
		unsafe {
			entity
				.byte_add(MOCK_STATE_OFFSET + offset_of!(MockState, physics_object))
				.cast::<*mut c_void>()
				.write(object)
		}
	};

	set_datamap(state_maps(vec![]));
	assert_eq!(mock.entity().physics_object(), Ok(None));

	set(object.as_ptr());

	let held = mock.entity().physics_object().unwrap().unwrap();

	assert_eq!(held.collisions_enabled(), Ok(true));
	assert_eq!(held.set_collisions_enabled(false), Ok(()));
	assert_eq!(held.collisions_enabled(), Ok(false));
	assert!(!object.enabled());
	assert_eq!(held.motion_enabled(), Ok(true));

	object.set_moving(false);
	assert_eq!(held.motion_enabled(), Ok(false));

	// Once the game replaces or destroys the object, it is not called.
	set(other.as_ptr());
	assert_eq!(
		held.set_collisions_enabled(true),
		Err(PhysicsObjectError::Gone)
	);

	set(std::ptr::null_mut());
	assert_eq!(held.collisions_enabled(), Err(PhysicsObjectError::Gone));

	set(object.as_ptr());

	// SAFETY: The mock stores the aligned flags at their offset, within its 64
	// words.
	unsafe {
		entity
			.byte_add(MOCK_EFLAGS_OFFSET)
			.cast::<c_int>()
			.write(EFL_KILLME)
	};

	assert_eq!(
		held.set_collisions_enabled(true),
		Err(PhysicsObjectError::MarkedForDeletion)
	);

	assert_eq!((object.switches(), other.switches()), (1, 0));
}
