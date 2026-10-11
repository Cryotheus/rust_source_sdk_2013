//! Tests of `crate::hooks::tf2::bot`: hooks before and after the `Dispatch` of a
//! mock `tf_bot_add`, whose runs put mock players in an edict table, through
//! the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::{PlayerInfoManager, ValveEngine};
use source_sdk_2013::raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::raw::tf2::script_binding::{INT, int};
use source_sdk_2013::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use source_sdk_2013::test_support::entities::MockEntity;
use source_sdk_2013::test_support::interfaces::cvar::{mock_command, mock_cvar};

use source_sdk_2013::test_support::interfaces::player_info_manager::{
	global_vars, serve_global_vars,
};

use source_sdk_2013::test_support::server::{export, mock_binding};

use source_sdk_2013::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use source_sdk_2013::tf2::bots::TF_BOT_TYPE;
use std::ffi::{c_int, c_void};
use std::mem::{offset_of, zeroed};
use std::ptr::null_mut;

/// The client limit of the mock server, whose edict table has one more slot,
/// for the world.
const MAX_CLIENTS: c_int = 6;

thread_local! {
	/// The entities whose `GetBotType` reports a bot.
	static BOTS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };

	/// The registry's first command, once [`serve`] made it on this thread,
	/// which links to the latest mock `tf_bot_add`.
	static HEAD: Cell<*mut sys::ConCommandBase> = const { Cell::new(null_mut()) };

	/// What ran during the runs since the last [`run`].
	static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };

	/// The players the next run of `tf_bot_add` puts in the server: their slot,
	/// entity and user ID.
	static JOINING: RefCell<Vec<(usize, *mut sys::CBaseEntity, c_int)>> = const { RefCell::new(Vec::new()) };

	/// The edict table [`serve`] made.
	static TABLE: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };

	/// The user ID of each slot's client, or -1, as the engine reports them.
	static USER_IDS: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };
}

/// What ran during a run of `tf_bot_add`.
#[derive(Debug, PartialEq)]
enum Call {
	/// The game's command ran.
	Game,

	/// The callback ran with the bots at these addresses.
	Added(Vec<usize>),
}

#[test]
fn added_bots_are_reported_after_the_command() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let command = serve();
		let server = mock_server(&scope);
		let cvar = server.cvar().unwrap();

		reset_players();

		// A human and a bot are in the server already.
		join(1, mock_player(false), 2);
		join(2, mock_player(true), 3);

		let hooks = api.hook_tf_bot_add(cvar, mock_binding(), on_added).unwrap();

		assert_eq!(
			api.hook_tf_bot_add(cvar, mock_binding(), on_added),
			Err(HookError::AlreadyInstalled)
		);

		// Two bots join during the run, and a human connects.
		let first = mock_player(true);
		let second = mock_player(true);

		JOINING.set(vec![
			(3, first, 7),
			(4, mock_player(false), 8),
			(6, second, 9),
		]);

		assert_eq!(
			run(harness, command),
			[Call::Game, Call::Added(vec![first.addr(), second.addr()])]
		);

		// A run that adds no bot calls nothing.
		JOINING.set(vec![(5, mock_player(false), 10)]);
		assert_eq!(run(harness, command), [Call::Game]);

		hooks.remove(api);
		JOINING.set(vec![(5, mock_player(true), 11)]);
		assert_eq!(run(harness, command), [Call::Game]);

		// Removed hooks can be replaced.
		let hooks = api.hook_tf_bot_add(cvar, mock_binding(), on_added).unwrap();

		JOINING.set(vec![(5, mock_player(true), 12)]);
		assert!(matches!(
			&run(harness, command)[..],
			[Call::Game, Call::Added(_)]
		));
		hooks.remove(api);
	});
}

/// `CTFPlayer::GetBotType`'s adapter, which reports [`TF_BOT_TYPE`] for the
/// entities [`BOTS`] lists, and 0 for any other.
unsafe extern "C" fn bot_type(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	_: *mut sys::ScriptVariant_t,
	_: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	let bot = BOTS.with_borrow(|bots| bots.contains(&object.addr()));

	// SAFETY: The binding returns an integer, so the caller passes a writable
	// result.
	unsafe { result.write(int(if bot { TF_BOT_TYPE } else { 0 })) };
	true
}

