//! Tests of how entities move: their flags, move types, velocities, gravity,
//! friction and move parents, read from the members their datamaps declare.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use crate::test_support::sdk_core::change_tracking_engine;

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
fn gravity_and_friction_are_read_and_written() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();

	set_datamap(state_maps(vec![]));
	mock.state().gravity = 0.5;
	mock.state().friction = 1.0;

	assert_eq!(mock.entity().gravity(), Ok(0.5));
	assert_eq!(mock.entity().friction(), Ok(1.0));

	mock.entity().set_gravity(engine, 2.0).unwrap();
	mock.entity().set_friction(engine, 0.25).unwrap();

	assert_eq!(mock.state().gravity, 2.0);
	assert_eq!(mock.state().friction, 0.25);
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
