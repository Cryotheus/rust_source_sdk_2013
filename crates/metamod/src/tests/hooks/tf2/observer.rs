//! Tests of `crate::hooks::tf2::observer`: post hooks of `SetObserverMode` on mock
//! player classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::interfaces::ServerTools;
use source_sdk_2013::raw::test_support::{mock_vtable, unexpected_call};

use source_sdk_2013::raw::tf2::observer::{
	GET_OBSERVER_MODE_SLOT, GetObserverModeFn, NUM_OBSERVER_MODES, OBS_MODE_CHASE,
};

use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::ffi::{CStr, c_char};
use std::mem::offset_of;
use std::ptr::null_mut;

thread_local! {
	/// The players the callback ran for since the last [`set_mode`].
	static CALLBACKS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// A player of a C++ class, as far as hooks know it, and the state the mock
/// game's observer methods read and write.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
	mode: c_int,
	/// Whether the player is on a team, which the game does not let roam.
	on_team: bool,
	/// Whether the match summary is showing, during which the game refuses
	/// every change.
	summary: bool,
	map: *mut sys::datamap_t,
	target: u32,
	force_chase: bool,
	is_player: bool,
}

impl Player {
	/// A player of a new class, whose vtable holds the mock game's
	/// `SetObserverMode` and `GetObserverMode`.
	fn of_new_class(on_team: bool) -> Box<Self> {
		let slots = Vec::leak(vec![
			null_mut::<c_void>();
			SET_OBSERVER_MODE_SLOT.max(GET_OBSERVER_MODE_SLOT) + 1
		]);

		slots[SET_OBSERVER_MODE_SLOT] = game_set_mode as SetObserverMode as *mut c_void;
		slots[GET_OBSERVER_MODE_SLOT] = game_get_mode as GetObserverModeFn as *mut c_void;
		slots[source_sdk_2013::raw::entities::GET_DATA_DESC_MAP_SLOT] = game_datamap as *mut c_void;
		slots[offset_of!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_IsPlayer)
			/ size_of::<usize>()] = game_is_player as *mut c_void;
		let mut target = source_sdk_2013::raw::test_support::entities::field(
			c"m_hObserverTarget",
			sys::_fieldtypes_FIELD_EHANDLE,
			offset_of!(Player, target),
		);
		target.fieldSize = 1;
		target.fieldSizeInBytes = size_of::<u32>() as c_int;
		let map = source_sdk_2013::raw::test_support::entities::data_map(
			c"CBasePlayer",
			vec![target],
			null_mut(),
		);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
			mode: OBS_MODE_CHASE,
			on_team,
			summary: false,
			map,
			target: u32::MAX,
			force_chase: false,
			is_player: true,
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `GetObserverMode`.
unsafe extern "C" fn game_get_mode(this: *mut sys::CBaseEntity) -> c_int {
	// SAFETY: Only mock players have this method in their vtables.
	unsafe { (*this.cast::<Player>()).mode }
}

/// The game's `SetObserverMode` outside PASS Time, as far as the hook relies
/// on it.
unsafe extern "C" fn game_set_mode(this: *mut sys::CBaseEntity, mut mode: c_int) -> bool {
	// SAFETY: As for `game_get_mode`.
	let player = unsafe { &mut *this.cast::<Player>() };

	if !(0..NUM_OBSERVER_MODES).contains(&mode) || player.summary {
		return false;
	}

	if mode == OBS_MODE_POI {
		mode = OBS_MODE_ROAMING;
	}

	if player.on_team && mode == OBS_MODE_ROAMING {
		mode = OBS_MODE_IN_EYE;
	}

	if mode == OBS_MODE_IN_EYE && player.force_chase {
		mode = OBS_MODE_CHASE;
	}

	player.mode = mode;
	true
}

/// The callback, which notes the player and lets them roam, as
/// `PlayerObserver::roam` would.
fn on_refused(_server: Server<'_>, player: Entity<'_>) {
	let player = player.as_ptr().cast::<Player>();

	CALLBACKS.with_borrow_mut(|callbacks| callbacks.push(player.addr()));

	// SAFETY: The hook passes the live mock player whose method just ran.
	unsafe { (*player).mode = OBS_MODE_ROAMING };
}

#[test]
fn only_roaming_the_game_turned_into_first_person_reaches_the_callback() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(true);
		let mut spectator = Player::of_new_class(false);
		let address = player.ptr().addr().get();

		for object in [player.ptr(), spectator.ptr()] {
			// SAFETY: The mock classes have both methods at their slots, and are
			// leaked.
			unsafe { api.install_roaming(object, tf2_binding(no_interfaces), on_refused) }.unwrap();
		}

