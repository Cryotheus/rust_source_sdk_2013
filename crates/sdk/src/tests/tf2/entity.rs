//! Tests of `crate::tf2::entity`: the native members of `CBaseEntity`,
//! called on mock entities whose script class records each call.

use super::*;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap, state_fields};
use crate::test_support::leak;
use crate::test_support::server::{mock_server, null_server};

use crate::test_support::tf2::script_binding::{
	class_description, member_binding, set_script_description,
};

use crate::tf2::damage::CUSTOM_DAMAGE_PLASMA;
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::tf2::script_binding::{FLOAT, HANDLE, INT, QANGLE, VECTOR};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};

/// A call of a native member: the entity, the member's name, and its
/// arguments.
type Call = (*mut c_void, CString, Vec<Argument>);

/// Where entities hold `m_hScriptInstance`, just before `m_iszScriptId`.
///
/// The crate finds the instance once per process, so every entity of its
/// tests holds it here, as `tests/tf2/script_instances.rs`'s do.
const INSTANCE_OFFSET: usize = 72;

/// The native members the mock entities' script class declares: each one's
/// name and parameter types. All return nothing, and others are missing.
const MEMBERS: [(&CStr, &[sys::ScriptDataType_t]); 15] = [
	(c"AddSolidFlags", &[INT]),
	(c"ApplyAbsVelocityImpulse", &[VECTOR]),
	(c"ApplyLocalAngularVelocityImpulse", &[VECTOR]),
	(c"RemoveSolidFlags", &[INT]),
	(c"SetAbsAngles", &[QANGLE]),
	(c"SetAbsOrigin", &[VECTOR]),
	(c"SetAbsVelocity", &[VECTOR]),
	(c"SetCollisionGroup", &[INT]),
	(c"SetLocalAngles", &[QANGLE]),
	(c"SetPhysAngularVelocity", &[VECTOR]),
	(c"SetPhysVelocity", &[VECTOR]),
	(c"SetSize", &[VECTOR, VECTOR]),
	(c"SetSolid", &[INT]),
	(c"SetSolidFlags", &[INT]),
	(
		c"TakeDamageCustom",
		&[HANDLE, HANDLE, HANDLE, VECTOR, VECTOR, FLOAT, INT, INT],
	),
];

/// Where entities hold `m_iszScriptId`, as their datamap declares.
const SCRIPT_ID_OFFSET: usize = 80;

/// An argument a native member received.
#[derive(Debug, Clone, PartialEq)]
enum Argument {
	Angles([f32; 3]),
	Float(f32),
	Handle(sys::HSCRIPT),
	Int(c_int),
	Vector([f32; 3]),
}

