//! Tests of `crate::tf2::overlays`: a fake TF2 player's native overlay methods.

use super::*;
use crate::test_support::entities::{MOCK_EFLAGS_OFFSET, base_entity_fields};
use crate::test_support::leak;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding,
};

use sdk_raw::test_support::entities::data_map;
use sdk_raw::tf2::script_binding::{INT, STRING, SV_FREE};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null, null_mut};

/// The function [`get_overlay`] implements.
const GET: isize = 1;

/// The function [`set_overlay`] implements.
const SET: isize = 0;

/// A TF2 player with an overlay and its two native methods.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	description: *mut sys::ScriptClassDesc_t,
	padding: usize,
	/// `m_iEFlags`, at [`MOCK_EFLAGS_OFFSET`].
	flags: Cell<c_int>,
	/// `m_Local.m_szScriptOverlayMaterial`.
	overlay: Cell<[c_char; MAX_MATERIAL_LEN + 1]>,
	/// What [`get_overlay`] returns.
	get_accepts: Cell<bool>,
	/// What [`set_overlay`] returns. It changes the overlay only while this is
	/// true.
	set_accepts: Cell<bool>,
	/// The variant flags [`get_overlay`] returns.
	result_flags: Cell<u16>,
	/// Whether [`get_overlay`] returns null instead of the overlay.
	returns_null: Cell<bool>,
	gets: Cell<usize>,
	sets: Cell<usize>,
}

/// The mock player, and the pointers through which tests change its native
/// methods.
struct Mock {
	object: NonNull<FakeEntity>,
	/// The setter's binding, followed by the getter's.
	bindings: *mut sys::ScriptFunctionBinding_t,
	/// The setter's parameter types.
	parameters: *mut sys::ScriptDataType_t,
}

impl Mock {
	fn state(&self) -> &FakeEntity {
		// SAFETY: The fake entity is leaked, and only ever shared: what changes
		// of it is in cells.
		unsafe { self.object.as_ref() }
	}
}

#[test]
fn clear_if_reports_a_matching_overlay_failing_to_clear() {
	let scope = ();
	let mock = mock_player(true);
	// SAFETY: The fake player is leaked, and its vtable answers what the
	// overlay calls.
	let entity = unsafe { Entity::from_raw(mock.object.cast()) };
	let overlay = ScreenOverlay::new(mock_server(&scope), entity).unwrap();
	let jarate = || overlay.get().unwrap();

	overlay.set(&OverlayMaterial::JARATE).unwrap();

	// The getter matches, then the setter's adapter refuses the clear.
	mock.state().set_accepts.set(false);
	assert_eq!(
		overlay.clear_if(&OverlayMaterial::JARATE),
		Err(OverlayError::Rejected)
	);
	assert_eq!((mock.state().sets.get(), mock.state().gets.get()), (2, 1));
	assert_eq!(jarate().as_deref(), Some(c"effects/jarate_overlay"));
	mock.state().set_accepts.set(true);

	// The getter matches, then the setter's signature differs, so the clear
	// never reaches the adapter.
	// SAFETY: `parameters` points to the setter's leaked parameter type, which
	// the wrapper only reads through the same pointer.
	unsafe { mock.parameters.write(INT) };
	assert_eq!(
		overlay.clear_if(&OverlayMaterial::JARATE),
		Err(OverlayError::UnsupportedMethod)
	);
	assert_eq!((mock.state().sets.get(), mock.state().gets.get()), (2, 3));
	assert_eq!(jarate().as_deref(), Some(c"effects/jarate_overlay"));
}

