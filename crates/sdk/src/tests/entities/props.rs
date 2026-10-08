//! Tests of animated props: the key values they spawn with, models that are
//! not precached, props that remove themselves, and the inputs they are
//! sent.

use super::*;

use crate::test_support::entities::{
	MockEntity, activations, input, set_accepts, set_datamap, state_maps, take_inputs,
};

use crate::test_support::leak;
use crate::test_support::server_tools::MockTools;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::ffi::c_char;
use std::ptr::NonNull;

/// The model the mock model info knows as precached.
const MODEL: &CStr = c"models/player/heavy.mdl";

struct Mocks {
	tools: MockTools,
	prop: MockEntity,
	target: MockEntity,
	_world: MockEntity,
}

impl Mocks {
	/// The mock prop, wrapped.
	fn prop(&mut self) -> DynamicProp<'static> {
		DynamicProp::new(self.tools.tools(), entity(&mut self.prop)).unwrap()
	}
}

#[test]
fn classes_and_solid_types_have_their_game_values() {
	assert_eq!(
		[
			DynamicPropClass::Dynamic,
			DynamicPropClass::Override,
			DynamicPropClass::Ornament,
		]
		.map(DynamicPropClass::class_name),
		[
			c"prop_dynamic",
			c"prop_dynamic_override",
			c"prop_dynamic_ornament"
		]
	);
	assert_eq!(
		[PropSolid::None, PropSolid::BoundingBox, PropSolid::Vphysics].map(PropSolid::to_raw),
		[0, 2, 6]
	);
	assert_eq!(PropSolid::default(), PropSolid::Vphysics);
	assert_eq!(DynamicPropClass::default(), DynamicPropClass::Dynamic);
}

/// A mock entity, for as long as the mocks live.
fn entity(mock: &mut MockEntity) -> Entity<'static> {
	// SAFETY: Mock entities are leaked, and their vtables answer what the
	// wrappers call of a `CBaseEntity`.
	unsafe { Entity::from_raw(NonNull::new(mock.as_ptr()).unwrap()) }
}

/// `IVModelInfo::GetModelIndex`, which knows only [`MODEL`] as precached.
unsafe extern "C" fn model_index(_: *const sys::IVModelInfo, name: *const c_char) -> c_int {
	// SAFETY: The wrapper passes a NUL-terminated name.
	if unsafe { CStr::from_ptr(name) } == MODEL {
		5
	} else {
		-1
	}
}

/// Mock model info, which knows only [`MODEL`] as precached.
fn models() -> ModelInfo<'static> {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::IVModelInfo__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVModelInfo_GetModelIndex).write(model_index);
		})
	};
	let raw = leak(sys::IVModelInfo {
		vtable_: Box::leak(vtable),
	});

	// SAFETY: The model info and its vtable are leaked, and the vtable answers
	// what spawning a prop calls.
	unsafe { ModelInfo::from_raw(NonNull::new(raw).unwrap()) }
}

#[test]
fn ornaments_attach_to_entities_through_the_activator() {
	let mut mocks = setup(true);
	let prop = mocks.prop();
	let target = entity(&mut mocks.target);

	prop.attach(target).unwrap();
	prop.detach().unwrap();

	let inputs = take_inputs();

	assert_eq!(inputs.len(), 2);
	assert_eq!(inputs[0].name.as_c_str(), c"SetAttached");
	assert_eq!(
		(inputs[0].target, inputs[0].activator, inputs[0].caller),
		(mocks.prop.as_ptr(), target.as_ptr(), mocks.prop.as_ptr())
	);
	// SAFETY: The value was pooled, and stays allocated for the rest of the
	// thread.
	assert_eq!(unsafe { CStr::from_ptr(inputs[0].string) }, c"!activator");
	assert_eq!(inputs[1].name.as_c_str(), c"Detach");

	// A prop that is no ornament.
	let mut mocks = setup(false);
	let prop = mocks.prop();
	let target = entity(&mut mocks.target);
	let wrong_class = Err(PropError::WrongClass {
		class: c"COrnamentProp",
	});

	assert_eq!(prop.attach(target), wrong_class);
	assert_eq!(prop.detach(), wrong_class);
	assert!(take_inputs().is_empty());
}

#[test]
fn props_are_sent_their_inputs() {
	let mut mocks = setup(false);
	let prop = mocks.prop();
	let target = mocks.prop.as_ptr();

	prop.set_animation(c"taunt01").unwrap();
	prop.set_default_animation(c"stand_MELEE").unwrap();
	prop.hide().unwrap();
	prop.show().unwrap();
	prop.disable_collision().unwrap();
	prop.enable_collision().unwrap();

	let inputs = take_inputs();
	let names: Vec<_> = inputs.iter().map(|input| input.name.as_c_str()).collect();

	assert_eq!(
		names,
		[
			c"SetAnimation",
			c"SetDefaultAnimation",
			c"TurnOff",
			c"TurnOn",
			c"DisableCollision",
			c"EnableCollision",
		]
	);
	assert!(inputs.iter().all(|input| {
		(input.target, input.activator, input.caller) == (target, target, target)
	}));
	// SAFETY: The values were pooled, and stay allocated for the rest of the
	// thread.
	assert_eq!(
		[0, 1].map(|index| unsafe { CStr::from_ptr(inputs[index].string) }),
		[c"taunt01", c"stand_MELEE"]
	);

	set_accepts(false);
	assert_eq!(prop.hide(), Err(PropError::Input(InputError::Rejected)));
}

