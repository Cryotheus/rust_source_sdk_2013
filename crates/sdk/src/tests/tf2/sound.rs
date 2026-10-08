//! Tests of `crate::tf2::sound`: broadcasts through a fake event manager and
//! channels, and emitting and precaching through entities' native members.

use super::*;
use crate::bitbuf::BitWriter;
use crate::interfaces::{GameEventManager, ServerTools, ValveEngine};
use crate::net::MESSAGE_TYPE_BITS;
use crate::server::Module;
use crate::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::net::MockChannel;
use crate::test_support::server::{export, mock_server};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use crate::test_support::user_messages::recipients;
use sdk_raw::bitbuf::BfWrite;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::STRING;
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::ptr::{NonNull, null_mut};

/// The ID the mock manager encodes the event with.
const EVENT_ID: u32 = 113;

thread_local! {
	/// The mock channel, and a mask of the player slots that have it.
	static CHANNELS: Cell<(*mut sys::INetChannel, u32)> = const { Cell::new((null_mut(), 0)) };
	static CREATE_FAILS: Cell<bool> = const { Cell::new(false) };
	static CREATED: Cell<usize> = const { Cell::new(0) };
	static EMIT_REJECTS: Cell<bool> = const { Cell::new(false) };
	static EMITTED: RefCell<Vec<(*mut c_void, CString)>> = const { RefCell::new(Vec::new()) };
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	static EVENT_VTABLE: Box<sys::IGameEvent__bindgen_vtable> = unsafe {
		mock_vtable::<sys::IGameEvent__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IGameEvent_IsReliable).write(event_is_reliable);
			(&raw mut (*vtable).IGameEvent_SetInt).write(event_set_int);
			(&raw mut (*vtable).IGameEvent_SetString).write(event_set_string);
		})
	};
	static EVENT: Cell<sys::IGameEvent> = Cell::new(sys::IGameEvent {
		vtable_: EVENT_VTABLE.with(|vtable| &raw const **vtable),
	});
	static FREED: Cell<usize> = const { Cell::new(0) };
	static INTS: RefCell<Vec<(CString, c_int)>> = const { RefCell::new(Vec::new()) };
	static PRECACHE_REJECTS: Cell<bool> = const { Cell::new(false) };
	static PRECACHED: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
	static RELIABLE: Cell<bool> = const { Cell::new(true) };
	static SERIALIZE_FAILS: Cell<bool> = const { Cell::new(false) };
	static STRINGS: RefCell<Vec<(CString, CString)>> = const { RefCell::new(Vec::new()) };
	// SAFETY: As for `EVENT_VTABLE`.
	static UNKNOWN_VTABLE: Box<sys::IServerUnknown__bindgen_vtable> = unsafe {
		mock_vtable::<sys::IServerUnknown__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerUnknown_GetBaseEntity).write(unknown_base_entity);
		})
	};
	static WORLD: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
}

/// A `teamplay_broadcast_audio` event, as the mock manager encodes it in the
/// order of TF2's description: team, sound, flags, then player.
#[derive(Debug, PartialEq)]
struct Decoded {
	team: u8,
	sound: CString,
	flags: i16,
	player: i16,
}

