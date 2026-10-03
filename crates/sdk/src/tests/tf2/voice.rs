//! Tests of `crate::tf2::voice`: speaking scenes through a fake TF2 player.

use super::*;

use crate::test_support::entities::{
	MOCK_EFLAGS_OFFSET, base_entity_fields, get_datamap, set_datamap,
};

use crate::test_support::server::mock_server;

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use sdk_raw::players::{LIFE_DEAD, LIFE_DYING};
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::tf2::script_binding::STRING;
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void};
use std::mem::offset_of;
use std::ptr::NonNull;

const PLAY_SCENE: usize =
	sdk_raw::vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_PlayScene);

thread_local! {
	static LENGTH: Cell<f32> = const { Cell::new(0.0) };
	static PLAYED: RefCell<Vec<(*mut c_void, CString, f32)>> = const { RefCell::new(Vec::new()) };
	static REJECT: Cell<bool> = const { Cell::new(false) };
}

/// A player whose fields lie where its datamaps say.
#[repr(C)]
struct FakePlayer {
	vtable: *const *const (),
	padding: [usize; 3],
	eflags: c_int,
	more_padding: [u8; 28],
	life_state: u8,
}

impl FakePlayer {
	fn new(vtable: &[*const ()]) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
			padding: [0; 3],
			eflags: 0,
			more_padding: [0; 28],
			life_state: LIFE_ALIVE,
		})
	}
}

/// The `m_lifeState` declaration of [`FakePlayer`].
fn life_state_field() -> sys::typedescription_t {
	let mut life_state = field(
		c"m_lifeState",
		sys::_fieldtypes_FIELD_CHARACTER,
		offset_of!(FakePlayer, life_state),
	);

	life_state.fieldSize = 1;
	life_state.fieldSizeInBytes = 1;
	life_state
}

#[test]
fn living_tf_players_speak_scenes_through_the_generated_vtable() {
	assert_eq!(offset_of!(FakePlayer, eflags), MOCK_EFLAGS_OFFSET);

	let mut vtable = vec![std::ptr::null(); PLAY_SCENE + 1];

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = get_datamap as *const ();
	vtable[PLAY_SCENE] = play_scene as *const ();

	let player = Box::into_raw(FakePlayer::new(&vtable));
	// SAFETY: The fake player stays allocated until it is dropped at the end of
	// the test, after the last use of the handle, and its vtable answers what
	// the speaker calls.
	let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
	let scope = ();
	let server = mock_server(&scope);

	set_datamap(maps(c"CTFPlayer", Some(life_state_field())));
	PLAYED.take();

	let speaker = Speaker::new(server, entity).unwrap();
	let thanks = ScenePath::voice_line(PlayerClass::Scout, 508).unwrap();

	assert!(speaker.is_alive());
	LENGTH.set(0.8156);
	assert_eq!(
		speaker.play_scene(&thanks),
		Ok(Duration::from_secs_f32(0.8156))
	);
	assert_eq!(
		PLAYED.take(),
		[(
			player.cast(),
			c"scenes/Player/Scout/low/508.vcd".to_owned(),
			0.0
		)]
	);

	// The game reports 0 for a scene it does not have.
	for length in [0.0, -1.0, f32::NAN, f32::INFINITY] {
		LENGTH.set(length);
		assert_eq!(speaker.play_scene(&thanks), Err(VoiceError::MissingScene));
	}

	PLAYED.take();

	// Dead or deleted players are refused before the game is called.
	// SAFETY: The fake player is allocated, and no reference to it is live.
	unsafe { (&raw mut (*player).life_state).write(LIFE_DEAD) };
	assert!(!speaker.is_alive());
	assert_eq!(speaker.play_scene(&thanks), Err(VoiceError::NotAlive));
	// SAFETY: As above.
	unsafe {
		(&raw mut (*player).life_state).write(LIFE_ALIVE);
		(&raw mut (*player).eflags).write(1);
	}
	assert_eq!(
		speaker.play_scene(&thanks),
		Err(VoiceError::MarkedForDeletion)
	);
	assert!(PLAYED.take().is_empty());
	// SAFETY: The player came from `Box::into_raw`, and is not used again.
	unsafe { drop(Box::from_raw(player)) };
}

/// The datamaps of a `class` deriving from `CBaseEntity`, whose own map has
/// `life_state` among its fields if given.
fn maps(class: &'static CStr, life_state: Option<sys::typedescription_t>) -> *mut sys::datamap_t {
	let mut fields = Vec::from(base_entity_fields());

	fields.extend(life_state);

	let base = data_map(c"CBaseEntity", fields, null_mut());

	data_map(class, vec![], base)
}

/// `CTFPlayer::PlayScene`, which records the scene, and returns the length
/// [`LENGTH`] holds.
unsafe extern "C" fn play_scene(
	player: *mut sys::CTFPlayer,
	scene: *const c_char,
	delay: f32,
	response: *mut sys::AI_Response,
	filter: *mut sys::IRecipientFilter,
) -> f32 {
	assert!(response.is_null());
	assert!(
		filter.is_null(),
		"a filter would be cast to CRecipientFilter"
	);

	// SAFETY: The speaker passes a NUL-terminated scene path.
	let scene = unsafe { CStr::from_ptr(scene) }.to_owned();

	PLAYED.with_borrow_mut(|played| played.push((player.cast(), scene, delay)));
	LENGTH.get()
}