thread_local! {
	/// Whether the native members' adapters accept their calls.
	static ACCEPTS: Cell<bool> = const { Cell::new(true) };

	/// Each call of a native member, in order.
	static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn attachment_relative_angles_are_copied_and_nonfinite_angles_are_refused() {
	let mut mock = MockEntity::new(1);
	let object = mock.as_ptr().cast::<c_void>();
	set_maps();
	describe();
	let scope = ();
	let server = mock_server(&scope);
	let entity = TfEntity::new(server, mock.entity()).unwrap();
	entity
		.set_local_angles(QAngle {
			pitch: 0.0,
			yaw: 90.0,
			roll: 0.0,
		})
		.unwrap();
	assert_eq!(
		CALLS.take(),
		[(
			object,
			c"SetLocalAngles".to_owned(),
			vec![Argument::Angles([0.0, 90.0, 0.0])]
		)]
	);
	for angles in [
		QAngle {
			pitch: f32::NAN,
			yaw: 0.0,
			roll: 0.0,
		},
		QAngle {
			pitch: 0.0,
			yaw: f32::INFINITY,
			roll: 0.0,
		},
		QAngle {
			pitch: 0.0,
			yaw: 0.0,
			roll: f32::NEG_INFINITY,
		},
	] {
		assert!(matches!(
			entity.set_local_angles(angles),
			Err(TfEntityError::NonFinite)
		));
	}
	assert!(CALLS.take().is_empty());
}

#[test]
fn damage_always_has_an_attacker_and_an_inflictor() {
	let mut target = MockEntity::new(1);
	let mut soldier = MockEntity::new(2);
	let mut rocket = MockEntity::new(3);
	let mut launcher = MockEntity::new(4);
	let instances = [&mut target, &mut soldier, &mut rocket, &mut launcher].map(give_instance);
	let [
		target_instance,
		soldier_instance,
		rocket_instance,
		launcher_instance,
	] = instances;
	let object = target.as_ptr().cast::<c_void>();

	set_maps();
	describe();

	let scope = ();
	let server = mock_server(&scope);
	let soldier_entity = soldier.entity();
	let rocket_entity = rocket.entity();
	let launcher_entity = launcher.entity();
	let entity = TfEntity::new(server, target.entity()).unwrap();

	// With no one named, the entity damages itself.
	entity
		.take_damage(Damage::new(25.0, DamageType::BULLET))
		.unwrap();
	// An attacker inflicts its own damage, and an inflictor deals its own.
	entity
		.take_damage(Damage::new(50.0, DamageType::CLUB).attacker(soldier_entity))
		.unwrap();
	entity
		.take_damage(
			Damage::new(90.0, DamageType::BLAST)
				.inflictor(rocket_entity)
				.weapon(launcher_entity)
				.force(Vector::new(0.0, 0.0, 400.0))
				.position(Vector::new(1.0, 2.0, 3.0))
				.custom(CUSTOM_DAMAGE_PLASMA),
		)
		.unwrap();
	entity
		.take_damage(
			Damage::new(-5.0, DamageType::empty())
				.attacker(soldier_entity)
				.inflictor(rocket_entity),
		)
		.unwrap();

	let damage =
		|inflictor, attacker, weapon, force, position, amount, kinds: DamageType, custom| {
			(
				object,
				c"TakeDamageCustom".to_owned(),
				vec![
					Argument::Handle(inflictor),
					Argument::Handle(attacker),
					Argument::Handle(weapon),
					Argument::Vector(force),
					Argument::Vector(position),
					Argument::Float(amount),
					Argument::Int(kinds.bits() as c_int),
					Argument::Int(custom),
				],
			)
		};

	let zero = [0.0; 3];

	assert_eq!(
		CALLS.take(),
		[
			damage(
				target_instance,
				target_instance,
				null_mut(),
				zero,
				zero,
				25.0,
				DamageType::BULLET,
				0,
			),
			damage(
				soldier_instance,
				soldier_instance,
				null_mut(),
				zero,
				zero,
				50.0,
				DamageType::CLUB,
				0,
			),
			damage(
				rocket_instance,
				rocket_instance,
				launcher_instance,
				[0.0, 0.0, 400.0],
				[1.0, 2.0, 3.0],
				90.0,
				DamageType::BLAST,
				CUSTOM_DAMAGE_PLASMA,
			),
			damage(
				rocket_instance,
				soldier_instance,
				null_mut(),
				zero,
				zero,
				-5.0,
				DamageType::empty(),
				0,
			),
		]
	);

	// The damage must be finite.
	let nan = Vector::new(f32::NAN, 0.0, 0.0);

	for damage in [
		Damage::new(f32::INFINITY, DamageType::GENERIC),
		Damage::new(1.0, DamageType::GENERIC).force(nan),
		Damage::new(1.0, DamageType::GENERIC).position(nan),
	] {
		assert_eq!(entity.take_damage(damage), Err(TfEntityError::NonFinite));
	}

	assert!(CALLS.take().is_empty());
}

#[test]
fn damage_from_entities_marked_for_deletion_is_refused() {
	let mut target = MockEntity::new(1);
	let mut soldier = MockEntity::new(2);

	give_instance(&mut target);
	give_instance(&mut soldier);
	soldier.set_eflags(EFL_KILLME);
	set_maps();
	describe();

	let scope = ();
	let server = mock_server(&scope);
	let soldier = soldier.entity();
	let entity = TfEntity::new(server, target.entity()).unwrap();

	assert_eq!(
		entity.take_damage(Damage::new(1.0, DamageType::GENERIC).attacker(soldier)),
		Err(TfEntityError::ScriptInstance(
			ScriptInstanceError::MarkedForDeletion
		))
	);
	assert!(CALLS.take().is_empty());
}

/// Declares [`MEMBERS`] on the `CBaseEntity` script class that mock entities
/// on this thread return, each called through [`record`], which accepts
/// calls until told otherwise.
fn describe() {
	let bindings = MEMBERS
		.iter()
		.map(|&(name, parameters)| {
			let parameters = parameters.to_vec().leak();
			let mut binding = member_binding(name, binding::VOID, parameters, Some(record));

			binding.m_pFunction.val_0 = name.as_ptr() as isize;
			binding
		})
		.collect::<Vec<_>>();

	set_script_description(leak(class_description(
		c"CBaseEntity",
		bindings.leak(),
		null_mut(),
	)));
	ACCEPTS.set(true);
	CALLS.take();
}

#[test]
fn entities_are_refused_unless_live_on_tf2_with_the_members() {
	let mut mock = MockEntity::new(1);

	set_maps();
	describe();

	let scope = ();
	let server = mock_server(&scope);

	assert!(matches!(
		TfEntity::new(null_server(Game::SourceSdk2013, &scope), mock.entity()),
		Err(TfEntityError::NotTf2)
	));

	ACCEPTS.set(false);
	assert_eq!(
		TfEntity::new(server, mock.entity())
			.unwrap()
			.set_collision_group(TfCollisionGroup::Rockets),
		Err(TfEntityError::Rejected)
	);
	assert_eq!(take_names(), [c"SetCollisionGroup".to_owned()]);

	set_script_description(null_mut());
	assert_eq!(
		TfEntity::new(server, mock.entity())
			.unwrap()
			.add_solid_flags(SolidFlags::NOT_SOLID),
		Err(TfEntityError::UnsupportedMethod)
	);

	describe();
	mock.set_eflags(EFL_KILLME);

	let entity = TfEntity::new(server, mock.entity()).unwrap();

	assert_eq!(
		entity.set_solid_flags(SolidFlags::empty()),
		Err(TfEntityError::MarkedForDeletion)
	);
	assert_eq!(
		entity.take_damage(Damage::new(1.0, DamageType::GENERIC)),
		Err(TfEntityError::MarkedForDeletion)
	);
	assert!(CALLS.take().is_empty());
}

/// Gives a mock entity a script instance, which stands for it in native
/// members' arguments, and returns it.
fn give_instance(mock: &mut MockEntity) -> sys::HSCRIPT {
	let instance = leak(0_u8).cast::<sys::HSCRIPT__>();

	// SAFETY: The handle lies within the mock's 64 words, aligned for it.
	unsafe {
		mock.as_ptr()
			.byte_add(INSTANCE_OFFSET)
			.cast::<sys::HSCRIPT>()
			.write(instance);
	}

	instance
}

#[test]
fn members_receive_their_arguments() {
	let mut mock = MockEntity::new(1);
	let object = mock.as_ptr().cast::<c_void>();

	set_maps();
	describe();

	let scope = ();
	let server = mock_server(&scope);
	let entity = TfEntity::new(server, mock.entity()).unwrap();
	let flags = SolidFlags::NOT_SOLID | SolidFlags::TRIGGER;

	assert_eq!(entity.entity().as_ptr(), object.cast());
	entity.add_solid_flags(flags).unwrap();
	entity.remove_solid_flags(SolidFlags::TRIGGER).unwrap();
	entity.set_solid_flags(SolidFlags::empty()).unwrap();
	entity.set_solid(SolidType::BoundingBox).unwrap();
	entity
		.set_collision_group(TfCollisionGroup::RespawnRooms)
		.unwrap();
	entity.set_abs_origin(Vector::new(4.0, 5.0, 6.0)).unwrap();
	entity
		.set_abs_angles(QAngle {
			pitch: 10.0,
			yaw: 20.0,
			roll: 30.0,
		})
		.unwrap();
	entity.set_abs_velocity(Vector::new(1.0, 2.0, 3.0)).unwrap();
	entity.apply_impulse(Vector::new(0.0, 0.0, 300.0)).unwrap();
	entity
		.apply_angular_impulse(Vector::new(0.0, 90.0, 0.0))
		.unwrap();
	entity
		.set_size(Vector::new(-8.0, -8.0, 0.0), Vector::new(8.0, 8.0, 16.0))
		.unwrap();
	// Empty bounds are ordered.
	entity
		.set_size(Vector::new(1.0, 1.0, 1.0), Vector::new(1.0, 1.0, 1.0))
		.unwrap();

	let call = |name: &CStr, arguments| (object, name.to_owned(), arguments);

	assert_eq!(
		CALLS.take(),
		[
			call(
				c"AddSolidFlags",
				vec![Argument::Int(c_int::from(flags.bits()))]
			),
			call(
				c"RemoveSolidFlags",
				vec![Argument::Int(c_int::from(SolidFlags::TRIGGER.bits()))]
			),
			call(c"SetSolidFlags", vec![Argument::Int(0)]),
			call(
				c"SetSolid",
				vec![Argument::Int(c_int::from(SolidType::BoundingBox.to_raw()))]
			),
			call(
				c"SetCollisionGroup",
				vec![Argument::Int(TfCollisionGroup::RespawnRooms.to_raw())]
			),
			call(c"SetAbsOrigin", vec![Argument::Vector([4.0, 5.0, 6.0])]),
			call(c"SetAbsAngles", vec![Argument::Angles([10.0, 20.0, 30.0])]),
			call(c"SetAbsVelocity", vec![Argument::Vector([1.0, 2.0, 3.0])]),
			call(
				c"ApplyAbsVelocityImpulse",
				vec![Argument::Vector([0.0, 0.0, 300.0])]
			),
			call(
				c"ApplyLocalAngularVelocityImpulse",
				vec![Argument::Vector([0.0, 90.0, 0.0])]
			),
			call(
				c"SetSize",
				vec![
					Argument::Vector([-8.0, -8.0, 0.0]),
					Argument::Vector([8.0, 8.0, 16.0]),
				]
			),
			call(
				c"SetSize",
				vec![
					Argument::Vector([1.0, 1.0, 1.0]),
					Argument::Vector([1.0, 1.0, 1.0]),
				]
			),
		]
	);
}

#[test]
fn physics_needs_a_physics_object() {
	let mut mock = MockEntity::new(1);
	let mut object = 0_u8;

	set_maps();
	describe();

	let scope = ();
	let server = mock_server(&scope);
	let velocity = Vector::new(0.0, 0.0, 100.0);

	// Without one, only entities VPhysics does not move can be pushed.
	{
		let entity = TfEntity::new(server, mock.entity()).unwrap();

		entity.apply_impulse(velocity).unwrap();
		assert_eq!(
			entity.set_physics_velocity(velocity),
			Err(TfEntityError::NoPhysicsObject)
		);
		assert_eq!(
			entity.set_physics_angular_velocity(velocity),
			Err(TfEntityError::NoPhysicsObject)
		);
	}

	mock.state().move_type = MoveType::VPhysics.to_raw();

	{
		let entity = TfEntity::new(server, mock.entity()).unwrap();

		assert_eq!(
			entity.apply_impulse(velocity),
			Err(TfEntityError::NoPhysicsObject)
		);
		assert_eq!(
			entity.apply_angular_impulse(velocity),
			Err(TfEntityError::NoPhysicsObject)
		);
	}

	assert_eq!(take_names(), [c"ApplyAbsVelocityImpulse".to_owned()]);
	mock.state().physics_object = (&raw mut object).cast();

	let entity = TfEntity::new(server, mock.entity()).unwrap();

	entity.apply_impulse(velocity).unwrap();
	entity.apply_angular_impulse(velocity).unwrap();
	entity.set_physics_velocity(velocity).unwrap();
	entity.set_physics_angular_velocity(velocity).unwrap();
	assert_eq!(
		take_names(),
		[
			c"ApplyAbsVelocityImpulse".to_owned(),
			c"ApplyLocalAngularVelocityImpulse".to_owned(),
			c"SetPhysVelocity".to_owned(),
			c"SetPhysAngularVelocity".to_owned(),
		]
	);
}

/// The adapter of every native member, which records the call in [`CALLS`]
/// and returns whether [`ACCEPTS`] says to accept it.
unsafe extern "C" fn record(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert!(result.is_null(), "every member returns nothing");

	// SAFETY: `describe` stores each member's name in its binding, and the
	// caller passes `count` arguments of the types the binding declares, whose
	// vectors it keeps alive for the call.
	let (name, arguments) = unsafe {
		(
			CStr::from_ptr(function.val_0 as *const c_char),
			std::slice::from_raw_parts(arguments, usize::try_from(count).unwrap()),
		)
	};

	let arguments = arguments
		.iter()
		.map(|argument| {
			let value = &argument.__bindgen_anon_1;

			// SAFETY: As above, each variant's member is the one its type names.
			unsafe {
				match c_int::from(argument.m_type) {
					FLOAT => Argument::Float(value.m_float),
					HANDLE => Argument::Handle(value.m_hScript),
					INT => Argument::Int(value.m_int),

					VECTOR => {
						let vector = &*value.m_pVector;

						Argument::Vector([vector.x, vector.y, vector.z])
					}

					QANGLE => {
						let angles = &*value.m_pData.cast::<sys::QAngle>();

						Argument::Angles([angles.x, angles.y, angles.z])
					}

					other => panic!("unexpected argument type {other}"),
				}
			}
		})
		.collect();

	CALLS.with_borrow_mut(|calls| calls.push((object, name.to_owned(), arguments)));
	ACCEPTS.get()
}

/// A datamap whose `CBaseEntity` map declares the members mock entities
/// store, and the script ID, for this thread's mock entities.
fn set_maps() {
	let mut fields = Vec::from(base_entity_fields());
	let mut script_id = field(
		c"m_iszScriptId",
		sys::_fieldtypes_FIELD_STRING,
		SCRIPT_ID_OFFSET,
	);

	script_id.fieldSizeInBytes = size_of::<sys::string_t>() as c_int;
	fields.extend(state_fields());
	fields.push(script_id);
	set_datamap(data_map(c"CBaseEntity", fields, null_mut()));
}

/// The names of the members called since the last call of this, in order.
fn take_names() -> Vec<CString> {
	CALLS.take().into_iter().map(|(_, name, _)| name).collect()
}

#[test]
fn transform_refresh_uses_native_virtual_and_refuses_deleted_entities() {
	thread_local! { static REFRESHES: Cell<usize> = const { Cell::new(0) }; }
	unsafe extern "C" fn refresh(
		_object: *const sys::CBaseEntity,
		forward: *mut sys::Vector,
		right: *mut sys::Vector,
		up: *mut sys::Vector,
	) {
		assert!(forward.is_null() && right.is_null() && up.is_null());
		REFRESHES.set(REFRESHES.get() + 1);
	}
	let mut mock = MockEntity::new(1);
	let slot = sdk_raw::vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetVectors);
	// SAFETY: MockEntity allocates this many initialized function-pointer
	// slots. Copy into a larger owned mock table before adding GetVectors.
	unsafe {
		let object = mock.as_ptr().cast::<*const *const ()>();
		let mut vtable =
			std::slice::from_raw_parts(object.read(), MockEntity::vtable_slots()).to_vec();
		vtable.resize(vtable.len().max(slot + 1), refresh as *const ());
		vtable[slot] = refresh as *const ();
		object.write(vtable.leak().as_ptr());
	}
	set_maps();
	let scope = ();
	let server = mock_server(&scope);
	TfEntity::new(server, mock.entity())
		.unwrap()
		.refresh_transform()
		.unwrap();
	assert_eq!(REFRESHES.get(), 1);
	mock.set_eflags(EFL_KILLME);
	assert_eq!(
		TfEntity::new(server, mock.entity())
			.unwrap()
			.refresh_transform(),
		Err(TfEntityError::MarkedForDeletion)
	);
	assert_eq!(REFRESHES.get(), 1);
}