#[test]
fn broadcasts_go_to_each_recipient_with_a_player_and_a_channel() {
	let (_engine_vtable, mut engine) = mock_engine();
	let (_manager_vtable, mut manager) = mock_manager();
	let mock = MockChannel::new();
	let mut unknown = player_unknown();
	// Slots 1 and 3 are clients in the game, which share the mock's recording
	// channel. Slot 2 is a bot, which has a player but no channel, and slot 4 a
	// client still connecting, which has a channel but no player yet.
	let mut table = edict_table(5, |_| false);

	for slot in [1, 2, 3] {
		table[slot]._base.m_pUnk = &raw mut unknown;
	}

	let edicts = table.as_mut_ptr();

	serve_edicts(edicts, table.len());
	CHANNELS.set((mock.channel().as_ptr(), 0b11010));
	export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
	export(Module::Engine, GameEventManager::VERSION, &raw mut manager);
	reset_event();

	let scope = ();
	let server = mock_server(&scope);
	let mut player = MockEntity::new(1);

	// SAFETY: Slot 3 lies within the table of 5 edicts.
	set_networking(null_mut(), unsafe { edicts.add(3) });

	let sent = broadcast(
		server,
		&recipients(&[1, 2, 3, 4, 9], false),
		c"Announcer.RoundBegins5Seconds",
		SoundFlags::STOP,
		Some(player.entity()),
	);

	assert_eq!(sent, Ok(2));
	assert_eq!(CREATED.get(), 1);
	assert_eq!(FREED.get(), 1, "the event is freed, never fired");

	let sends = mock.take_sent();

	assert_eq!(sends.len(), 2);

	for (bits, reliable) in &sends {
		assert!(*reliable);
		assert_eq!(
			decode(bits),
			Decoded {
				team: 255,
				sound: c"Announcer.RoundBegins5Seconds".to_owned(),
				flags: SoundFlags::STOP.bits() as i16,
				player: 3,
			}
		);
	}

	// Without a player, the event names none, and an event described as
	// unreliable is sent unreliably.
	RELIABLE.set(false);
	reset_event();
	assert_eq!(
		broadcast(
			server,
			&recipients(&[3], true),
			c"vo/announcer_ends_5sec.mp3",
			SoundFlags::NONE,
			None,
		),
		Ok(1)
	);

	let sends = mock.take_sent();
	let decoded = decode(&sends[0].0);

	assert!(!sends[0].1);
	assert_eq!((decoded.flags, decoded.player), (0, -1));
	assert_eq!(decoded.sound.as_c_str(), c"vo/announcer_ends_5sec.mp3");
	RELIABLE.set(true);

	// A full stream is not counted.
	mock.refuse();
	reset_event();
	assert_eq!(
		broadcast(
			server,
			&recipients(&[1], false),
			c"x",
			SoundFlags::NONE,
			None
		),
		Ok(0)
	);
	assert_eq!((CREATED.get(), FREED.get()), (1, 1));
	mock.take_sent();

	// No recipient has both a player and a channel, so no event is created:
	// the manager would refuse one while no client listens.
	reset_event();

	for players in [&[2][..], &[4], &[2, 4]] {
		assert_eq!(
			broadcast(
				server,
				&recipients(players, false),
				c"x",
				SoundFlags::NONE,
				None
			),
			Ok(0)
		);
	}

	assert_eq!(
		broadcast(server, &Recipients::new(), c"x", SoundFlags::NONE, None),
		Ok(0)
	);
	assert_eq!((CREATED.get(), FREED.get()), (0, 0));
	assert!(mock.take_sent().is_empty());

	// A player without an edict cannot be named.
	set_networking(null_mut(), null_mut());
	assert_eq!(
		broadcast(
			server,
			&recipients(&[1], false),
			c"x",
			SoundFlags::NONE,
			Some(player.entity()),
		),
		Err(BroadcastError::NotNetworked)
	);
	assert_eq!(CREATED.get(), 0);
	serve_edicts(null_mut(), 0);
	CHANNELS.set((null_mut(), 0));
}