		// A second hook of a class is refused, so that each change is seen
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_roaming(player.ptr(), tf2_binding(no_interfaces), on_refused) },
			Err(ObserverHookError::Hook(HookError::AlreadyInstalled))
		));

		// Roaming, and the point of interest outside PASS Time, turn into first
		// person for a player on a team, which the callback undoes.
		for mode in [OBS_MODE_ROAMING, OBS_MODE_POI] {
			assert_eq!(
				set_mode(harness, &mut player, mode),
				(true, OBS_MODE_ROAMING, vec![address])
			);
		}

		// Other modes, and spectators, are left to the game.
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_IN_EYE),
			(true, OBS_MODE_IN_EYE, vec![])
		);
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_CHASE),
			(true, OBS_MODE_CHASE, vec![])
		);
		assert_eq!(
			set_mode(harness, &mut spectator, OBS_MODE_ROAMING),
			(true, OBS_MODE_ROAMING, vec![])
		);

		// So is a change the game refused, even of a player following in first
		// person.
		player.mode = OBS_MODE_IN_EYE;
		player.summary = true;
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_ROAMING),
			(false, OBS_MODE_IN_EYE, vec![])
		);
	});
}

/// Calls `player`'s hooked `SetObserverMode` with `mode`, and returns what it
/// returned, the player's mode after it, and whom the callback ran for.
fn set_mode(harness: &Harness, player: &mut Player, mode: c_int) -> (bool, c_int, Vec<usize>) {
	CALLBACKS.take();

	let accepted =
		harness.call::<SetObserverMode>(player.ptr().as_ptr(), SET_OBSERVER_MODE_SLOT, (mode,));

	(accepted, player.mode, CALLBACKS.take())
}

thread_local! {
	static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
	static TOOLS: Cell<*mut sys::IServerTools> = const { Cell::new(null_mut()) };
}

unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	ENTITY_LIST.get()
}

unsafe extern "C" fn game_datamap(this: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only mock players have this method in their vtables.
	unsafe { (*this.cast::<Player>()).map }
}

unsafe extern "C" fn game_interfaces(name: *const c_char, _: *mut c_int) -> *mut c_void {
	// SAFETY: CreateInterface receives a valid nul-terminated interface name.
	if unsafe { CStr::from_ptr(name) } == ServerTools::VERSION {
		TOOLS.get().cast()
	} else {
		null_mut()
	}
}

unsafe extern "C" fn game_is_player(this: *mut sys::CBaseEntity) -> bool {
	// SAFETY: Only mock players and cameras have this method in their vtables.
	unsafe { (*this.cast::<Player>()).is_player }
}

#[test]
fn map_camera_chase_fallback_reaches_the_callback_but_player_and_stale_targets_do_not() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(true);
		let mut camera = Player::of_new_class(false);
		camera.is_player = false;
		serve_target(&mut camera);
		player.force_chase = true;
		player.target = 1 | 1 << 16;
		let address = player.ptr().addr().get();
		// SAFETY: The mock class has the methods at their generated slots and
		// its vtable is leaked; the target and exported list survive the calls.
		unsafe { api.install_roaming(player.ptr(), tf2_binding(game_interfaces), on_refused) }
			.unwrap();
		for mode in [OBS_MODE_ROAMING, OBS_MODE_POI] {
			assert_eq!(
				set_mode(harness, &mut player, mode),
				(true, OBS_MODE_ROAMING, vec![address])
			);
		}
		// A direct request to chase must not invoke the roaming policy.
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_CHASE),
			(true, OBS_MODE_CHASE, vec![])
		);
		camera.is_player = true;
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_ROAMING),
			(true, OBS_MODE_CHASE, vec![])
		);
		camera.is_player = false;
		for handle in [u32::MAX, 1 | 2 << 16] {
			player.target = handle;
			assert_eq!(
				set_mode(harness, &mut player, OBS_MODE_ROAMING),
				(true, OBS_MODE_CHASE, vec![])
			);
		}
		// A failed mode change is still ignored even with a current camera.
		player.target = 1 | 1 << 16;
		player.summary = true;
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_ROAMING),
			(false, OBS_MODE_CHASE, vec![])
		);
	});
}

/// Exports a current target at slot one, with serial one.
fn serve_target(target: &mut Player) {
	let list = Box::leak(Box::<sys::CGlobalEntityList>::new_zeroed()).as_mut_ptr();
	// SAFETY: The zeroed list is leaked and slot one lies within its array.
	// The target stays alive for every method invocation in the test.
	unsafe {
		let entry = (&raw mut (*list)._base.m_EntPtrArray)
			.cast::<sys::CEntInfo>()
			.add(1);
		(&raw mut (*entry).m_pEntity).write(target.ptr().as_ptr().cast());
		(&raw mut (*entry).m_SerialNumber).write(1);
	}
	ENTITY_LIST.set(list);
	// SAFETY: The vtable holds function pointers; every unconfigured slot
	// aborts, and the patch writes only the entity-list method.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
		})
	});
	TOOLS.set(Box::leak(Box::new(sys::IServerTools { vtable_: vtable })));
}