/// `ConCommand::Dispatch` of the game's `tf_bot_add`, which puts the
/// [`JOINING`] players in the server.
unsafe extern "C" fn game_dispatch(_: *mut sys::ConCommand, _: *const sys::CCommand) {
	CALLS.with_borrow_mut(|calls| calls.push(Call::Game));

	for (slot, entity, user_id) in JOINING.take() {
		join(slot, entity, user_id);
	}
}

/// `IServerUnknown::GetBaseEntity` of a mock player, which is its own base
/// entity.
unsafe extern "C" fn get_base_entity(this: *mut sys::IServerUnknown) -> *mut sys::CBaseEntity {
	this.cast()
}

/// `ConCommandBase::IsCommand` of the mock `tf_bot_add`.
unsafe extern "C" fn is_command(_: *const sys::ConCommand) -> bool {
	true
}

/// Puts `entity` in the server at `slot`, as a client with `user_id`.
fn join(slot: usize, entity: *mut sys::CBaseEntity, user_id: c_int) {
	// SAFETY: The table is leaked, and has a slot for each client.
	unsafe { (*TABLE.get().add(slot))._base.m_pUnk = entity.cast() };
	USER_IDS.with_borrow_mut(|ids| ids[slot] = user_id);
}

/// A leaked `tf_bot_add` the registry lists after `next`, whose `Dispatch` is
/// [`game_dispatch`].
fn mock_bot_add(next: *mut sys::ConCommandBase) -> *mut sys::ConCommand {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::ConCommand__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ConCommand_IsCommand).write(is_command);
			(&raw mut (*vtable).ConCommand_Dispatch).write(game_dispatch);
		})
	};

	// SAFETY: Zero is valid for every field of `ConCommand`.
	let mut command: sys::ConCommand = unsafe { zeroed() };

	command._base.vtable_ = (&raw const *Box::leak(vtable)).cast();
	command._base.m_pNext = next;
	command._base.m_bRegistered = true;
	command._base.m_pszName = COMMAND.as_ptr();
	command._base.m_pszHelpString = c"".as_ptr();

	Box::into_raw(Box::new(command))
}

/// An engine that serves the edict table [`serve`] made, and the user IDs of
/// [`USER_IDS`].
fn mock_engine() -> *mut sys::IVEngineServer {
	/// `IVEngineServer::GetPlayerUserId`.
	unsafe extern "C" fn user_id(_: *mut sys::IVEngineServer, edict: *const sys::edict_t) -> c_int {
		// SAFETY: The wrappers pass edicts of the table, which is leaked.
		let slot = unsafe { (*edict)._base.m_EdictIndex };

		USER_IDS.with_borrow(|ids| ids[slot as usize])
	}

	// SAFETY: As for the command's vtable in `mock_bot_add`.
	let vtable = unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
			(&raw mut (*vtable).IVEngineServer_GetPlayerUserId).write(user_id);
		})
	};

	Box::into_raw(Box::new(sys::IVEngineServer {
		vtable_: Box::leak(vtable),
	}))
}

/// A leaked player whose `GetBaseEntity` and `GetScriptDesc` answer, a bot if
/// `bot` is set.
fn mock_player(bot: bool) -> *mut sys::CBaseEntity {
	let pointer = MockEntity::new(1).as_ptr();
	let mut vtable = vec![unexpected_call as *const (); SCRIPT_DESCRIPTION_SLOT + 1];

	// The mock's own vtable answers every slot before the descriptor's, which
	// include the datamap's.
	// SAFETY: A mock entity starts with the pointer to its vtable, which has
	// slots up to TF2's `Teleport`, past `GetScriptDesc`.
	unsafe {
		let original = pointer.cast::<*const *const ()>().read();

		for (slot, entry) in vtable.iter_mut().enumerate().take(SCRIPT_DESCRIPTION_SLOT) {
			*entry = original.add(slot).read();
		}
	}

	let base_entity = offset_of!(
		sys::IServerUnknown__bindgen_vtable,
		IServerUnknown_GetBaseEntity
	);

	vtable[base_entity / size_of::<usize>()] = get_base_entity as *const ();
	vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();

	// SAFETY: A mock entity starts with the pointer to its vtable, and the new
	// vtable is leaked.
	unsafe {
		pointer
			.cast::<*const *const ()>()
			.write(vtable.leak().as_ptr())
	};

	if bot {
		BOTS.with_borrow_mut(|bots| bots.push(pointer.addr()));
	}

	pointer
}