#[test]
fn broadcasts_report_the_managers_refusals() {
	let (_engine_vtable, mut engine) = mock_engine();
	let (_manager_vtable, mut manager) = mock_manager();
	let mock = MockChannel::new();
	let mut unknown = player_unknown();
	let mut table = edict_table(2, |_| false);

	table[1]._base.m_pUnk = &raw mut unknown;
	serve_edicts(table.as_mut_ptr(), table.len());
	CHANNELS.set((mock.channel().as_ptr(), 0b10));
	export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
	export(Module::Engine, GameEventManager::VERSION, &raw mut manager);
	reset_event();

	let scope = ();
	let server = mock_server(&scope);
	let send = || {
		broadcast(
			server,
			&recipients(&[1], false),
			c"x",
			SoundFlags::NONE,
			None,
		)
	};

	// No listener asked for the event, though a client has a channel.
	CREATE_FAILS.set(true);
	assert!(matches!(send(), Err(BroadcastError::Create(_))));
	assert_eq!((CREATED.get(), FREED.get()), (0, 0));
	CREATE_FAILS.set(false);

	// The manager could not encode the event, which is freed anyway.
	SERIALIZE_FAILS.set(true);
	assert_eq!(send(), Err(BroadcastError::NotSerialized));
	assert_eq!((CREATED.get(), FREED.get()), (1, 1));
	SERIALIZE_FAILS.set(false);
	assert!(mock.take_sent().is_empty());
	serve_edicts(null_mut(), 0);
	CHANNELS.set((null_mut(), 0));
}

/// `IGameEventManager2::CreateEvent`, which hands out the one mock event,
/// unless [`CREATE_FAILS`] is set.
unsafe extern "C" fn create_event(
	_: *mut sys::IGameEventManager2,
	name: *const c_char,
	force: bool,
) -> *mut sys::IGameEvent {
	// SAFETY: The manager is passed a NUL-terminated event name.
	assert_eq!(unsafe { CStr::from_ptr(name) }, BROADCAST_EVENT);
	assert!(!force);

	if CREATE_FAILS.get() {
		return null_mut();
	}

	CREATED.set(CREATED.get() + 1);
	EVENT.with(Cell::as_ptr)
}

/// Reads back the event the mock manager encoded from a sent message.
fn decode(bits: &BitWriter) -> Decoded {
	let mut reader = bits.reader();

	assert_eq!(reader.read_ubits(MESSAGE_TYPE_BITS), Ok(25));

	let len = reader.read_ubits(11).unwrap() as usize;
	let payload = reader.read_bits(len).unwrap();

	assert_eq!(reader.remaining(), 0);

	let mut reader = payload.reader();

	assert_eq!(reader.read_ubits(9), Ok(EVENT_ID));

	let decoded = Decoded {
		team: reader.read_u8().unwrap(),
		sound: reader.read_cstring().unwrap(),
		flags: reader.read_i16().unwrap(),
		player: reader.read_i16().unwrap(),
	};

	assert_eq!(reader.remaining(), 0);
	decoded
}

unsafe extern "C" fn event_is_reliable(_: *const sys::IGameEvent) -> bool {
	RELIABLE.get()
}

unsafe extern "C" fn event_set_int(_: *mut sys::IGameEvent, key: *const c_char, value: c_int) {
	// SAFETY: Events are passed NUL-terminated keys.
	let key = unsafe { CStr::from_ptr(key) }.to_owned();

	INTS.with_borrow_mut(|ints| ints.push((key, value)));
}

unsafe extern "C" fn event_set_string(
	_: *mut sys::IGameEvent,
	key: *const c_char,
	value: *const c_char,
) {
	// SAFETY: Events are passed NUL-terminated keys and values.
	let (key, value) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };

	STRINGS.with_borrow_mut(|strings| strings.push((key.to_owned(), value.to_owned())));
}

unsafe extern "C" fn free_event(_: *mut sys::IGameEventManager2, event: *mut sys::IGameEvent) {
	assert_eq!(event, EVENT.with(Cell::as_ptr));
	FREED.set(FREED.get() + 1);
}

unsafe extern "C" fn get_entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	assert_eq!(index, 0);
	WORLD.get()
}

/// The integer the event was last given for `key`.
fn int(key: &CStr) -> c_int {
	INTS.with_borrow(|ints| {
		ints.iter()
			.rev()
			.find(|(name, _)| name.as_c_str() == key)
			.map(|&(_, value)| value)
			.expect("the key was set")
	})
}

