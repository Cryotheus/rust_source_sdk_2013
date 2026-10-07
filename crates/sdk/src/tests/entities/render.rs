//! Tests of how clients draw entities: effects, which update the transmit
//! state as the game's setters do, render modes and colors, models, skins,
//! body groups and fade scales.

use super::*;

use crate::test_support::entities::{
	MockEntity, models_set, set_datamap, set_networking, state_maps, transmit_state_updates,
};

use crate::test_support::leak;
use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::field;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::ffi::c_char;
use std::ptr::{NonNull, null_mut};

/// Where the test's `CBaseAnimating` map declares `m_nSkin`, `m_nBody` and
/// `m_flFadeScale`, one after another.
const ANIMATING_OFFSET: usize = 320;

/// A `CBaseAnimating` map declaring `m_nSkin`, `m_nBody` and
/// `m_flFadeScale`.
fn animating() -> (&'static CStr, Vec<sys::typedescription_t>) {
	let member = |name, field_type, offset| {
		let mut member = field(name, field_type, ANIMATING_OFFSET + offset);

		member.fieldSize = 1;
		member.fieldSizeInBytes = size_of::<c_int>() as c_int;
		member
	};

	(
		c"CBaseAnimating",
		vec![
			member(c"m_nSkin", sys::_fieldtypes_FIELD_INTEGER, 0),
			member(c"m_nBody", sys::_fieldtypes_FIELD_INTEGER, 4),
			member(c"m_flFadeScale", sys::_fieldtypes_FIELD_FLOAT, 8),
		],
	)
}

#[test]
fn effects_update_the_transmit_state_as_the_game_sets_them() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();
	let mut edict = mock_edict(5, false);

	set_datamap(state_maps(vec![]));
	set_networking(null_mut(), &raw mut edict);
	mock.state().effects = EF_NOSHADOW;

	assert_eq!(mock.entity().effects(), Ok(Effects::NO_SHADOW));

	// Setting the same effects changes nothing.
	mock.entity()
		.set_effects(engine, Effects::NO_SHADOW)
		.unwrap();
	assert_eq!(transmit_state_updates(), 0);

	// Other effects update it.
	mock.entity()
		.set_effects(engine, Effects::NO_SHADOW | Effects::NO_DRAW)
		.unwrap();
	assert_eq!(mock.state().effects, EF_NOSHADOW | EF_NODRAW);
	assert_eq!(transmit_state_updates(), 1);
	assert_eq!(
		edict._base.m_fStateFlags & FL_EDICT_DIRTY_PVS_INFORMATION,
		0
	);

	// Drawing the entity again marks where it is as to be found again.
	mock.entity().set_effects(engine, Effects::empty()).unwrap();
	assert_eq!(mock.state().effects, 0);
	assert_eq!(transmit_state_updates(), 2);
	assert_ne!(
		edict._base.m_fStateFlags & FL_EDICT_DIRTY_PVS_INFORMATION,
		0
	);

	// Bits without a constant are kept.
	mock.entity()
		.set_effects(engine, Effects::from_bits_retain(1 << 20))
		.unwrap();
	assert_eq!(
		mock.entity().effects().map(|effects| effects.bits()),
		Ok(1 << 20)
	);

	set_networking(null_mut(), null_mut());
}

/// `IVModelInfo::GetModelIndex`, which knows `models/box.mdl` as precached,
/// at 3, and `models/dynamic.mdl` as a dynamic model.
unsafe extern "C" fn model_index(_: *const sys::IVModelInfo, name: *const c_char) -> c_int {
	// SAFETY: The wrapper passes a NUL-terminated name.
	match unsafe { CStr::from_ptr(name) }.to_bytes() {
		b"models/box.mdl" => 3,
		b"models/dynamic.mdl" => -3,
		_ => -1,
	}
}

#[test]
fn models_are_only_set_once_precached() {
	let mut mock = MockEntity::new(5);

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
	// what setting a model calls.
	let models = unsafe { ModelInfo::from_raw(NonNull::new(raw).unwrap()) };

	for model in [c"models/missing.mdl", c"models/dynamic.mdl"] {
		assert_eq!(
			mock.entity().set_model(models, model),
			Err(SetModelError::NotPrecached {
				model: model.to_owned()
			})
		);
	}

	mock.entity().set_model(models, c"models/box.mdl").unwrap();
	assert_eq!(
		models_set(),
		[(mock.as_ptr(), c"models/box.mdl".to_owned())]
	);
}

#[test]
fn owned_transmit_states_are_left_to_their_owners() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();

	set_datamap(state_maps(vec![]));
	mock.state().transmit_state_owners = 1;

	mock.entity().update_transmit_state().unwrap();
	mock.entity().set_effects(engine, Effects::NO_DRAW).unwrap();
	assert_eq!(mock.state().effects, EF_NODRAW);
	assert_eq!(transmit_state_updates(), 0);

	mock.state().transmit_state_owners = 0;
	mock.entity().update_transmit_state().unwrap();
	assert_eq!(transmit_state_updates(), 1);
}

#[test]
fn render_modes_convert_both_ways() {
	for raw in 0..=u8::MAX {
		if let Some(mode) = RenderMode::from_raw(raw) {
			assert_eq!(mode.to_raw(), raw);
		}
	}

	assert_eq!(
		RenderMode::from_raw(sys::RenderMode_t_kRenderNone as u8),
		Some(RenderMode::None)
	);
	assert_eq!(
		RenderMode::from_raw(sys::RenderMode_t_kRenderModeCount as u8),
		None
	);
}

#[test]
fn render_state_is_read_and_written() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();

	set_datamap(state_maps(vec![(c"CTFPlayer", vec![]), animating()]));

	let state = mock.state();

	state.render_mode = sys::RenderMode_t_kRenderTransAlpha as u8;
	state.render_color = Color32::new(255, 128, 0, 200).into();
	state.model_index = 42;
	state.model_name = sys::string_t {
		pszValue: c"models/player/heavy.mdl".as_ptr(),
	};

	let entity = mock.entity();

	assert_eq!(entity.render_mode(), Ok(Some(RenderMode::TransAlpha)));
	assert_eq!(entity.render_color(), Ok(Color32::new(255, 128, 0, 200)));
	assert_eq!(entity.model_index(), Ok(42));
	assert_eq!(entity.model_name(), Ok(Some(c"models/player/heavy.mdl")));

	entity.set_render_mode(engine, RenderMode::Glow).unwrap();
	entity
		.set_render_color(engine, Color32::new(1, 2, 3, 4))
		.unwrap();
	entity.set_skin(engine, 1).unwrap();
	entity.set_body(engine, 6).unwrap();
	entity.set_fade_scale(engine, 0.5).unwrap();

	assert_eq!(entity.render_mode(), Ok(Some(RenderMode::Glow)));
	assert_eq!(entity.render_color(), Ok(Color32::new(1, 2, 3, 4)));
	assert_eq!(entity.skin(), Ok(1));
	assert_eq!(entity.body(), Ok(6));
	assert_eq!(entity.fade_scale(), Ok(0.5));
	assert_eq!(
		mock.state().render_mode,
		sys::RenderMode_t_kRenderGlow as u8
	);
	assert_eq!(mock.int(ANIMATING_OFFSET + 4), 6);

	// Render modes past the last are none the SDK knows.
	mock.state().render_mode = 200;
	assert_eq!(mock.entity().render_mode(), Ok(None));
}
