//! A mock engine with clients, their channels, and a console variable
//! registry, for tests of leases through the engine.
//!
//! Every object the mock hands the engine wrappers is a box `MockEngine::new`
//! leaks, so it stays valid, at the same address, for the rest of the test
//! process. What tests change afterwards sits behind `Cell` or `RefCell`, so
//! shared references to the objects never alias a mutation.

use super::*;
use crate::bitbuf::RawBfWrite;
use crate::ffi::test_support::{mock_vtable, unexpected_call};
use crate::interfaces::ValveEngine;
use crate::net::MESSAGE_TYPE_BITS;
use crate::server::Module;
use crate::server::test_support::{export, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::ptr::{NonNull, null_mut};

#[repr(C)]
struct ChannelObject {
	interface: sys::INetChannel,
	loopback: bool,
	room: Cell<usize>,
	sent: RefCell<Vec<BitWriter>>,
}

#[repr(C)]
struct ClientObject {
	interface: sys::IClient,
	channel: *mut ChannelObject,
	spec: Cell<MockClient>,
}

#[repr(C)]
struct CvarObject {
	interface: sys::ICvar,
	head: *mut sys::ConCommandBase,
	vars: Vec<*mut sys::ConVar>,
}

/// A message a test decoded from what a channel was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decoded {
	/// `svc_GetCvarValue`: a cookie and a variable's name.
	Query(c_int, CString),

	/// `net_SetConVar`: names and values.
	SetConVar(Vec<(CString, CString)>),

	/// `net_StringCmd`: a command.
	StringCmd(CString),
}

#[repr(C)]
struct EngineObject {
	interface: sys::IVEngineServer,
	server: *mut ServerObject,
}

/// One player slot of a [`MockEngine`]. A slot whose user ID is 0 is empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MockClient {
	pub(crate) user_id: c_int,
	pub(crate) active: bool,
	pub(crate) fake: bool,
	pub(crate) hltv: bool,
	pub(crate) loopback: bool,
	/// Whether the client has no channel although it is a remote player.
	pub(crate) no_channel: bool,
}

impl MockClient {
	/// A remote player, fully in the game.
	pub(crate) const fn active(user_id: c_int) -> Self {
		Self {
			user_id,
			active: true,
			fake: false,
			hltv: false,
			loopback: false,
			no_channel: false,
		}
	}
}

/// An engine exported on the current thread, whose server has the clients
/// it was made with, and whose registry has `sv_cheats` and the variables it
/// was made with.
pub(crate) struct MockEngine {
	cheats: *mut sys::ConVar,
	clients: Vec<*mut ClientObject>,
}