/// An engine serving the edicts of [`serve_edicts`], and the channels of
/// [`CHANNELS`].
fn mock_engine() -> (
	Box<sys::IVEngineServer__bindgen_vtable>,
	sys::IVEngineServer,
) {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
			(&raw mut (*vtable).IVEngineServer_GetPlayerNetInfo).write(net_info);
		})
	};
	let engine = sys::IVEngineServer {
		vtable_: &raw const *vtable,
	};

	(vtable, engine)
}

/// A game event manager that creates, frees, and encodes the mock event.
fn mock_manager() -> (
	Box<sys::IGameEventManager2__bindgen_vtable>,
	sys::IGameEventManager2,
) {
	// SAFETY: As for the engine's vtable in `mock_engine`.
	let vtable = unsafe {
		mock_vtable::<sys::IGameEventManager2__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IGameEventManager2_CreateEvent).write(create_event);
				(&raw mut (*vtable).IGameEventManager2_FreeEvent).write(free_event);
				(&raw mut (*vtable).IGameEventManager2_SerializeEvent).write(serialize_event);
			},
		)
	};
	let manager = sys::IGameEventManager2 {
		vtable_: &raw const *vtable,
	};

	(vtable, manager)
}

/// The mock channel for the player slots whose bit is set in the mask.
unsafe extern "C" fn net_info(
	_: *mut sys::IVEngineServer,
	index: c_int,
) -> *mut sys::INetChannelInfo {
	let (channel, mask) = CHANNELS.get();

	match u32::try_from(index) {
		Ok(index) if index < u32::BITS && mask & (1 << index) != 0 => channel.cast(),
		_ => null_mut(),
	}
}

/// The `IServerUnknown` of a player slot's entity, which the slot's edict
/// points to.
fn player_unknown() -> sys::IServerUnknown {
	sys::IServerUnknown {
		vtable_: UNKNOWN_VTABLE.with(|vtable| &raw const **vtable),
	}
}

/// The adapter of `CBaseEntity::EmitSound`, which records the entity and the
/// name, unless [`EMIT_REJECTS`] is set.
unsafe extern "C" fn emit_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 1);
	assert!(result.is_null(), "a void member gets no result");

	if EMIT_REJECTS.get() {
		return false;
	}

	// SAFETY: The binding declares one string parameter, so the caller passes
	// one string variant, NUL-terminated.
	let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };

	EMITTED.with_borrow_mut(|emitted| emitted.push((object, name.to_owned())));
	true
}

/// The adapter of `CBaseEntity::PrecacheScriptSound`, which records the name,
/// unless [`PRECACHE_REJECTS`] is set.
unsafe extern "C" fn precache_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(object, WORLD.get().cast());
	assert_eq!(count, 1);
	assert!(result.is_null(), "a void member gets no result");

	if PRECACHE_REJECTS.get() {
		return false;
	}

	// SAFETY: The binding declares one string parameter, so the caller passes
	// one string variant, NUL-terminated.
	let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };

	PRECACHED.with_borrow_mut(|precached| precached.push(name.to_owned()));
	true
}

/// Forgets the values and calls that earlier broadcasts recorded.
fn reset_event() {
	CREATED.set(0);
	FREED.set(0);
	INTS.take();
	STRINGS.take();
}

