//! Tests of `crate::server_hooks`: `GameFrame`, `Think` and
//! `SetServerHibernation` hooks on a mock `IServerGameDLL`, through the mock
//! SourceHook and KHook.

use super::*;
use crate::api::MetamodVersion;
use crate::sys::sourcehook::MetaRes;
use crate::test_support::foreign::{ForeignDelegate, khook_superseding};
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::tf2_binding;
use source_sdk_2013::sys;
use std::cell::RefCell;

/// The size of the mock `IServerGameDLL`'s vtable, every slot of which
/// holds [`game_frame`]: up to `SetServerHibernation`, the last hooked.
const SLOTS: usize = SET_SERVER_HIBERNATION_SLOT + 1;

thread_local! {
	/// What ran during the frames since the last [`run_frame`], in order.
	static FRAMES: RefCell<Vec<(&'static str, bool)>> = const { RefCell::new(Vec::new()) };

	/// The mock `IServerGameDLL` the game server's factory exports.
	static GAME_DLL: Cell<*mut c_void> = const { Cell::new(ptr::null_mut()) };
}

fn after_frame(_server: Server<'_>, simulating: bool) {
	FRAMES.with_borrow_mut(|frames| frames.push(("after", simulating)));
}

fn after_hibernation(_server: Server<'_>, hibernating: bool) {
	FRAMES.with_borrow_mut(|frames| frames.push(("hibernation", hibernating)));
}

fn before_frame(_server: Server<'_>, simulating: bool) {
	FRAMES.with_borrow_mut(|frames| frames.push(("before", simulating)));
}

fn before_think(_server: Server<'_>, final_tick: bool) {
	FRAMES.with_borrow_mut(|frames| frames.push(("think", final_tick)));
}

/// A binding to a game server exporting only the mock of [`game_dll`].
fn binding() -> ServerBinding {
	tf2_binding(game_server_factory)
}

#[test]
fn each_game_dll_hook_installs_once() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let game_dll = game_dll(&scope);

		api.hook_game_frame(game_dll, binding(), before_frame)
			.unwrap();
		api.hook_server_think(game_dll, binding(), before_think)
			.unwrap();
		api.hook_server_hibernation(game_dll, binding(), after_hibernation)
			.unwrap();

		assert!(matches!(
			api.hook_server_think(game_dll, binding(), before_think),
			Err(HookError::AlreadyInstalled)
		));

		assert!(matches!(
			api.hook_server_hibernation(game_dll, binding(), after_hibernation),
			Err(HookError::AlreadyInstalled)
		));

		assert_eq!(
			run(harness, game_dll, THINK, true),
			[("think", true), ("game", true)]
		);
	});
}

/// A new mock of the game's `IServerGameDLL`, which the game server's
/// factory exports from then on.
fn game_dll(scope: &()) -> ServerGameDll<'_> {
	let vtable = Vec::leak(vec![game_frame as GameFrame as *mut c_void; SLOTS]);

	let object = Box::leak(Box::new(sys::IServerGameDLL {
		vtable_: vtable.as_mut_ptr().cast(),
	}));

	FRAMES.take();
	GAME_DLL.set(ptr::from_mut(object).cast());

	// SAFETY: The tests leak the mock the factory exports, and only turn the
	// binding into servers on the thread running their hooks, within the
	// test's call.
	let server = unsafe { binding().server(scope) };

	server
		.server_game_dll()
		.expect("the factory exports the mock")
}

unsafe extern "C" fn game_frame(_this: *mut sys::IServerGameDLL, simulating: bool) {
	FRAMES.with_borrow_mut(|frames| frames.push(("game", simulating)));
}

#[test]
fn game_frame_hooks_run_around_the_frame() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let game_dll = game_dll(&scope);

		api.hook_game_frame(game_dll, binding(), before_frame)
			.unwrap();
		api.hook_game_frame_post(game_dll, binding(), after_frame)
			.unwrap();

		for simulating in [true, false] {
			assert_eq!(
				run_frame(harness, game_dll, simulating),
				[
					("before", simulating),
					("game", simulating),
					("after", simulating)
				]
			);
		}
	});
}

#[test]
fn game_frame_post_hooks_run_after_superseded_frames() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let game_dll = game_dll(&scope);

		api.hook_game_frame_post(game_dll, binding(), after_frame)
			.unwrap();

		// SAFETY: The mock starts with its vtable.
		let vtable = unsafe { game_dll.as_ptr().cast::<*mut *mut c_void>().read() };

		let foreign =
			ForeignDelegate::new::<bool>(harness.sourcehook_ptr(), MetaRes::SUPERCEDE, ());

		// Another plugin's hook, added after this one's, which skips every
		// frame.
		match api.version() {
			MetamodVersion::Stable1226 => harness.sourcehook.add_foreign(
				// SAFETY: The vtable has the slot.
				unsafe { vtable.add(GAME_FRAME.index()) },
				foreign.ptr(),
			),

			MetamodVersion::Dev1469 => harness.khook.add_foreign(
				vtable,
				GAME_FRAME.index(),
				khook_superseding::<sys::IServerGameDLL, bool, ()>(&()),
			),
		}

		// The game's frame is skipped, but this plugin's post hook still runs.
		assert_eq!(run_frame(harness, game_dll, true), [("after", true)]);
	});
}

unsafe extern "C" fn game_server_factory(
	name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	// SAFETY: Factories are called with NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) };

	if name == ServerGameDll::VERSION {
		GAME_DLL.get()
	} else {
		ptr::null_mut()
	}
}

#[test]
fn hibernation_hooks_run_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let game_dll = game_dll(&scope);

		api.hook_server_hibernation(game_dll, binding(), after_hibernation)
			.unwrap();

		for hibernating in [true, false] {
			assert_eq!(
				run(harness, game_dll, SET_SERVER_HIBERNATION, hibernating),
				[("game", hibernating), ("hibernation", hibernating)]
			);
		}
	});
}

/// Calls the mock's hooked `function`, and returns what ran during the call.
fn run(
	harness: &Harness,
	game_dll: ServerGameDll<'_>,
	function: VirtualFunction<GameFrame>,
	argument: bool,
) -> Vec<(&'static str, bool)> {
	harness.call::<GameFrame>(game_dll.as_ptr(), function.index(), (argument,));
	FRAMES.take()
}

/// Runs a frame through the mock's hooked `GameFrame`, and returns what ran
/// during it.
fn run_frame(
	harness: &Harness,
	game_dll: ServerGameDll<'_>,
	simulating: bool,
) -> Vec<(&'static str, bool)> {
	run(harness, game_dll, GAME_FRAME, simulating)
}

#[test]
fn think_hooks_run_before_think() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let game_dll = game_dll(&scope);

		api.hook_server_think(game_dll, binding(), before_think)
			.unwrap();

		for final_tick in [true, false] {
			assert_eq!(
				run(harness, game_dll, THINK, final_tick),
				[("think", final_tick), ("game", final_tick)]
			);
		}

		// Frames are not thinks.
		assert_eq!(run_frame(harness, game_dll, true), [("game", true)]);
	});
}