impl MockEngine {
	/// Exports the engine on this thread. `sv_cheats` is listed first in the
	/// registry, then `vars`, and each variable is its own parent.
	pub(crate) fn new(clients: &[MockClient], cheats: &'static CStr, vars: &[MockVar]) -> Self {
		let mut next: *mut sys::ConCommandBase = null_mut();
		let mut listed = Vec::new();

		for var in vars.iter().rev() {
			let raw = match var.flags {
				Some(flags) => {
					let raw = mock_var(var.name, var.default, var.value, flags, next);

					listed.push(raw);
					raw.cast()
				}

				None => mock_command(var.name, next),
			};

			next = raw;
		}

		let flags = CommandFlags::REPLICATED | CommandFlags::NOTIFY;
		let cheats_var = mock_var(c"sv_cheats", c"0", cheats, flags, next);

		// SAFETY: `mock_var` returned a leaked, initialized variable.
		unsafe { (*cheats_var).m_nValue = c_int::from(is_set(cheats)) };
		listed.push(cheats_var);

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let cvar_vtable = unsafe {
			mock_vtable::<sys::ICvar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).ICvar_FindVar).write(find_var);
				(&raw mut (*vtable).ICvar_GetCommands).write(get_commands);
			})
		};
		let cvar = Box::into_raw(Box::new(CvarObject {
			interface: sys::ICvar {
				vtable_: Box::leak(cvar_vtable),
			},
			head: cheats_var.cast(),
			vars: listed,
		}));

		// SAFETY: As for the registry's vtable.
		let client_vtable: &'static _ = Box::leak(unsafe {
			mock_vtable::<sys::IClient__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IClient_GetNetChannel).write(get_net_channel);
				(&raw mut (*vtable).IClient_GetUserID).write(get_user_id);
				(&raw mut (*vtable).IClient_IsActive).write(is_active);
				(&raw mut (*vtable).IClient_IsConnected).write(is_connected);
				(&raw mut (*vtable).IClient_IsFakeClient).write(is_fake_client);
				(&raw mut (*vtable).IClient_IsHLTV).write(is_hltv);
			})
		});
		// SAFETY: As for the registry's vtable.
		let channel_vtable: &'static _ = Box::leak(unsafe {
			mock_vtable::<sys::INetChannel__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetChannel_IsLoopback).write(is_loopback);
					(&raw mut (*vtable).INetChannel_SendData).write(send_data);
				},
			)
		});

		let clients: Vec<*mut ClientObject> = clients
			.iter()
			.map(|&spec| {
				let channel = Box::into_raw(Box::new(ChannelObject {
					interface: sys::INetChannel {
						vtable_: channel_vtable,
					},
					loopback: spec.loopback,
					room: Cell::new(usize::MAX),
					sent: RefCell::new(Vec::new()),
				}));

				Box::into_raw(Box::new(ClientObject {
					interface: sys::IClient {
						vtable_: client_vtable,
					},
					channel,
					spec: Cell::new(spec),
				}))
			})
			.collect();

		// SAFETY: As for the registry's vtable.
		let server_vtable = unsafe {
			mock_vtable::<sys::IServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IServer_GetClient).write(get_client);
				(&raw mut (*vtable).IServer_GetClientCount).write(get_client_count);
			})
		};
		let server = Box::into_raw(Box::new(ServerObject {
			interface: sys::IServer {
				vtable_: Box::leak(server_vtable),
			},
			clients: clients.clone(),
		}));

		// SAFETY: As for the registry's vtable.
		let engine_vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_GetIServer).write(get_iserver);
				},
			)
		};
		let engine = Box::into_raw(Box::new(EngineObject {
			interface: sys::IVEngineServer {
				vtable_: Box::leak(engine_vtable),
			},
			server,
		}));

		export(Module::Engine, Cvar::VERSION, cvar);
		export(Module::Engine, ValveEngine::VERSION, engine);

		Self {
			cheats: cheats_var,
			clients,
		}
	}

	/// The client in `slot`.
	fn client(&self, slot: usize) -> &ClientObject {
		// SAFETY: `new` leaked the client, and tests only change it through
		// its `Cell`.
		unsafe { &*self.clients[slot] }
	}

	/// The client in `slot`, as the engine wrappers see it.
	pub(crate) fn game_client(&self, slot: usize) -> GameClient<'_> {
		let client = NonNull::new(self.clients[slot].cast()).unwrap();

		// SAFETY: `new` leaked the client, whose vtable answers every call
		// the wrappers make of a lease's client, so it outlives the borrow.
		unsafe { GameClient::from_raw(client) }
	}

	/// Makes `SendData` refuse data of more than `bits` bits for the client in
	/// `slot`.
	pub(crate) fn room(&self, slot: usize, bits: usize) {
		// SAFETY: `new` leaked the channel, and its room is a `Cell`.
		unsafe { (*self.client(slot).channel).room.set(bits) };
	}

	/// A server scoped to the mock, whose factories export it.
	pub(crate) fn server(&self) -> Server<'_> {
		mock_server(self)
	}

	/// Sets the server's own `sv_cheats`.
	pub(crate) fn set_cheats(&self, value: &'static CStr) {
		// SAFETY: `new` leaked the variable, and no reference to it is live:
		// the wrappers only read it with raw reads during calls.
		unsafe {
			(*self.cheats).m_pszString = value.as_ptr().cast_mut();
			(*self.cheats).m_nValue = c_int::from(is_set(value));
		}
	}

	/// Changes the client in `slot`.
	pub(crate) fn set_client(&self, slot: usize, spec: MockClient) {
		self.client(slot).spec.set(spec);
	}

	/// Takes what the client in `slot` was sent, decoded, one entry per
	/// `SendData` call.
	pub(crate) fn take_sent(&self, slot: usize) -> Vec<Vec<Decoded>> {
		// SAFETY: `new` leaked the channel, and what it was sent is in a
		// `RefCell`.
		let sent = unsafe { (*self.client(slot).channel).sent.take() };

		sent.iter().map(decode).collect()
	}
}

/// A console variable or command a [`MockEngine`] registers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MockVar {
	pub(crate) name: &'static CStr,
	pub(crate) default: &'static CStr,
	pub(crate) value: &'static CStr,
	/// The variable's flags, or `None` for a command.
	pub(crate) flags: Option<CommandFlags>,
}

