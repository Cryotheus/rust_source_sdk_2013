//! TF2 sounds played to chosen clients without a position, as TF2's announcer
//! plays its lines, and the precaching of sound script entries.
//!
//! [`broadcast`] sends each recipient TF2's `teamplay_broadcast_audio` game
//! event, which the client answers by playing the sound from its own view.
//! The client resolves the name, so it can be a sound script entry, such as
//! `Announcer.RoundBegins5Seconds`, or a sample path. For a sound positioned
//! in the world or played from an entity, use
//! [`EngineSound::emit_sound`](crate::interfaces::EngineSound::emit_sound),
//! which takes precached samples only. To make a player speak a voice line,
//! use [`voice`](crate::tf2::voice).
//!
//! # Precaching
//!
//! The server's precache table lists the samples clients load ahead of play.
//! It is emptied at every level change, so a plugin precaches its sounds
//! again for each map: samples through
//! [`EngineSound::precache_sound`](crate::interfaces::EngineSound::precache_sound),
//! and every sample of a sound script entry through [`precache_script_sound`].
//! TF2 precaches the sounds it plays itself, such as every voice line its
//! response rules can choose, when the level starts.
//!
//! # Bots
//!
//! Bots and other fake clients have no client, so they hear nothing.
//! [`broadcast`] skips them.
//!
//! # Unverified
//!
//! On TF2's 64-bit Windows server, a client has been observed to play a
//! broadcast sound script entry, `Game.YourTeamWon`, as its handler in the
//! SDK does (`game/client/clientmode_shared.cpp`): from its own view, not
//! positioned in the world. Whether a client loads on demand a sample the
//! server never precached has not been tested, nor have Linux servers.

use crate::entities::Entity;
use crate::interfaces::engine_sound::SoundFlags;
use crate::interfaces::game_event::CreateEventError;
use crate::net::messages::GameEvent;
use crate::net::{EncodeError, NetChannel, NetMessage, Reliability};
use crate::tf2::game_events::GameEventId;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::user_messages::Recipients;
use crate::{Game, InterfaceError, Server};
use std::ffi::{CStr, c_int};

/// The `team` of a `teamplay_broadcast_audio` event that clients on every team
/// play.
const ANY_TEAM: c_int = 255;

/// The game event TF2's `BroadcastSound` fires.
const BROADCAST_EVENT: &CStr = GameEventId::TeamplayBroadcastAudio.name_cstr();

/// The longest sound name, in bytes, that [`broadcast`] sends.
///
/// An `svc_GameEvent` carries at most 2047 bits of event. TF2 describes
/// `teamplay_broadcast_audio` (`resource/modevents.res`) as a byte `team`,
/// the string `sound`, and the shorts `additional_flags` and `player`, which
/// with the event's 9-bit ID take 49 bits besides the name, leaving 249
/// bytes for the name and its terminator.
pub const MAX_SOUND_LEN: usize = 248;

/// The `player` of a `teamplay_broadcast_audio` event that names no player.
const NO_PLAYER: c_int = -1;