/// A player info manager whose globals set the client limit.
fn mock_players() -> *mut sys::IPlayerInfoManager {
	// SAFETY: As for the command's vtable in `mock_bot_add`.
	let vtable = unsafe {
		mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
			},
		)
	};

	Box::into_raw(Box::new(sys::IPlayerInfoManager {
		vtable_: Box::leak(vtable),
	}))
}

/// A server whose factories export what [`serve`] did.
fn mock_server(scope: &()) -> Server<'_> {
	// SAFETY: The tests leak every interface they export, and turn the binding
	// into servers only on the thread running their hooks.
	unsafe { mock_binding().server(scope) }
}

fn on_added(_server: Server<'_>, bots: &[TfBot<'_>]) {
	let added = bots
		.iter()
		.map(|bot| bot.entity().as_ptr().addr())
		.collect();

	CALLS.with_borrow_mut(|calls| calls.push(Call::Added(added)));
}

#[test]
fn registries_without_the_command_are_refused() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let others = mock_command(c"tf_bot_kick", mock_command(c"tf_bot_quota", null_mut()));

		export(Module::Engine, Cvar::VERSION, mock_cvar(others, vec![]));

		let cvar = mock_server(&scope).cvar().unwrap();

		assert_eq!(
			api.hook_tf_bot_add(cvar, mock_binding(), on_added),
			Err(HookError::InvalidArgument)
		);
	});
}

/// Empties every player slot, and forgets the bots.
fn reset_players() {
	let len = MAX_CLIENTS as usize + 1;

	// SAFETY: The table is leaked, and has `len` slots.
	unsafe {
		for slot in 0..len {
			(*TABLE.get().add(slot))._base.m_pUnk = null_mut();
		}
	}

	USER_IDS.set(vec![-1; len]);
	BOTS.take();
	JOINING.take();
}

/// Runs the hooked `tf_bot_add`, and returns what ran.
fn run(harness: &Harness, command: *mut sys::ConCommand) -> Vec<Call> {
	// SAFETY: Zero is valid for every field of `CCommand`, which the mock
	// command does not read.
	let args: sys::CCommand = unsafe { zeroed() };

	CALLS.take();
	harness.call::<Dispatch>(command, DISPATCH_SLOT, (&raw const args,));
	CALLS.take()
}

/// Exports the mock engine, registry and player info manager on this thread,
/// the first time, and returns a new mock `tf_bot_add`, which the registry
/// lists from then on.
///
/// Each harness gets its own command, since SourceHook's mock leaves its
/// hooks in the vtable of the command it hooked.
fn serve() -> *mut sys::ConCommand {
	if HEAD.get().is_null() {
		let table = Box::leak(edict_table(MAX_CLIENTS as usize + 1, |_| false));
		let head = mock_command(c"tf_bot_kick", null_mut());

		serve_edicts(table.as_mut_ptr(), table.len());
		serve_global_vars(MAX_CLIENTS);
		TABLE.set(table.as_mut_ptr());
		HEAD.set(head);
		export(Module::Engine, Cvar::VERSION, mock_cvar(head, vec![]));
		export(Module::Engine, ValveEngine::VERSION, mock_engine());
		export(
			Module::GameServer,
			PlayerInfoManager::VERSION,
			mock_players(),
		);

		let parameters: &mut [sys::ScriptDataType_t] = &mut [];
		let bindings = Box::leak(Box::new([member_binding(
			c"GetBotType",
			INT,
			parameters,
			Some(bot_type),
		)]));

		set_script_description(Box::into_raw(Box::new(class_description(
			c"CTFPlayer",
			bindings,
			null_mut(),
		))));
	}

	let command = mock_bot_add(null_mut());

	// SAFETY: The head is leaked, and the registry reads its link only while
	// it is listed.
	unsafe { (*HEAD.get()).m_pNext = command.cast() };
	command
}