#[test]
fn deleted_players_and_mismatched_methods_are_refused() {
	let scope = ();
	let mock = mock_player(true);
	// SAFETY: The fake player is leaked, and its vtable answers what the
	// overlay calls.
	let entity = unsafe { Entity::from_raw(mock.object.cast()) };
	let overlay = ScreenOverlay::new(mock_server(&scope), entity).unwrap();

	mock.state().flags.set(1);

	for result in [
		overlay.set(&OverlayMaterial::JARATE).err(),
		overlay.clear().err(),
		overlay.get().err(),
		overlay.clear_if(&OverlayMaterial::JARATE).err(),
	] {
		assert_eq!(result, Some(OverlayError::MarkedForDeletion));
	}

	assert_eq!((mock.state().sets.get(), mock.state().gets.get()), (0, 0));
	mock.state().flags.set(0);

	// The adapters' failures, after they ran.
	mock.state().set_accepts.set(false);
	assert_eq!(
		overlay.set(&OverlayMaterial::JARATE),
		Err(OverlayError::Rejected)
	);
	mock.state().get_accepts.set(false);
	assert_eq!(overlay.get(), Err(OverlayError::Rejected));
	assert_eq!((mock.state().sets.get(), mock.state().gets.get()), (1, 1));
	mock.state().get_accepts.set(true);
	mock.state().set_accepts.set(true);

	// A string the game would have to free.
	mock.state().result_flags.set(SV_FREE);
	assert_eq!(overlay.get(), Err(OverlayError::UnsupportedMethod));
	mock.state().result_flags.set(0);

	mock.state().returns_null.set(true);
	assert_eq!(overlay.get(), Ok(None));
	mock.state().returns_null.set(false);

	// SAFETY: `parameters` points to the setter's leaked parameter type, which
	// the wrapper only reads through the same pointer.
	unsafe { mock.parameters.write(INT) };
	assert_eq!(
		overlay.set(&OverlayMaterial::JARATE),
		Err(OverlayError::UnsupportedMethod)
	);
	// SAFETY: As above.
	unsafe { mock.parameters.write(STRING) };

	// SAFETY: `bindings` points to the two leaked bindings, the getter's
	// second, which the wrapper only reads through the same pointer.
	unsafe { (*mock.bindings.add(1)).m_desc.m_pszScriptName = c"Other".as_ptr() };
	assert_eq!(overlay.get(), Err(OverlayError::UnsupportedMethod));
	assert_eq!(
		overlay.clear_if(&OverlayMaterial::JARATE),
		Err(OverlayError::UnsupportedMethod)
	);
	assert_eq!((mock.state().sets.get(), mock.state().gets.get()), (1, 3));
}

/// `CBaseEntity::GetScriptDesc`, which returns the fake entity's descriptor.
unsafe extern "C" fn description(entity: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
	// SAFETY: Only fake entities have this vtable, and they are leaked.
	unsafe { (*entity.cast::<FakeEntity>()).description }
}

/// `const char *CBasePlayer::GetScriptOverlayMaterial()`, returning the
/// overlay without a copy, like the SDK's member adapter.
unsafe extern "C" fn get_overlay(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	_: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(function.val_0, GET);
	assert_eq!(count, 0);
	assert!(!result.is_null());

	// SAFETY: The binding is only declared by fake entities' descriptors, so
	// the object is a fake entity, which is leaked and only ever shared.
	let object = unsafe { &*object.cast::<FakeEntity>() };
	let mut value = binding::string(c"");

	object.gets.set(object.gets.get() + 1);
	value.__bindgen_anon_1.m_pszString = if object.returns_null.get() {
		null()
	} else {
		object.overlay.as_ptr().cast()
	};
	value.m_flags = object.result_flags.get();
	// SAFETY: The binding returns a string, so the caller passes a writable
	// result.
	unsafe { result.write(value) };
	object.get_accepts.get()
}