/// Why [`broadcast`] could not send its event.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BroadcastError {
	/// The game event manager did not create `teamplay_broadcast_audio`,
	/// which it refuses for an event the game did not declare or no listener
	/// asked for. A TF2 client asks for it while it connects, by sending its
	/// listeners to the engine (`clc_ListenEvents`), so until a human client
	/// has done so, this can fail even though a recipient has a player and a
	/// channel.
	#[error(transparent)]
	Create(#[from] CreateEventError),

	/// The sound's name is longer than [`MAX_SOUND_LEN`] bytes, or the encoded
	/// event does not fit in an `svc_GameEvent`, which carries fewer than 2048
	/// bits. The name's length is checked before anything else.
	#[error(transparent)]
	Encode(#[from] EncodeError),

	/// An interface needed to send the event is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The entity given for the sound's voice has no edict, so no client
	/// knows it.
	#[error("the voice's entity is not networked, so no client knows it")]
	NotNetworked,

	/// The game event manager did not encode the event, which it refuses for
	/// an event without a description.
	#[error("the game event manager could not encode the event")]
	NotSerialized,

	/// The server does not run TF2.
	#[error("broadcast sounds require a TF2 server")]
	NotTf2,
}

/// Why [`precache_script_sound`] could not precache a sound.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrecacheError {
	/// An interface needed to find the world is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// No map is loaded, its entities have not spawned yet, or the world is
	/// marked for deletion.
	#[error("the world entity, which the precache is made through, is missing")]
	NoWorld,

	/// The server does not run TF2.
	#[error("script sound precaching requires a TF2 server")]
	NotTf2,

	/// The binding adapter of the world's `PrecacheScriptSound` reported
	/// failure.
	#[error("the native PrecacheScriptSound method rejected its arguments")]
	Rejected,

	/// The world's script class descriptors lack `PrecacheScriptSound`, or
	/// its signature differs from the SDK's.
	#[error("the game does not expose the expected native PrecacheScriptSound method")]
	UnsupportedMethod,
}

impl From<BindingError> for PrecacheError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// Plays a sound to every client `recipients` names, from the client's own
/// view, as TF2's announcer lines play. Returns how many clients' channels
/// queued it.
///
/// This builds the `teamplay_broadcast_audio` event that TF2's
/// `CTFGameRules::BroadcastSound` fires (`game/shared/tf/tf_gamerules.cpp`),
/// but instead of firing it to every client, sends it to each recipient as an
/// `svc_GameEvent`, in the stream the event's description asks for. The
/// client plays `sound`, a sound script entry or a sample path of at most
/// [`MAX_SOUND_LEN`] bytes, with `flags` added, through
/// `C_BaseEntity::EmitSound` from `SOUND_FROM_LOCAL_PLAYER`, so it does not
/// fade with distance. The server neither resolves nor checks the name.
///
/// # Teams
///
/// The client plays the event only for its `team`
/// (`ClientModeShared::FireGameEvent` in `game/client/clientmode_shared.cpp`):
/// a client plays a team number if it is on that team, or spectates a player
/// on it in first-person, chase or point-of-interest mode. Team 0 plays only
/// for a client that finds no team of its own (`GetLocalTeam` is null) and
/// spectates a player on team 0, which almost never happens. Team 255 plays
/// for every client whatever its team, and is what this sends, so
/// `recipients` alone choose who hears the sound.
///
/// # Voice
///
/// `voice_of` fills the event's `player` field. If it is a TF2 player, and
/// the sound plays on `CHAN_VOICE`, each client pitches the sound as it
/// pitches that player's voice (`C_TFPlayer::ClientAdjustStartSoundParams`),
/// such as for Pyrovision or a voice-changing spell. `None` sends -1, which no
/// client adjusts the sound for. Clients adjust the sound for TF2 players
/// only, so any other networked entity changes nothing, as `None` does.
///
/// # Recipients
///
/// Only recipients with a player entity and a net channel get the event.
/// Bots, SourceTV and empty slots have no channel, and a client still
/// connecting has no player until the game creates one
/// (`ClientPutInServer`), so all of them are skipped. If none is left, this
/// returns `Ok(0)` before creating the event, which the game event manager
/// refuses to create while no client listens for it, as on a server with
/// only bots. A recipient whose stream has no room left for the event is not
/// counted.
///
/// [`Recipients::reliable`] has no effect here: the event goes in the stream
/// its description asks for, as when the game fires it.
///
/// The event is sent, never fired, so no listener on the server sees it.
/// SourceTV relays the fired event to its spectators and demos
/// (`CTFHLTVDirector::GetModEvents` in `game/server/tf/tf_hltvdirector.cpp`),
/// but SourceTV demos and Replay recordings never contain a broadcast.
///
/// # Errors
///
/// The sound's length and `voice_of` are checked first, so an empty or
/// bot-only `recipients` fails as any other would: with
/// [`BroadcastError::Encode`] for a sound longer than [`MAX_SOUND_LEN`] bytes,
/// and with [`BroadcastError::NotNetworked`] for a `voice_of` without an
/// edict.
#[doc(alias("BroadcastSound", "teamplay_broadcast_audio"))]
pub fn broadcast(
	server: Server<'_>,
	recipients: &Recipients,
	sound: &CStr,
	flags: SoundFlags,
	voice_of: Option<Entity<'_>>,
) -> Result<usize, BroadcastError> {
	if server.game() != Game::TeamFortress2 {
		return Err(BroadcastError::NotTf2);
	}

	EncodeError::check_len("sound", sound.count_bytes(), MAX_SOUND_LEN)?;

	let player = match voice_of {
		Some(player) => player.edict().ok_or(BroadcastError::NotNetworked)?.index(),
		None => NO_PLAYER,
	};
	let engine = server.valve_engine()?;
	let channels: Vec<NetChannel<'_>> = recipients
		.players()
		.iter()
		.filter_map(|&index| engine.edict_of_index(index))
		.filter(|edict| edict.entity().is_some())
		.filter_map(|edict| engine.net_channel(edict))
		.collect();

	if channels.is_empty() {
		return Ok(0);
	}

	let events = server.game_events()?;
	let mut event = events.create_event(BROADCAST_EVENT)?;

	event.set_int(c"team", ANY_TEAM);
	event.set_string(c"sound", sound);
	event.set_int(c"additional_flags", flags.bits());
	event.set_int(c"player", player);

	// The engine, which the SDK does not include, sends a fired event to each
	// client in the stream `IGameEvent::IsReliable` names.
	let reliability = match event.as_event().is_reliable() {
		true => Reliability::Reliable,
		false => Reliability::Unreliable,
	};
	let data = events
		.serialize_event(event.as_event())
		.ok_or(BroadcastError::NotSerialized)?;

	drop(event);

	let message = GameEvent { data: &data }.encode()?;

	Ok(channels
		.into_iter()
		.filter(|channel| channel.send_encoded(&message, reliability).is_ok())
		.count())
}