#[test]
fn values_the_game_mishandles_are_refused() {
	let mut mock = MockEntity::new(1);

	set_maps();
	describe();

	let scope = ();
	let server = mock_server(&scope);
	let entity = TfEntity::new(server, mock.entity()).unwrap();
	let finite = Vector::new(1.0, 2.0, 3.0);

	for value in [
		Vector::new(f32::NAN, 0.0, 0.0),
		Vector::new(0.0, f32::INFINITY, 0.0),
		Vector::new(0.0, 0.0, f32::NEG_INFINITY),
	] {
		let angles = QAngle {
			pitch: value.x,
			yaw: value.y,
			roll: value.z,
		};

		for result in [
			entity.set_abs_origin(value),
			entity.set_abs_angles(angles),
			entity.set_abs_velocity(value),
			entity.apply_impulse(value),
			entity.apply_angular_impulse(value),
			entity.set_physics_velocity(value),
			entity.set_physics_angular_velocity(value),
			entity.set_size(value, finite),
			entity.set_size(Vector::new(-1.0, -2.0, -3.0), value),
		] {
			assert_eq!(result, Err(TfEntityError::NonFinite), "{value:?}");
		}
	}

	// A minimum above its maximum would stop the server.
	for (mins, maxs) in [
		(Vector::new(0.0, 0.0, 1.0), Vector::new(8.0, 8.0, 0.0)),
		(Vector::new(9.0, -8.0, 0.0), Vector::new(8.0, 8.0, 8.0)),
	] {
		assert_eq!(
			entity.set_size(mins, maxs),
			Err(TfEntityError::InvalidBounds)
		);
	}

	assert!(CALLS.take().is_empty());
}