/// `CBaseEntity::GetDataDescMap`, which returns the fake entity's datamap.
unsafe extern "C" fn map(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this vtable, and they are leaked.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

/// A leaked TF2 player, or another entity, whose script descriptors declare
/// both native overlay methods on `CBasePlayer`.
fn mock_player(player: bool) -> Mock {
	let parameters = Box::leak(Box::new([STRING]));
	let bindings = Box::leak(Box::new([
		member_binding(
			c"SetScriptOverlayMaterial",
			binding::VOID,
			parameters,
			Some(set_overlay),
		),
		member_binding(
			c"GetScriptOverlayMaterial",
			STRING,
			&mut [],
			Some(get_overlay),
		),
	]));

	bindings[0].m_pFunction.val_0 = SET;
	bindings[1].m_pFunction.val_0 = GET;

	let base = leak(class_description(c"CBasePlayer", bindings, null_mut()));

	// Changed later only through the pointers `call` reads them by, since
	// writing through the leaked references would invalidate those.
	// SAFETY: The descriptor is leaked, and nothing refers to it yet.
	let bindings = unsafe { (*base).m_FunctionBindings.m_Memory.m_pMemory };
	// SAFETY: The descriptor's first binding is the setter, which views the
	// leaked parameters.
	let parameters = unsafe { (*bindings).m_desc.m_Parameters.m_Memory.m_pMemory };
	let derived = leak(class_description(c"CTFPlayer", &mut [], base));

	let mut chain = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let classes: &[&'static CStr] = if player {
		&[c"CBasePlayer", c"CBaseMultiplayerPlayer", c"CTFPlayer"]
	} else {
		&[c"CBaseAnimating", c"CTFWearable"]
	};

	for &class in classes {
		chain = data_map(class, vec![], chain);
	}

	let vtable = Box::leak(Box::new([null::<()>(); SCRIPT_DESCRIPTION_SLOT + 1]));

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = map as *const ();
	vtable[SCRIPT_DESCRIPTION_SLOT] = description as *const ();

	let object = NonNull::from(Box::leak(Box::new(FakeEntity {
		vtable: vtable.as_ptr(),
		map: chain,
		description: derived,
		padding: 0,
		flags: Cell::new(0),
		overlay: Cell::new([0; MAX_MATERIAL_LEN + 1]),
		get_accepts: Cell::new(true),
		set_accepts: Cell::new(true),
		result_flags: Cell::new(0),
		returns_null: Cell::new(false),
		gets: Cell::new(0),
		sets: Cell::new(0),
	})));

	assert_eq!(offset_of!(FakeEntity, flags), MOCK_EFLAGS_OFFSET);

	Mock {
		object,
		bindings,
		parameters,
	}
}

#[test]
fn overlays_require_tf2_players() {
	let scope = ();
	let player = mock_player(true);
	// SAFETY: The fake entities are leaked, and their vtables answer the
	// datamap lookups the overlay makes.
	let player = unsafe { Entity::from_raw(player.object.cast()) };
	let wearable = mock_player(false);
	// SAFETY: As for the player.
	let wearable = unsafe { Entity::from_raw(wearable.object.cast()) };

	assert!(ScreenOverlay::new(mock_server(&scope), player).is_ok());
	assert_eq!(
		ScreenOverlay::new(mock_server(&scope), wearable).err(),
		Some(OverlayError::NotAPlayer)
	);
}

#[test]
fn overlays_round_trip_through_the_native_methods() {
	let scope = ();
	let mock = mock_player(true);
	// SAFETY: The fake player is leaked, and its vtable answers what the
	// overlay calls.
	let entity = unsafe { Entity::from_raw(mock.object.cast()) };
	let overlay = ScreenOverlay::new(mock_server(&scope), entity).unwrap();
	let custom = OverlayMaterial::new(c"example/overlay").unwrap();

	assert_eq!(overlay.get(), Ok(None));
	overlay.set(&OverlayMaterial::JARATE).unwrap();
	assert_eq!(
		overlay.get().unwrap().as_deref(),
		Some(c"effects/jarate_overlay")
	);
	overlay.set(&custom).unwrap();
	assert_eq!(overlay.get().unwrap().as_deref(), Some(c"example/overlay"));
	overlay.clear().unwrap();
	assert_eq!(overlay.get(), Ok(None));
	assert_eq!(mock.state().sets.get(), 3);

	// Another writer's overlay is left alone.
	overlay.set(&OverlayMaterial::JARATE).unwrap();
	assert_eq!(overlay.clear_if(&custom), Ok(false));
	assert_eq!(mock.state().sets.get(), 4);
	assert_eq!(
		overlay.get().unwrap().as_deref(),
		Some(c"effects/jarate_overlay")
	);

	// Names match ignoring ASCII case, as TF2 compares them.
	let upper = OverlayMaterial::new(c"EFFECTS/Jarate_Overlay").unwrap();

	assert_eq!(overlay.clear_if(&upper), Ok(true));
	assert_eq!(mock.state().sets.get(), 5);
	assert_eq!(overlay.get(), Ok(None));
	assert_eq!(overlay.clear_if(&upper), Ok(false));
	assert_eq!(mock.state().sets.get(), 5);
}

/// `void CBasePlayer::SetScriptOverlayMaterial(const char *)`, which copies at
/// most [`MAX_MATERIAL_LEN`] bytes, like `V_strncpy`. Like the SDK's adapter,
/// it fails before calling the method, so a refused call leaves the overlay
/// alone.
unsafe extern "C" fn set_overlay(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(function.val_0, SET);
	assert_eq!(count, 1);
	assert!(result.is_null());

	// SAFETY: The binding is only declared by fake entities' descriptors, so
	// the object is a fake entity, which is leaked and only ever shared.
	let object = unsafe { &*object.cast::<FakeEntity>() };
	// SAFETY: The binding declares one string parameter, so the caller passes
	// one string variant, NUL-terminated.
	let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };
	let mut overlay = [0; MAX_MATERIAL_LEN + 1];

	for (stored, &byte) in overlay.iter_mut().zip(name.to_bytes()) {
		*stored = byte as c_char;
	}

	object.sets.set(object.sets.get() + 1);

	if object.set_accepts.get() {
		object.overlay.set(overlay);
	}

	object.set_accepts.get()
}