/// Precaches every sample of a sound script entry, such as `Scout.Thanks01`,
/// for the current level, as the game's `CBaseEntity::PrecacheScriptSound`
/// does.
///
/// A name that is no entry is precached as a sample path if it contains
/// `.wav` in any case, or `.mp3` in lowercase only
/// (`CSoundEmitterSystem::PrecacheScriptSound` in
/// `game/shared/SoundEmitterSystem.cpp`), so give sample paths a lowercase
/// extension.
///
/// This calls the `PrecacheScriptSound` member that `CBaseEntity` exposes to
/// VScript (`game/server/baseentity.cpp`) on the world entity, through its
/// native binding, without a script VM. The member returns nothing, so an
/// unknown entry is not reported as an error; the game prints
/// `PrecacheScriptSound '...' failed, no such sound script entry` to the
/// developer console instead, the first time each name fails.
///
/// # Timing
///
/// The world exists once the map's entities have spawned; before that, this
/// fails with [`PrecacheError::NoWorld`]. The game allows precaching until
/// the end of its own `ServerActivate`, which closes the window
/// (`CBaseEntity::SetAllowPrecache(false)` in `game/server/gameinterface.cpp`),
/// so precache in a hook that runs after the game's `LevelInit`, or before
/// the game's `ServerActivate`. A callback that runs after the game's
/// `ServerActivate`, such as a post-hook of it, is already late. Unlike
/// VScript's global `PrecacheScriptSound`, the member does not reopen the
/// window, so a late call still precaches each sample, but the game warns
/// `Late precache of ...` for each one the engine reports as not precached
/// yet (`CBaseEntity::PrecacheSound`).
#[doc(alias("PrecacheScriptSound"))]
pub fn precache_script_sound(server: Server<'_>, name: &CStr) -> Result<(), PrecacheError> {
	if server.game() != Game::TeamFortress2 {
		return Err(PrecacheError::NotTf2);
	}

	let world = server
		.server_tools()?
		.entity_by_index(0)
		.filter(|world| !world.is_marked_for_deletion())
		.ok_or(PrecacheError::NoWorld)?;

	// SAFETY: The checked `CBaseEntity` member looks the entry up and adds its
	// samples to the engine's precache table, which copies them. It creates
	// and frees no entities, and reads the name only during the call.
	unsafe {
		binding::call(
			world,
			c"CBaseEntity",
			c"PrecacheScriptSound",
			&mut [binding::string(name)],
			binding::VOID,
		)?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::InterfaceFactory;
	use crate::bitbuf::BitWriter;
	use crate::edicts::test_support::mock_edict;
	use crate::entities::test_support::{MockEntity, set_networking};
	use crate::interfaces::{GameEventManager, ServerTools, ValveEngine};
	use crate::net::MESSAGE_TYPE_BITS;
	use crate::net::test_support::MockChannel;
	use crate::server::Module;
	use crate::server::test_support::{export, mock_server};
	use crate::user_messages::test_support::recipients;
	use sdk_raw::bitbuf::BfWrite;
	use sdk_raw::test_support::{mock_vtable, unexpected_call};
	use sdk_raw::tf2::script_binding::SF_MEMBER_FUNC;
	use std::cell::{Cell, RefCell};
	use std::ffi::{CString, c_char, c_void};
	use std::marker::PhantomData;
	use std::mem::zeroed;
	use std::ptr::{NonNull, null_mut};

	/// The ID the mock manager encodes the event with.
	const EVENT_ID: u32 = 113;

	thread_local! {
		/// The mock channel, and a mask of the player slots that have it.
		static CHANNELS: Cell<(*mut sys::INetChannel, u32)> = const { Cell::new((null_mut(), 0)) };
		static CREATE_FAILS: Cell<bool> = const { Cell::new(false) };
		static CREATED: Cell<usize> = const { Cell::new(0) };
		static DESCRIPTION: Cell<*mut sys::ScriptClassDesc_t> = const { Cell::new(null_mut()) };
		static EDICTS: Cell<(*mut sys::edict_t, usize)> = const { Cell::new((null_mut(), 0)) };
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
		static UNKNOWN_VTABLE: Box<sys::IServerUnknown__bindgen_vtable> = unsafe {
			mock_vtable::<sys::IServerUnknown__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IServerUnknown_GetBaseEntity).write(unknown_base_entity);
			})
		};
		static WORLD: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
	}

	/// A `teamplay_broadcast_audio` event, as the mock manager encodes it in
	/// the order of TF2's description: team, sound, flags, then player.
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
		// Slots 1 and 3 are clients in the game, which share the mock's
		// recording channel. Slot 2 is a bot, which has a player but no
		// channel, and slot 4 a client still connecting, which has a channel
		// but no player yet.
		let mut table = [0, 1, 2, 3, 4].map(|index| mock_edict(index, false));

		for slot in [1, 2, 3] {
			table[slot]._base.m_pUnk = &raw mut unknown;
		}

		let edicts = table.as_mut_ptr();

		EDICTS.set((edicts, table.len()));
		CHANNELS.set((mock.channel().as_ptr(), 0b11010));
		export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
		export(Module::Engine, GameEventManager::VERSION, &raw mut manager);
		reset_event();

		let scope = ();
		let server = mock_server(&scope);
		let mut player = MockEntity::new(1);

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

		// No recipient has both a player and a channel, so no event is
		// created: the manager would refuse one while no client listens.
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
		EDICTS.set((null_mut(), 0));
		CHANNELS.set((null_mut(), 0));
	}

	#[test]
	fn broadcasts_report_the_managers_refusals() {
		let (_engine_vtable, mut engine) = mock_engine();
		let (_manager_vtable, mut manager) = mock_manager();
		let mock = MockChannel::new();
		let mut unknown = player_unknown();
		let mut table = [0, 1].map(|index| mock_edict(index, false));

		table[1]._base.m_pUnk = &raw mut unknown;
		EDICTS.set((table.as_mut_ptr(), table.len()));
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
		EDICTS.set((null_mut(), 0));
		CHANNELS.set((null_mut(), 0));
	}

	#[test]
	fn broadcasts_too_long_for_a_game_event_message_are_refused_first() {
		let (_engine_vtable, mut engine) = mock_engine();
		let (_manager_vtable, mut manager) = mock_manager();
		let mock = MockChannel::new();
		let mut unknown = player_unknown();
		// Slot 1 is a client in the game, and slot 2 a bot.
		let mut table = [0, 1, 2].map(|index| mock_edict(index, false));

		table[1]._base.m_pUnk = &raw mut unknown;
		table[2]._base.m_pUnk = &raw mut unknown;
		EDICTS.set((table.as_mut_ptr(), table.len()));
		CHANNELS.set((mock.channel().as_ptr(), 0b10));
		export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
		export(Module::Engine, GameEventManager::VERSION, &raw mut manager);
		reset_event();

		let scope = ();
		let server = mock_server(&scope);
		let longest = CString::new(vec![b'a'; MAX_SOUND_LEN]).unwrap();

		// The longest name fills the message.
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

		// A longer one is refused before anything else, whoever would hear it.
		reset_event();

		for len in [MAX_SOUND_LEN + 1, 300] {
			let long = CString::new(vec![b'a'; len]).unwrap();

			for players in [&[1][..], &[2], &[]] {
				assert_eq!(
					broadcast(
						server,
						&recipients(players, false),
						&long,
						SoundFlags::NONE,
						None
					),
					Err(BroadcastError::Encode(EncodeError::TooLong {
						field: "sound",
						len,
						max: MAX_SOUND_LEN,
					}))
				);
			}
		}

		assert_eq!((CREATED.get(), FREED.get()), (0, 0));
		assert!(mock.take_sent().is_empty());
		EDICTS.set((null_mut(), 0));
		CHANNELS.set((null_mut(), 0));
	}

	unsafe extern "C" fn create_event(
		_: *mut sys::IGameEventManager2,
		name: *const c_char,
		force: bool,
	) -> *mut sys::IGameEvent {
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

	unsafe extern "C" fn edict_of_index(
		_: *mut sys::IVEngineServer,
		index: c_int,
	) -> *mut sys::edict_t {
		let (table, len) = EDICTS.get();

		match usize::try_from(index) {
			Ok(slot) if slot < len => unsafe { table.add(slot) },
			_ => null_mut(),
		}
	}

	unsafe extern "C" fn event_is_reliable(_: *const sys::IGameEvent) -> bool {
		RELIABLE.get()
	}

	unsafe extern "C" fn event_set_int(_: *mut sys::IGameEvent, key: *const c_char, value: c_int) {
		let key = unsafe { CStr::from_ptr(key) }.to_owned();

		INTS.with_borrow_mut(|ints| ints.push((key, value)));
	}

	unsafe extern "C" fn event_set_string(
		_: *mut sys::IGameEvent,
		key: *const c_char,
		value: *const c_char,
	) {
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

	fn mock_engine() -> (
		Box<sys::IVEngineServer__bindgen_vtable>,
		sys::IVEngineServer,
	) {
		let vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
					(&raw mut (*vtable).IVEngineServer_GetPlayerNetInfo).write(net_info);
				},
			)
		};
		let engine = sys::IVEngineServer {
			vtable_: &raw const *vtable,
		};

		(vtable, engine)
	}

	fn mock_manager() -> (
		Box<sys::IGameEventManager2__bindgen_vtable>,
		sys::IGameEventManager2,
	) {
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

	unsafe extern "C" fn no_interface(_: *const c_char, _: *mut c_int) -> *mut c_void {
		null_mut()
	}

	#[test]
	fn other_games_are_refused() {
		let scope = ();
		let factory = InterfaceFactory::new(no_interface);
		let server = unsafe { Server::new(factory, factory, Game::SourceSdk2013, &scope) };

		assert_eq!(
			broadcast(server, &Recipients::new(), c"x", SoundFlags::NONE, None),
			Err(BroadcastError::NotTf2)
		);
		assert_eq!(
			precache_script_sound(server, c"x"),
			Err(PrecacheError::NotTf2)
		);
	}

	/// The `IServerUnknown` of a player slot's entity, which the slot's edict
	/// points to.
	fn player_unknown() -> sys::IServerUnknown {
		sys::IServerUnknown {
			vtable_: UNKNOWN_VTABLE.with(|vtable| &raw const **vtable),
		}
	}

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

	unsafe extern "C" fn script_description(
		_: *mut sys::CBaseEntity,
	) -> *mut sys::ScriptClassDesc_t {
		DESCRIPTION.get()
	}

	#[test]
	fn script_sounds_are_precached_through_the_worlds_native_member() {
		let tools_vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex)
						.write(get_entity_by_index);
				},
			)
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
		let mut parameters = [binding::STRING];
		let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };

		bindings[0].m_desc.m_pszScriptName = c"PrecacheScriptSound".as_ptr();
		bindings[0].m_desc.m_ReturnType = binding::VOID;
		bindings[0].m_desc.m_Parameters = vector(&mut parameters);
		bindings[0].m_flags = SF_MEMBER_FUNC;
		bindings[0].m_pfnBinding = Some(precache_adapter);

		let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };

		description.m_pszClassname = c"CBaseEntity".as_ptr();
		description.m_FunctionBindings = vector(&mut bindings);

		// Changed below only through the pointer `call` reads it by.
		let binding = description.m_FunctionBindings.m_Memory.m_pMemory;
		let mut world = MockEntity::new(0);
		let world_ptr = world.as_ptr();
		let get_description =
			sdk_raw::vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetScriptDesc);
		let mut vtable = vec![unexpected_call as *const (); get_description + 1];

		// The mock's own vtable answers every slot before the descriptor's,
		// which include the datamap's.
		unsafe {
			let original = world_ptr.cast::<*const *const ()>().read();

			for (slot, entry) in vtable.iter_mut().enumerate().take(get_description) {
				*entry = original.add(slot).read();
			}
		}

		vtable[get_description] = script_description as *const ();
		unsafe { world_ptr.cast::<*const *const ()>().write(vtable.as_ptr()) };
		DESCRIPTION.set(&raw mut description);
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
		unsafe { (*binding).m_desc.m_ReturnType = binding::FLOAT };
		assert_eq!(
			precache_script_sound(server, c"Scout.Thanks01"),
			Err(PrecacheError::UnsupportedMethod)
		);
		assert!(PRECACHED.take().is_empty());
		WORLD.set(null_mut());
		DESCRIPTION.set(null_mut());
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

		unsafe {
			BfWrite::append(
				NonNull::new(buffer.cast()).unwrap(),
				bits.as_words(),
				bits.len(),
			)
		}
	}

	/// Any entity, as the base entity of every [`player_unknown`].
	unsafe extern "C" fn unknown_base_entity(
		unknown: *mut sys::IServerUnknown,
	) -> *mut sys::CBaseEntity {
		unknown.cast()
	}

	/// A vector over `values`. Both element pointers come from one
	/// `as_mut_ptr` call, since a second call would invalidate the first.
	fn vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
		let len = c_int::try_from(values.len()).unwrap();
		let elements = values.as_mut_ptr();

		sys::CUtlVector {
			_phantom_0: PhantomData,
			_phantom_1: PhantomData,
			m_Memory: sys::CUtlMemory {
				_phantom_0: PhantomData,
				m_pMemory: elements,
				m_nAllocationCount: len,
				m_nGrowSize: 0,
			},
			m_Size: len,
			m_pElements: elements,
		}
	}
}