#[test]
fn script_sounds_are_emitted_through_the_entitys_native_member() {
	let scope = ();
	let server = mock_server(&scope);
	let mut parameters = [STRING];
	let mut bindings = [member_binding(
		c"EmitSound",
		binding::VOID,
		&mut parameters,
		Some(emit_adapter),
	)];
	let mut description = class_description(c"CBaseEntity", &mut bindings, null_mut());

	// Changed below only through the pointer `call` reads it by.
	let binding = description.m_FunctionBindings.m_Memory.m_pMemory;
	let mut mock = MockEntity::new(1);
	let pointer = mock.as_ptr();
	let mut vtable = [unexpected_call as *const (); SCRIPT_DESCRIPTION_SLOT + 1];

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

	vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();
	// SAFETY: A mock entity starts with the pointer to its vtable, and the new
	// vtable outlives the entity's last use, at the end of the test.
	unsafe { pointer.cast::<*const *const ()>().write(vtable.as_ptr()) };
	set_script_description(&raw mut description);
	EMITTED.take();

	// SAFETY: Mock entities are leaked, and the vtable answers what the call
	// reads of an entity.
	let entity = unsafe { Entity::from_raw(NonNull::new(pointer).unwrap()) };

	assert_eq!(
		emit_script_sound(server, entity, c"TFPlayer.Decapitated"),
		Ok(())
	);
	assert_eq!(
		EMITTED.take(),
		[(pointer.cast(), c"TFPlayer.Decapitated".to_owned())]
	);

	// The adapter's refusal is reported.
	EMIT_REJECTS.set(true);
	assert_eq!(
		emit_script_sound(server, entity, c"TFPlayer.Decapitated"),
		Err(EmitError::Rejected)
	);
	EMIT_REJECTS.set(false);

	// An entity marked for deletion is not used.
	mock.set_eflags(1);
	assert_eq!(
		emit_script_sound(server, entity, c"TFPlayer.Decapitated"),
		Err(EmitError::MarkedForDeletion)
	);
	mock.set_eflags(0);

	// Another signature is refused before the call.
	// SAFETY: `binding` points to the binding in `bindings`, which is alive,
	// and which the call only reads through the same pointer.
	unsafe { (*binding).m_desc.m_ReturnType = binding::FLOAT };
	assert_eq!(
		emit_script_sound(server, entity, c"TFPlayer.Decapitated"),
		Err(EmitError::UnsupportedMethod)
	);
	assert!(EMITTED.take().is_empty());
	set_script_description(null_mut());
}

#[test]
fn script_sounds_are_precached_through_the_worlds_native_member() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let tools_vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex).write(get_entity_by_index);
		})
	};
	let mut tools = sys::IServerTools {
		vtable_: &raw const *tools_vtable,
	};

	export(Module::GameServer, ServerTools::VERSION, &raw mut tools);
	WORLD.set(null_mut());

	let scope = ();
	let server = mock_server(&scope);

	assert_eq!(
		precache_script_sound(server, c"Scout.Thanks01"),
		Err(PrecacheError::NoWorld)
	);

	// A world whose script class is `CBaseEntity`, with the one binding.
	let mut parameters = [STRING];
	let mut bindings = [member_binding(
		c"PrecacheScriptSound",
		binding::VOID,
		&mut parameters,
		Some(precache_adapter),
	)];
	let mut description = class_description(c"CBaseEntity", &mut bindings, null_mut());

	// Changed below only through the pointer `call` reads it by.
	let binding = description.m_FunctionBindings.m_Memory.m_pMemory;
	let mut world = MockEntity::new(0);
	let world_ptr = world.as_ptr();
	let get_description = SCRIPT_DESCRIPTION_SLOT;
	let mut vtable = vec![unexpected_call as *const (); get_description + 1];

	// The mock's own vtable answers every slot before the descriptor's, which
	// include the datamap's.
	// SAFETY: A mock entity starts with the pointer to its vtable, which has
	// slots up to TF2's `Teleport`, past `GetScriptDesc`.
	unsafe {
		let original = world_ptr.cast::<*const *const ()>().read();

		for (slot, entry) in vtable.iter_mut().enumerate().take(get_description) {
			*entry = original.add(slot).read();
		}
	}

	vtable[get_description] = script_description as *const ();
	// SAFETY: A mock entity starts with the pointer to its vtable, and the new
	// vtable outlives the world's last use, at the end of the test.
	unsafe { world_ptr.cast::<*const *const ()>().write(vtable.as_ptr()) };
	set_script_description(&raw mut description);
	WORLD.set(world_ptr);
	PRECACHED.take();

	assert_eq!(precache_script_sound(server, c"Scout.Thanks01"), Ok(()));
	assert_eq!(PRECACHED.take(), [c"Scout.Thanks01".to_owned()]);

	// The adapter's refusal is reported.
	PRECACHE_REJECTS.set(true);
	assert_eq!(
		precache_script_sound(server, c"Scout.Thanks01"),
		Err(PrecacheError::Rejected)
	);
	PRECACHE_REJECTS.set(false);

	// A world marked for deletion is not used.
	world.set_eflags(1);
	assert_eq!(
		precache_script_sound(server, c"Scout.Thanks01"),
		Err(PrecacheError::NoWorld)
	);
	world.set_eflags(0);

	// Another signature is refused before the call.
	// SAFETY: `binding` points to the binding in `bindings`, which is alive,
	// and which the wrapper only reads through the same pointer.
	unsafe { (*binding).m_desc.m_ReturnType = binding::FLOAT };
	assert_eq!(
		precache_script_sound(server, c"Scout.Thanks01"),
		Err(PrecacheError::UnsupportedMethod)
	);
	assert!(PRECACHED.take().is_empty());
	WORLD.set(null_mut());
	set_script_description(null_mut());
}