impl MockVar {
	/// A command.
	pub(crate) const fn command(name: &'static CStr) -> Self {
		Self {
			name,
			default: c"",
			value: c"",
			flags: None,
		}
	}

	/// A variable.
	pub(crate) const fn var(
		name: &'static CStr,
		default: &'static CStr,
		value: &'static CStr,
		flags: CommandFlags,
	) -> Self {
		Self {
			name,
			default,
			value,
			flags: Some(flags),
		}
	}
}

#[repr(C)]
struct ServerObject {
	interface: sys::IServer,
	clients: Vec<*mut ClientObject>,
}

/// Decodes the messages in what a channel was sent.
pub(crate) fn decode(bits: &BitWriter) -> Vec<Decoded> {
	let mut reader = bits.reader();
	let mut messages = Vec::new();

	while reader.remaining() > 0 {
		let message = match reader.read_ubits(MESSAGE_TYPE_BITS).unwrap() {
			4 => Decoded::StringCmd(reader.read_cstring().unwrap()),

			5 => {
				let count = reader.read_u8().unwrap();
				let mut convars = Vec::new();

				for _ in 0..count {
					let name = reader.read_cstring().unwrap();

					convars.push((name, reader.read_cstring().unwrap()));
				}

				Decoded::SetConVar(convars)
			}

			31 => {
				let cookie = reader.read_i32().unwrap();

				Decoded::Query(cookie, reader.read_cstring().unwrap())
			}

			other => panic!("unexpected message type {other}"),
		};

		messages.push(message);
	}

	messages
}

unsafe extern "C" fn find_var(this: *mut sys::ICvar, name: *const c_char) -> *mut sys::ConVar {
	// SAFETY: The wrappers pass NUL-terminated names, and the only registry
	// is a `CvarObject`, which `MockEngine::new` leaked.
	let (name, cvar) = unsafe { (CStr::from_ptr(name), &*this.cast::<CvarObject>()) };

	cvar.vars
		.iter()
		.copied()
		.find(|&var| {
			// SAFETY: The registry's variables are leaked, and named by
			// string literals.
			let listed = unsafe { CStr::from_ptr((*var)._base.m_pszName) };

			listed.to_bytes().eq_ignore_ascii_case(name.to_bytes())
		})
		.unwrap_or(null_mut())
}

unsafe extern "C" fn get_client(this: *mut sys::IServer, slot: c_int) -> *mut sys::IClient {
	// SAFETY: The only server is a `ServerObject`, which `MockEngine::new`
	// leaked.
	let server = unsafe { &*this.cast::<ServerObject>() };

	server.clients[usize::try_from(slot).unwrap()].cast()
}

unsafe extern "C" fn get_client_count(this: *const sys::IServer) -> c_int {
	// SAFETY: As for `get_client`.
	unsafe { (*this.cast::<ServerObject>()).clients.len() as c_int }
}

unsafe extern "C" fn get_commands(this: *mut sys::ICvar) -> *mut sys::ConCommandBase {
	// SAFETY: As for `find_var`.
	unsafe { (*this.cast::<CvarObject>()).head }
}

unsafe extern "C" fn get_iserver(this: *mut sys::IVEngineServer) -> *mut sys::IServer {
	// SAFETY: The only engine is an `EngineObject`, which `MockEngine::new`
	// leaked.
	unsafe { (*this.cast::<EngineObject>()).server.cast() }
}

unsafe extern "C" fn get_net_channel(this: *mut sys::IClient) -> *mut sys::INetChannel {
	// SAFETY: Every client of the mock is a `ClientObject`.
	let client = unsafe { &*this.cast::<ClientObject>() };
	let spec = client.spec.get();

	match spec.user_id == 0 || spec.fake || spec.hltv || spec.no_channel {
		true => null_mut(),
		false => client.channel.cast(),
	}
}

unsafe extern "C" fn get_user_id(this: *const sys::IClient) -> c_int {
	// SAFETY: Every client of the mock is a `ClientObject`.
	unsafe { spec(this) }.user_id
}

unsafe extern "C" fn is_active(this: *const sys::IClient) -> bool {
	// SAFETY: As for `get_user_id`.
	unsafe { spec(this) }.active
}

unsafe extern "C" fn is_command(_: *const sys::ConCommandBase) -> bool {
	true
}

unsafe extern "C" fn is_connected(this: *const sys::IClient) -> bool {
	// SAFETY: As for `get_user_id`.
	unsafe { spec(this) }.user_id != 0
}

