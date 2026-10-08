//! Tests of `crate::tag_hooks`: a post hook of `GetTaggedConVarList` on a mock
//! `IServerGameTags`, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::tf2_binding;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

thread_local! {
	/// What ran during the calls since the last [`list`], in order, with the
	/// list each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// The mock `IServerGameTags` the game server's factory exports.
	static GAME_TAGS: Cell<*mut c_void> = const { Cell::new(ptr::null_mut()) };
}

#[test]
fn a_paused_plugin_leaves_the_list_to_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let tags = game_tags(&scope);

		api.hook_tagged_convars(tags, binding(), on_tags).unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(list(harness, tags, 0x40), [("game", 0x40)]);

		harness.set_status(true, false, harness.generation);
		assert_eq!(
			list(harness, tags, 0x40),
			[("game", 0x40), ("callback", 0x40)]
		);
	});
}

/// A binding to a game server exporting only the mock of [`game_tags`].
fn binding() -> ServerBinding {
	tf2_binding(game_server_factory)
}

/// The game's `GetTaggedConVarList`, which notes that it ran.
unsafe extern "C" fn game_list(
	_this: *mut IServerGameTags,
	list: *mut source_sdk_2013::sys::KeyValues,
) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", list.addr())));
}

unsafe extern "C" fn game_server_factory(
	name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	// SAFETY: Factories are called with NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) };

	if name == ServerGameTags::VERSION {
		GAME_TAGS.get()
	} else {
		ptr::null_mut()
	}
}

/// A new mock of the game's `IServerGameTags`, which the game server's
/// factory exports from then on.
fn game_tags(scope: &()) -> ServerGameTags<'_> {
	let vtable = Box::leak(Box::new(
		source_sdk_2013::raw::interfaces::server_game_tags::IServerGameTagsVtable {
			get_tagged_con_var_list: game_list,
		},
	));

	let object = Box::leak(Box::new(IServerGameTags {
		vtable_: &raw const *vtable,
	}));

	GAME_TAGS.set(ptr::from_mut(object).cast());

	// SAFETY: The tests leak the mock the factory exports, and only turn the
	// binding into servers on the thread running their hooks, within the
	// test's call.
	let server = unsafe { binding().server(scope) };

	server
		.server_game_tags()
		.expect("the factory exports the mock")
}

/// Calls the mock's hooked `GetTaggedConVarList` with the list at `list`,
/// which no one reads, and returns what ran.
fn list(harness: &Harness, tags: ServerGameTags<'_>, list: usize) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<GetTaggedConVarList>(
		tags.as_ptr(),
		GET_TAGGED_CON_VAR_LIST_SLOT,
		(ptr::without_provenance_mut(list),),
	);
	CALLS.take()
}

fn on_tags(_server: Server<'_>, tags: TaggedConVars<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", tags.as_ptr().addr())));
}

#[test]
fn the_callback_sees_the_list_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let tags = game_tags(&scope);

		api.hook_tagged_convars(tags, binding(), on_tags).unwrap();

		// The callback is installed once.
		assert_eq!(
			api.hook_tagged_convars(tags, binding(), on_tags),
			Err(HookError::AlreadyInstalled)
		);

		assert_eq!(
			list(harness, tags, 0x40),
			[("game", 0x40), ("callback", 0x40)]
		);

		// A null list reaches the game only.
		assert_eq!(list(harness, tags, 0), [("game", 0)]);
	});
}