/// Encodes the event's ID, then its fields as TF2 describes them, from what
/// the event was last set to.
unsafe extern "C" fn serialize_event(
	_: *mut sys::IGameEventManager2,
	event: *mut sys::IGameEvent,
	buffer: *mut sys::bf_write,
) -> bool {
	assert_eq!(event, EVENT.with(Cell::as_ptr));

	if SERIALIZE_FAILS.get() {
		return false;
	}

	let sound = STRINGS.with_borrow(|strings| {
		let (key, value) = strings.last().expect("the sound was set");

		assert_eq!(key.as_c_str(), c"sound");
		value.clone()
	});
	let mut bits = BitWriter::new();

	bits.write_ubits(EVENT_ID, 9);
	bits.write_u8(int(c"team") as u8);
	bits.write_cstr(&sound);
	bits.write_i16(int(c"additional_flags") as i16);
	bits.write_i16(int(c"player") as i16);

	// SAFETY: The wrapper passes a live `bf_write` over its own word-aligned
	// buffer, which nothing else accesses while the manager encodes into it.
	unsafe {
		BfWrite::append(
			NonNull::new(buffer.cast()).unwrap(),
			bits.as_words(),
			bits.len(),
		)
	}
}

#[test]
fn the_longest_sound_name_fills_a_game_event_message() {
	let (_engine_vtable, mut engine) = mock_engine();
	let (_manager_vtable, mut manager) = mock_manager();
	let mock = MockChannel::new();
	let mut unknown = player_unknown();
	// Slot 1 is a client in the game.
	let mut table = edict_table(2, |_| false);

	table[1]._base.m_pUnk = &raw mut unknown;
	serve_edicts(table.as_mut_ptr(), table.len());
	CHANNELS.set((mock.channel().as_ptr(), 0b10));
	export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
	export(Module::Engine, GameEventManager::VERSION, &raw mut manager);
	reset_event();

	let scope = ();
	let server = mock_server(&scope);
	let longest = CString::new(vec![b'a'; MAX_SOUND_LEN]).unwrap();

	// The longest name fills the message, and arrives whole.
	assert_eq!(
		broadcast(
			server,
			&recipients(&[1], false),
			&longest,
			SoundFlags::NONE,
			None
		),
		Ok(1)
	);
	assert_eq!(decode(&mock.take_sent()[0].0).sound, longest);
	serve_edicts(null_mut(), 0);
	CHANNELS.set((null_mut(), 0));
}

/// Any entity, as the base entity of every [`player_unknown`].
unsafe extern "C" fn unknown_base_entity(
	unknown: *mut sys::IServerUnknown,
) -> *mut sys::CBaseEntity {
	unknown.cast()
}