unsafe extern "C" fn is_fake_client(this: *const sys::IClient) -> bool {
	// SAFETY: As for `get_user_id`.
	let spec = unsafe { spec(this) };

	spec.fake || spec.hltv
}

unsafe extern "C" fn is_hltv(this: *const sys::IClient) -> bool {
	// SAFETY: As for `get_user_id`.
	unsafe { spec(this) }.hltv
}

unsafe extern "C" fn is_loopback(this: *const sys::INetChannel) -> bool {
	// SAFETY: Every channel of the mock is a leaked `ChannelObject`.
	unsafe { (*this.cast::<ChannelObject>()).loopback }
}

unsafe extern "C" fn is_variable(_: *const sys::ConCommandBase) -> bool {
	false
}

/// A `ConCommandBase` whose `IsCommand` returns `command`, linked to `next`.
fn mock_base(
	name: &'static CStr,
	command: bool,
	flags: CommandFlags,
	next: *mut sys::ConCommandBase,
) -> sys::ConCommandBase {
	// SAFETY: As for the registry's vtable in `MockEngine::new`.
	let vtable = unsafe {
		mock_vtable::<sys::ConCommandBase__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ConCommandBase_IsCommand).write(match command {
				true => is_command,
				false => is_variable,
			});
		})
	};

	sys::ConCommandBase {
		vtable_: Box::leak(vtable),
		m_pNext: next,
		m_bRegistered: true,
		m_pszName: name.as_ptr(),
		m_pszHelpString: c"".as_ptr(),
		m_nFlags: flags.bits(),
	}
}

/// A command linked to `next`.
fn mock_command(name: &'static CStr, next: *mut sys::ConCommandBase) -> *mut sys::ConCommandBase {
	Box::into_raw(Box::new(mock_base(name, true, CommandFlags::NONE, next)))
}

/// A variable that is its own parent, linked to `next`.
fn mock_var(
	name: &'static CStr,
	default: &'static CStr,
	value: &'static CStr,
	flags: CommandFlags,
	next: *mut sys::ConCommandBase,
) -> *mut sys::ConVar {
	// SAFETY: Zero is valid for every field of `ConVar`.
	let var = Box::into_raw(Box::new(unsafe { std::mem::zeroed::<sys::ConVar>() }));

	// SAFETY: `var` is the box just leaked, and nothing else refers to it yet.
	unsafe {
		(*var)._base = mock_base(name, false, flags, next);
		(*var).m_pParent = var;
		(*var).m_pszDefaultValue = default.as_ptr();
		(*var).m_pszString = value.as_ptr().cast_mut();
	}

	var
}

/// The cookie of the only query in what a client was sent.
pub(crate) fn query_cookie(sent: &[Decoded]) -> c_int {
	let mut cookies = sent.iter().filter_map(|message| match message {
		Decoded::Query(cookie, _) => Some(*cookie),
		_ => None,
	});
	let cookie = cookies.next().expect("a query");

	assert_eq!(cookies.next(), None, "a single query");
	cookie
}

/// A client's answer to a query for `sv_cheats`.
pub(crate) fn response(cookie: c_int, status: c_int, value: &CStr) -> Incoming {
	Incoming::RespondCvarValue {
		cookie,
		status,
		name: c"sv_cheats".to_owned(),
		value: value.to_owned(),
	}
}

unsafe extern "C" fn send_data(
	this: *mut sys::INetChannel,
	buffer: *mut sys::bf_write,
	reliable: bool,
) -> bool {
	// SAFETY: Every channel of the mock is a leaked `ChannelObject`, and
	// `NetChannel::send_encoded` passes a live `bf_write` it wrote.
	let (channel, bits) = unsafe {
		(
			&*this.cast::<ChannelObject>(),
			RawBfWrite::read_back(NonNull::new(buffer.cast()).unwrap()),
		)
	};
	let bits = bits.expect("a readable buffer");

	assert!(reliable, "leases only use the reliable stream");

	if bits.len() > channel.room.get() {
		return false;
	}

	channel.sent.borrow_mut().push(bits);
	true
}

/// The slot settings of a mock client.
///
/// # Safety
///
/// `client` must be one of the clients [`MockEngine::new`] made.
unsafe fn spec(client: *const sys::IClient) -> MockClient {
	// SAFETY: As the caller promises, the client is a leaked `ClientObject`.
	unsafe { (*client.cast::<ClientObject>()).spec.get() }
}