#[test]
fn props_are_spawned_with_their_key_values_and_activated() {
	let mut mocks = setup(false);
	let prop = mocks.prop.as_ptr();
	let mut spawn = DynamicPropSpawn::new(MODEL, Vector::new(1.0, -2.5, 64.0));

	spawn.class = DynamicPropClass::Override;
	spawn.angles = QAngle {
		pitch: 0.0,
		yaw: 90.0,
		roll: 0.0,
	};
	spawn.solid = PropSolid::None;
	spawn.animation = Some(c"stand_MELEE");
	spawn.skin = 1;
	spawn.start_disabled = true;
	spawn.name = Some(c"actor");

	let spawned = DynamicProp::spawn(mocks.tools.tools(), models(), &spawn).unwrap();
	let key = |key: &CStr, value: &CStr| (prop, key.to_owned(), value.to_owned());

	assert_eq!(spawned.entity().as_ptr(), prop);
	assert_eq!(
		mocks.tools.take_classes(),
		[c"prop_dynamic_override".to_owned()]
	);
	assert_eq!(
		mocks.tools.take_keys(),
		[
			key(c"model", MODEL),
			key(c"origin", c"1 -2.5 64"),
			key(c"angles", c"0 90 0"),
			key(c"solid", c"0"),
			key(c"skin", c"1"),
			key(c"StartDisabled", c"1"),
			key(c"DefaultAnim", c"stand_MELEE"),
			key(c"targetname", c"actor"),
		]
	);
	assert_eq!(mocks.tools.take_spawned(), [prop]);
	assert_eq!(activations(), [prop]);

	// The defaults leave out the animation and name.
	mocks.tools.set_created(prop);
	DynamicProp::spawn(
		mocks.tools.tools(),
		models(),
		&DynamicPropSpawn::new(MODEL, Vector::new(0.0, 0.0, 0.0)),
	)
	.unwrap();
	assert_eq!(mocks.tools.take_classes(), [c"prop_dynamic".to_owned()]);
	assert_eq!(
		mocks.tools.take_keys(),
		[
			key(c"model", MODEL),
			key(c"origin", c"0 0 0"),
			key(c"angles", c"0 0 0"),
			key(c"solid", c"6"),
			key(c"skin", c"0"),
			key(c"StartDisabled", c"0"),
		]
	);
}

#[test]
fn props_that_cannot_spawn_are_reported() {
	let mut mocks = setup(false);
	let missing = DynamicPropSpawn::new(c"models/missing.mdl", Vector::new(0.0, 0.0, 0.0));

	assert_eq!(
		DynamicProp::spawn(mocks.tools.tools(), models(), &missing).err(),
		Some(PropError::ModelNotPrecached {
			model: c"models/missing.mdl".to_owned()
		})
	);
	assert!(mocks.tools.take_classes().is_empty());

	let mut spawn = DynamicPropSpawn::new(MODEL, Vector::new(f32::NAN, 0.0, 0.0));

	assert_eq!(
		DynamicProp::spawn(mocks.tools.tools(), models(), &spawn).err(),
		Some(PropError::Spawn(SpawnError::NonFinite {
			key: c"origin".to_owned()
		}))
	);
	assert!(mocks.tools.take_classes().is_empty());

	// A `prop_dynamic` refusing its model removes itself.
	spawn.origin = Vector::new(0.0, 0.0, 0.0);
	mocks.tools.set_spawn_removes(true);
	assert_eq!(
		DynamicProp::spawn(mocks.tools.tools(), models(), &spawn).err(),
		Some(PropError::Spawn(SpawnError::RemovedItself))
	);
	assert!(activations().is_empty());

	// An entity that is no prop.
	set_datamap(state_maps(vec![(c"CBaseAnimating", vec![])]));
	assert_eq!(
		DynamicProp::new(mocks.tools.tools(), entity(&mut mocks.target)).err(),
		Some(PropError::WrongClass {
			class: c"CDynamicProp"
		})
	);
}

/// Mock tools that create the mock prop, whose class is a `CDynamicProp`,
/// or a `COrnamentProp` deriving from it if `ornament`, declaring the
/// inputs the wrappers send.
fn setup(ornament: bool) -> Mocks {
	let mut world = MockEntity::new(0);
	let mut mocks = Mocks {
		tools: MockTools::new(world.as_ptr()),
		prop: MockEntity::new(7 | 3 << 16),
		target: MockEntity::new(9 | 1 << 16),
		_world: world,
	};

	let mut classes = Vec::new();

	if ornament {
		classes.push((
			c"COrnamentProp",
			vec![
				input(c"SetAttached", sys::_fieldtypes_FIELD_STRING),
				input(c"Detach", sys::_fieldtypes_FIELD_VOID),
			],
		));
	}

	classes.push((
		c"CDynamicProp",
		[
			(c"SetAnimation", sys::_fieldtypes_FIELD_STRING),
			(c"SetDefaultAnimation", sys::_fieldtypes_FIELD_STRING),
			(c"TurnOn", sys::_fieldtypes_FIELD_VOID),
			(c"TurnOff", sys::_fieldtypes_FIELD_VOID),
			(c"EnableCollision", sys::_fieldtypes_FIELD_VOID),
			(c"DisableCollision", sys::_fieldtypes_FIELD_VOID),
		]
		.into_iter()
		.map(|(name, field_type)| input(name, field_type))
		.collect(),
	));
	classes.push((c"CBaseAnimating", vec![]));

	set_datamap(state_maps(classes));
	mocks.tools.set_created(mocks.prop.as_ptr());
	set_accepts(true);
	take_inputs();

	mocks
}
