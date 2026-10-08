//! Tests of the players in the game, found through the engine's clients and
//! edicts, and of kicking them.

use super::*;
use crate::server::{Game, Module};
use crate::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use crate::test_support::entities::MockEntity;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int};
use std::ptr::{NonNull, null_mut};

/// A client, as the mock server's slots hold them.
#[repr(C)]
struct ClientObject {
	interface: sys::IClient,
	slot: c_int,
	user_id: c_int,
	connected: bool,
	fake: bool,
}

/// What an edict's `m_pUnk` points to: the entity in the slot.
#[repr(C)]
struct UnknownObject {
	interface: sys::IServerUnknown,
	entity: *mut sys::CBaseEntity,
}

thread_local! {
	/// The mock server's clients, by slot.
	static CLIENTS: RefCell<Vec<*mut ClientObject>> = const { RefCell::new(Vec::new()) };

	/// The commands `ServerCommand` queued, in order.
	static COMMANDS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };

	/// The server `GetIServer` returns.
	static SERVER: Cell<*mut sys::IServer> = const { Cell::new(null_mut()) };
}

/// The client in `slot`, as the wrappers see it.
fn client(slot: usize) -> GameClient<'static> {
	let raw = CLIENTS.with_borrow(|clients| clients[slot]);

	// SAFETY: The clients are leaked, and their vtable answers what the
	// wrappers call.
	unsafe { GameClient::from_raw(NonNull::new(raw.cast()).unwrap()) }
}

/// `IClient::IsConnected`.
unsafe extern "C" fn client_connected(this: *const sys::IClient) -> bool {
	// SAFETY: Every client the mock server returns is a `ClientObject`.
	unsafe { (*this.cast::<ClientObject>()).connected }
}

/// `IClient::IsFakeClient`.
unsafe extern "C" fn client_fake(this: *const sys::IClient) -> bool {
	// SAFETY: As for `client_connected`.
	unsafe { (*this.cast::<ClientObject>()).fake }
}

/// `IClient::GetPlayerSlot`.
unsafe extern "C" fn client_slot(this: *const sys::IClient) -> c_int {
	// SAFETY: As for `client_connected`.
	unsafe { (*this.cast::<ClientObject>()).slot }
}

/// `IClient::GetUserID`.
unsafe extern "C" fn client_user_id(this: *const sys::IClient) -> c_int {
	// SAFETY: As for `client_connected`.
	unsafe { (*this.cast::<ClientObject>()).user_id }
}

/// `IServer::GetClient`.
unsafe extern "C" fn get_client(_: *mut sys::IServer, slot: c_int) -> *mut sys::IClient {
	CLIENTS.with_borrow(|clients| clients[usize::try_from(slot).unwrap()].cast())
}

/// `IServer::GetClientCount`.
unsafe extern "C" fn get_client_count(_: *const sys::IServer) -> c_int {
	CLIENTS.with_borrow(|clients| c_int::try_from(clients.len()).unwrap())
}

/// `IVEngineServer::GetIServer`, which returns [`SERVER`].
unsafe extern "C" fn get_iserver(_: *mut sys::IVEngineServer) -> *mut sys::IServer {
	SERVER.get()
}

#[test]
fn kicks_are_queued_by_user_id_with_their_reason() {
	mock_engine(&[(10, true, false), (0, false, false)], &[]);

	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();

	client(0).kick(engine, c"").unwrap();
	client(0).kick(engine, c"Too slow, \xC3\xA9").unwrap();

	assert_eq!(
		COMMANDS.take(),
		[
			c"kickid 10\n".to_owned(),
			c"kickid 10 Too slow, \xC3\xA9\n".to_owned(),
		]
	);

	for reason in [c"one; quit", c"\"quoted\"", c"two\nlines", c"\x7f"] {
		assert_eq!(
			client(0).kick(engine, reason),
			Err(KickError::InvalidReason),
			"{reason:?}"
		);
	}

	assert_eq!(client(1).kick(engine, c""), Err(KickError::NoPlayer));
	assert!(COMMANDS.take().is_empty());
}