/// The adapter of `CBaseFlex::PlayScene`, which records the scene, and returns
/// the length [`LENGTH`] holds, unless [`REJECT`] is set.
unsafe extern "C" fn play_scene_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 2);

	if REJECT.get() {
		return false;
	}

	// SAFETY: The binding declares a string and a float parameter, so the
	// caller passes two variants of those types, the string NUL-terminated.
	let (scene, delay) = unsafe {
		(
			CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString).to_owned(),
			(*arguments.add(1)).__bindgen_anon_1.m_float,
		)
	};

	PLAYED.with_borrow_mut(|played| played.push((object, scene, delay)));
	// SAFETY: The binding returns a float, so the caller passes a writable
	// result.
	unsafe { result.write(binding::float(LENGTH.get())) };
	true
}

#[test]
fn scripted_scenes_use_the_checked_cbaseflex_member() {
	let mut parameters = [STRING, binding::FLOAT];
	let mut bindings = [member_binding(
		c"PlayScene",
		binding::FLOAT,
		&mut parameters,
		Some(play_scene_adapter),
	)];
	let mut flex = class_description(c"CBaseFlex", &mut bindings, null_mut());

	// Changed below only through the pointer `call` reads it by.
	let binding = flex.m_FunctionBindings.m_Memory.m_pMemory;
	let mut player_description = class_description(c"CTFPlayer", &mut [], &raw mut flex);

	// The vtable slot `play_scene` would call fails the test if reached.
	let mut vtable = vec![std::ptr::null(); PLAY_SCENE + 1];

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = get_datamap as *const ();
	vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();
	vtable[PLAY_SCENE] = sdk_raw::test_support::unexpected_call as *const ();

	let player = Box::into_raw(FakePlayer::new(&vtable));
	// SAFETY: The fake player stays allocated until it is dropped at the end of
	// the test, after the last use of the handle, and its vtable answers what
	// the speaker calls.
	let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
	let scope = ();
	let server = mock_server(&scope);

	set_datamap(maps(c"CTFPlayer", Some(life_state_field())));
	set_script_description(&raw mut player_description);
	PLAYED.take();

	let speaker = Speaker::new(server, entity).unwrap();
	let line = ScenePath::voice_line(PlayerClass::Heavy, "cm_heavy_gamewon_01").unwrap();

	LENGTH.set(2.5);
	assert_eq!(
		speaker.play_scene_scripted(&line),
		Ok(Duration::from_secs_f32(2.5))
	);
	assert_eq!(
		PLAYED.take(),
		[(
			player.cast(),
			c"scenes/Player/Heavy/low/cm_heavy_gamewon_01.vcd".to_owned(),
			0.0
		)]
	);

	LENGTH.set(0.0);
	assert_eq!(
		speaker.play_scene_scripted(&line),
		Err(VoiceError::MissingScene)
	);

	REJECT.set(true);
	assert_eq!(
		speaker.play_scene_scripted(&line),
		Err(VoiceError::Rejected)
	);
	REJECT.set(false);

	// SAFETY: The fake player is allocated, and no reference to it is live.
	unsafe { (&raw mut (*player).life_state).write(LIFE_DYING) };
	assert_eq!(
		speaker.play_scene_scripted(&line),
		Err(VoiceError::NotAlive)
	);
	// SAFETY: As above.
	unsafe { (&raw mut (*player).life_state).write(LIFE_ALIVE) };

	// A member whose signature differs is refused before it is called.
	// SAFETY: `binding` points to the binding in `bindings`, which is alive,
	// and which the speaker only reads through the same pointer.
	unsafe { (*binding).m_desc.m_ReturnType = binding::VOID };
	assert_eq!(
		speaker.play_scene_scripted(&line),
		Err(VoiceError::UnsupportedMethod)
	);
	set_script_description(null_mut());
	assert_eq!(
		speaker.play_scene_scripted(&line),
		Err(VoiceError::UnsupportedMethod)
	);
	PLAYED.take();
	// SAFETY: The player came from `Box::into_raw`, and is not used again.
	unsafe { drop(Box::from_raw(player)) };
}

#[test]
fn speakers_require_tf_players_with_a_life_state() {
	let mut vtable = vec![std::ptr::null(); PLAY_SCENE + 1];

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = get_datamap as *const ();

	let player = Box::into_raw(FakePlayer::new(&vtable));
	// SAFETY: The fake player stays allocated until it is dropped at the end of
	// the test, after the last use of the handle, and its vtable answers the
	// datamap lookups the speaker makes.
	let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
	let scope = ();
	let server = mock_server(&scope);

	set_datamap(maps(c"CTFPlayer", Some(life_state_field())));
	assert!(Speaker::new(server, entity).is_ok());

	// TF2's bots derive from `CTFPlayer`, and speak as players do.
	set_datamap(data_map(
		c"CTFBot",
		vec![],
		maps(c"CTFPlayer", Some(life_state_field())),
	));
	assert!(Speaker::new(server, entity).is_ok());

	set_datamap(maps(c"CBaseCombatCharacter", Some(life_state_field())));
	assert!(matches!(
		Speaker::new(server, entity),
		Err(VoiceError::NotTfPlayer)
	));

	set_datamap(maps(c"CTFPlayer", None));
	assert!(matches!(
		Speaker::new(server, entity),
		Err(VoiceError::UnsupportedLayout)
	));

	let mut wide = life_state_field();

	wide.fieldSizeInBytes = 4;

	let mut array = life_state_field();

	array.fieldSize = 2;

	let mut integer = life_state_field();

	integer.fieldType = sys::_fieldtypes_FIELD_INTEGER;

	let mut far = life_state_field();

	far.fieldOffset[0] = BASE_ENTITY_FIELD_OFFSET_LIMIT as c_int;

	for life_state in [wide, array, integer, far] {
		set_datamap(maps(c"CTFPlayer", Some(life_state)));
		assert!(matches!(
			Speaker::new(server, entity),
			Err(VoiceError::UnsupportedLayout)
		));
	}

	// SAFETY: The player came from `Box::into_raw`, and is not used again.
	unsafe { drop(Box::from_raw(player)) };
}
