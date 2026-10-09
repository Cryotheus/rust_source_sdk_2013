//! Tests of `EngineReplay`: finding the engine's interface, and the slot its
//! recalculation of the server's tags calls.

use super::*;
use crate::server::Module;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::interfaces::engine_replay::IEngineReplayVtable;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;

thread_local! {
	/// The object of every call to `RecalculateTags`, in order.
	static RECALCULATED: RefCell<Vec<*mut IEngineReplay>> = const { RefCell::new(Vec::new()) };
}

/// `IEngineReplay::RecalculateTags`, which notes the call.
unsafe extern "C" fn recalculate_tags(this: *mut IEngineReplay) {
	RECALCULATED.with_borrow_mut(|calls| calls.push(this));
}

/// A leaked `IEngineReplay` whose every slot but `RecalculateTags` fails the
/// test when called.
fn mock_engine_replay() -> *mut IEngineReplay {
	// SAFETY: The vtable holds only pointer-sized slots, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<IEngineReplayVtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).recalculate_tags).write(recalculate_tags);
		})
	};

	leak(IEngineReplay {
		vtable_: Box::leak(vtable),
	})
}

#[test]
fn tags_are_recalculated_through_the_engines_interface() {
	let replay = mock_engine_replay();

	export(Module::Engine, VERSION, replay);

	let scope = ();
	let engine_replay = mock_server(&scope).engine_replay().unwrap();

	assert_eq!(engine_replay.as_ptr(), replay);

	engine_replay.recalculate_tags();
	engine_replay.recalculate_tags();

	assert_eq!(RECALCULATED.with_borrow(Clone::clone), [replay, replay]);
}

#[test]
fn a_game_server_module_does_not_export_it() {
	export(Module::GameServer, VERSION, mock_engine_replay());

	let scope = ();

	assert!(mock_server(&scope).engine_replay().is_err());
	assert!(
		null_server(crate::server::Game::TeamFortress2, &scope)
			.engine_replay()
			.is_err()
	);
	assert!(RECALCULATED.with_borrow(Vec::is_empty));
}