/// Exports an engine whose server has a client in each slot of `clients`,
/// given as its user ID, whether it is connected, and whether it is a bot,
/// and whose edicts hold `entities`, by index, and whose `ServerCommand`
/// records each command.
fn mock_engine(clients: &[(c_int, bool, bool)], entities: &[Option<*mut sys::CBaseEntity>]) {
	// SAFETY: The vtables hold only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and each patch only writes slots of
	// the vtable being built.
	let (client_vtable, server_vtable, unknown_vtable, engine_vtable) = unsafe {
		(
			mock_vtable::<sys::IClient__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IClient_GetPlayerSlot).write(client_slot);
				(&raw mut (*vtable).IClient_GetUserID).write(client_user_id);
				(&raw mut (*vtable).IClient_IsConnected).write(client_connected);
				(&raw mut (*vtable).IClient_IsFakeClient).write(client_fake);
			}),
			mock_vtable::<sys::IServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IServer_GetClient).write(get_client);
				(&raw mut (*vtable).IServer_GetClientCount).write(get_client_count);
			}),
			mock_vtable::<sys::IServerUnknown__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerUnknown_GetBaseEntity).write(unknown_entity);
				},
			),
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_GetIServer).write(get_iserver);
					(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
					(&raw mut (*vtable).IVEngineServer_ServerCommand).write(server_command);
				},
			),
		)
	};

	let client_vtable = Box::leak(client_vtable);
	let unknown_vtable = Box::leak(unknown_vtable);

	let clients = clients
		.iter()
		.zip(0..)
		.map(|(&(user_id, connected, fake), slot)| {
			leak(ClientObject {
				interface: sys::IClient {
					vtable_: &raw const *client_vtable,
				},
				slot,
				user_id,
				connected,
				fake,
			})
		})
		.collect();

	let mut table = edict_table(entities.len(), |_| false);

	for (edict, &entity) in table.iter_mut().zip(entities) {
		if let Some(entity) = entity {
			edict._base.m_pUnk = leak(UnknownObject {
				interface: sys::IServerUnknown {
					vtable_: &raw const *unknown_vtable,
				},
				entity,
			})
			.cast();
		}
	}

	let table = Box::leak(table);

	serve_edicts(table.as_mut_ptr(), table.len());
	CLIENTS.set(clients);
	COMMANDS.take();
	SERVER.set(leak(sys::IServer {
		vtable_: Box::leak(server_vtable),
	}));

	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer {
			vtable_: Box::leak(engine_vtable),
		}),
	);
}

#[test]
fn players_are_the_connected_clients_with_entities() {
	let mut human = MockEntity::new(1);
	let mut bot = MockEntity::new(3);

	// Slot 1 is empty, and the player in slot 3 has no entity yet.
	mock_engine(
		&[
			(10, true, false),
			(0, false, false),
			(12, true, true),
			(13, true, false),
		],
		&[None, Some(human.as_ptr()), None, Some(bot.as_ptr()), None],
	);

	let scope = ();
	let server = mock_server(&scope);
	let players = server.players().unwrap().collect::<Vec<_>>();

	assert_eq!(players.len(), 2);
	assert_eq!(
		players
			.iter()
			.map(|player| (
				player.client.slot(),
				player.edict.index(),
				player.entity.as_ptr(),
				player.is_human(),
				player.user_id().map(UserId::to_raw),
			))
			.collect::<Vec<_>>(),
		[
			(0, 1, human.as_ptr(), true, Some(10)),
			(2, 3, bot.as_ptr(), false, Some(12)),
		]
	);

	// Without a game server, there are no players.
	SERVER.set(null_mut());
	assert_eq!(server.players().unwrap().count(), 0);

	let scope = ();

	assert!(null_server(Game::TeamFortress2, &scope).players().is_err());
}

/// `IVEngineServer::ServerCommand`, which records the command.
unsafe extern "C" fn server_command(_: *mut sys::IVEngineServer, command: *const c_char) {
	// SAFETY: The wrapper passes a NUL-terminated command.
	let command = unsafe { CStr::from_ptr(command) }.to_owned();

	COMMANDS.with_borrow_mut(|commands| commands.push(command));
}

/// `IServerUnknown::GetBaseEntity`.
unsafe extern "C" fn unknown_entity(this: *mut sys::IServerUnknown) -> *mut sys::CBaseEntity {
	// SAFETY: Every edict the mock engine serves points to an
	// `UnknownObject`.
	unsafe { (*this.cast::<UnknownObject>()).entity }
}
