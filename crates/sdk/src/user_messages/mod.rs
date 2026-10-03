//! User messages: the game's messages to clients' HUDs and screen effects,
//! such as fades, shakes, and chat.
//!
//! The game registers each message by name, with its payload's size if
//! fixed, and the engine sends a message to each client a [`Recipients`]
//! names. [`messages`] has typed messages; [`RawUserMessage`] sends any other.
//!
//! [`net::messages::UserMessage`](crate::net::messages::UserMessage) sends a
//! user message to one client through its channel instead, without the checks
//! here.

pub mod messages;

use crate::bitbuf::{BitWriter, RawBfWrite};
use crate::edicts::Edict;
use crate::entities::Entity;
use crate::net::EncodeError;
use crate::net::messages::MAX_MESSAGE_DATA_BYTES;
use crate::server::{InterfaceError, Server};
use sdk_raw::user_messages::RecipientFilter;
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_int};
use std::ptr::NonNull;

/// Any user message, from its name and payload.
///
/// Its size is still checked against the size the game registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawUserMessage<'a> {
	/// The name the game registered the message under.
	pub name: &'a CStr,

	/// The payload, sent as written.
	pub data: &'a BitWriter,
}

impl UserMessage for RawUserMessage<'_> {
	fn name(&self) -> &CStr {
		self.name
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_bits(self.data);
		Ok(())
	}
}

/// The clients a user message or sound goes to, by player index, for the
/// engine's `IRecipientFilter`.
///
/// The engine skips indices no client in the game owns, and fake clients.
///
/// [`EngineSound::emit_sound`](crate::interfaces::EngineSound::emit_sound)
/// sends sounds to them too.
#[doc(alias = "IRecipientFilter")]
#[doc(alias = "CRecipientFilter")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recipients {
	players: Vec<c_int>,
	reliable: bool,
}

impl Recipients {
	/// No clients, sent unreliably.
	pub const fn new() -> Self {
		Self {
			players: Vec::new(),
			reliable: false,
		}
	}

	/// Every player slot in use: each up to the client limit whose edict has
	/// an entity.
	///
	/// Fails if the engine or the player info manager is missing.
	#[doc(alias = "AddAllPlayers")]
	pub fn all_players(server: Server<'_>) -> Result<Self, InterfaceError> {
		let engine = server.valve_engine()?;
		let max_clients = server
			.player_info_manager()?
			.global_vars()
			.map_or(0, |globals| globals.max_clients());
		let mut recipients = Self::new();

		for index in 1..=max_clients {
			if let Some(edict) = engine
				.edict_of_index(index)
				.filter(|edict| edict.entity().is_some())
			{
				recipients.add(edict);
			}
		}

		Ok(recipients)
	}

	/// One client.
	#[doc(alias = "CSingleUserRecipientFilter")]
	pub fn player(client: Edict<'_>) -> Self {
		let mut recipients = Self::new();

		recipients.add(client);
		recipients
	}

	/// Every player on a team, by the team index the game reports for the
	/// player of each slot up to the client limit, such as 2 and 3 for TF2's
	/// RED and BLU.
	///
	/// A client that is still connecting has no player yet, so it is left out
	/// until the game creates one. A player that has not joined a team yet is
	/// on `TEAM_UNASSIGNED` (0), and spectators are on `TEAM_SPECTATOR` (1),
	/// from `game/shared/shareddefs.h`. Bots on the team are included, as in
	/// [`all_players`](Self::all_players), though the engine sends them
	/// nothing. Unlike the game's `CTeamRecipientFilter`, spectators watching
	/// a player on the team are not added.
	///
	/// Fails if the engine or the player info manager is missing.
	#[doc(alias = "CTeamRecipientFilter")]
	pub fn team(server: Server<'_>, team: c_int) -> Result<Self, InterfaceError> {
		let engine = server.valve_engine()?;
		let players = server.player_info_manager()?;
		let max_clients = players
			.global_vars()
			.map_or(0, |globals| globals.max_clients());
		let mut recipients = Self::new();

		for index in 1..=max_clients {
			if let Some(edict) = engine.edict_of_index(index)
				&& players
					.player_info(edict)
					.is_some_and(|player| player.team() == team)
			{
				recipients.add(edict);
			}
		}

		Ok(recipients)
	}

	/// Adds a client, once.
	#[doc(alias = "AddRecipient")]
	pub fn add(&mut self, client: Edict<'_>) {
		let index = client.index();

		if !self.players.contains(&index) {
			self.players.push(index);
		}
	}

	/// The `IRecipientFilter` the engine calls through its vtable while it
	/// sends a message or sound, which lends it these recipients.
	pub(crate) const fn filter(&self) -> RecipientFilter<'_> {
		RecipientFilter::new(self.players.as_slice(), self.reliable)
	}

	/// Whether no client was added.
	pub fn is_empty(&self) -> bool {
		self.players.is_empty()
	}

	/// The number of clients added, each counted once.
	#[doc(alias = "GetRecipientCount")]
	pub fn len(&self) -> usize {
		self.players.len()
	}

	/// The player index of each client, in the order they were added.
	#[doc(alias = "GetRecipientIndex")]
	pub fn players(&self) -> &[c_int] {
		&self.players
	}

	/// Sends the message in each client's reliable stream, in order with the
	/// other reliable messages, instead of dropping it when the packet is full.
	#[doc(alias = "MakeReliable")]
	pub fn reliable(mut self) -> Self {
		self.reliable = true;
		self
	}

	/// Removes a client, if it was added, keeping the others in order.
	#[doc(alias = "RemoveRecipient")]
	pub fn remove(&mut self, client: Edict<'_>) {
		let index = client.index();

		self.players.retain(|&player| player != index);
	}
}

/// A user message: its registered name and its payload.
pub trait UserMessage {
	/// The name the game registered the message under, such as `Fade`.
	fn name(&self) -> &CStr;

	/// Writes the payload, as the game's own sender does.
	///
	/// An error stops [`send`] before the engine begins the message.
	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError>;
}

/// Why a user message could not be sent.
#[derive(Debug, thiserror::Error)]
pub enum UserMessageError {
	/// An interface needed to send the message is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The message could not write its payload, or the payload is longer than
	/// [`MAX_MESSAGE_DATA_BYTES`].
	#[error(transparent)]
	Encode(#[from] EncodeError),

	/// The game registered no message by this name, or registered it with a
	/// type above 255, which no net message can carry.
	#[error("the game registered no user message named {0:?}")]
	Unknown(CString),

	/// The payload's size differs from the fixed size the game registered.
	#[error("the game registered {name:?} with {expected} bytes, but the payload has {bytes}")]
	WrongSize {
		/// The message's name.
		name: CString,

		/// The size the game registered, in bytes.
		expected: usize,

		/// The payload's size, in bytes.
		bytes: usize,
	},

	/// The engine gave no buffer to write the payload into.
	#[error("the engine did not start the message")]
	NotStarted,

	/// The payload did not fit in the engine's buffer, so the message was
	/// dropped rather than sent.
	#[error("the payload did not fit in the engine's buffer")]
	Overflow,

	/// The entity has no edict or server class, so no client knows it.
	#[error("the entity is not networked, so no client knows it")]
	NotNetworked,
}

/// Copies a payload into the buffer a message began with, then ends it.
fn finish(
	engine: *mut sys::IVEngineServer,
	buffer: *mut sys::bf_write,
	data: &BitWriter,
) -> Result<(), UserMessageError> {
	let buffer = NonNull::new(buffer).ok_or(UserMessageError::NotStarted)?;

	// SAFETY: The engine's message buffer stays allocated, and only this
	// writes to it, until `MessageEnd`.
	let copied = unsafe { RawBfWrite::append(buffer.cast(), data) };

	// A begun message must end, or the engine refuses the next. An overflowed
	// one is dropped rather than sent.
	//
	// SAFETY: A message is in progress.
	unsafe { vcall!(engine => IVEngineServer_MessageEnd()) };

	match copied {
		true => Ok(()),
		false => Err(UserMessageError::Overflow),
	}
}

/// Sends a user message to every client `recipients` names, as the game's
/// `UserMessageBegin` and `MessageEnd` do.
///
/// The payload must have the size the game registered the message with, if
/// fixed, and at most [`MAX_MESSAGE_DATA_BYTES`] otherwise.
#[doc(alias = "UserMessageBegin")]
#[doc(alias = "MessageEnd")]
pub fn send(
	server: Server<'_>,
	recipients: &Recipients,
	message: &(impl UserMessage + ?Sized),
) -> Result<(), UserMessageError> {
	let name = message.name();
	let registered = server
		.server_game_dll()?
		.user_messages()
		.find(|registered| registered.name.as_c_str() == name)
		.ok_or_else(|| UserMessageError::Unknown(name.to_owned()))?;
	let mut data = BitWriter::new();

	message.write(&mut data)?;

	let bytes = data.byte_len();

	match registered.size {
		Some(expected) if bytes != expected => {
			return Err(UserMessageError::WrongSize {
				name: name.to_owned(),
				expected,
				bytes,
			});
		}

		_ => EncodeError::check_len("user message data", bytes, MAX_MESSAGE_DATA_BYTES)?,
	}

	// User message types fit in a byte, as the net message carrying them
	// sends them in one.
	let id = c_int::try_from(registered.index)
		.ok()
		.filter(|&id| id <= u8::MAX.into())
		.ok_or_else(|| UserMessageError::Unknown(name.to_owned()))?;
	let engine = server.valve_engine()?;
	let filter = recipients.filter();

	// SAFETY: `Server::new` guarantees the interface is live. The filter
	// outlives the message, which ends before this returns, and nothing runs
	// between its beginning and end but the payload's copy.
	let buffer =
		unsafe { vcall!(engine.as_ptr() => IVEngineServer_UserMessageBegin(filter.as_raw(), id)) };

	finish(engine.as_ptr(), buffer, &data)
}

/// Sends a message to an entity's client-side object, as the game's
/// `EntityMessageBegin` and `MessageEnd` do, to every client that knows the
/// entity.
///
/// The payload must be at most [`MAX_MESSAGE_DATA_BYTES`] bytes.
#[doc(alias = "EntityMessageBegin")]
pub fn send_entity_message(
	server: Server<'_>,
	entity: Entity<'_>,
	reliable: bool,
	data: &BitWriter,
) -> Result<(), UserMessageError> {
	EncodeError::check_len(
		"entity message data",
		data.byte_len(),
		MAX_MESSAGE_DATA_BYTES,
	)?;

	let edict = entity.edict().ok_or(UserMessageError::NotNetworked)?;
	let class = entity
		.server_class()
		.ok_or(UserMessageError::NotNetworked)?;
	let engine = server.valve_engine()?;

	// SAFETY: As for `send`, with a networked entity's live class.
	let buffer = unsafe {
		vcall!(engine.as_ptr() => IVEngineServer_EntityMessageBegin(edict.index(), class.as_ptr(), reliable))
	};

	finish(engine.as_ptr(), buffer, data)
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;

	/// Recipients with exactly these player indices, in order.
	pub(crate) fn recipients(players: &[c_int], reliable: bool) -> Recipients {
		Recipients {
			players: players.to_vec(),
			reliable,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::edicts::test_support::mock_edict;
	use crate::interfaces::{PlayerInfoManager, ValveEngine};
	use crate::server::Module;
	use crate::server::test_support::{export, mock_server};
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::mem::MaybeUninit;
	use std::ptr::null_mut;

	thread_local! {
		static GLOBALS: Cell<*mut sys::CGlobalVars> = const { Cell::new(null_mut()) };
		static TABLE: Cell<(*mut sys::edict_t, usize)> = const { Cell::new((null_mut(), 0)) };
		static PLAYERS: Cell<*const [*mut MockPlayer]> =
			const { Cell::new(std::ptr::slice_from_raw_parts(std::ptr::null(), 0)) };
	}

	/// A player's `IPlayerInfo`, which reports the team it was made with.
	#[repr(C)]
	struct MockPlayer {
		interface: sys::IPlayerInfo,
		team: c_int,
	}

	#[test]
	fn clients_are_added_and_removed_once() {
		let mut table = [
			mock_edict(0, false),
			mock_edict(1, false),
			mock_edict(2, false),
		];
		let base = table.as_mut_ptr();
		// SAFETY: The table outlives every handle.
		let edict = |slot: usize| unsafe { Edict::from_raw(NonNull::new(base.add(slot)).unwrap()) };
		let mut recipients = Recipients::new();

		recipients.add(edict(2));
		recipients.add(edict(1));
		recipients.add(edict(2));
		assert_eq!(recipients.players(), [2, 1]);
		assert_eq!(recipients.len(), 2);

		recipients.remove(edict(0));
		assert_eq!(recipients.players(), [2, 1]);

		recipients.remove(edict(2));
		assert_eq!(recipients.players(), [1]);

		recipients.remove(edict(1));
		assert!(recipients.is_empty());
		assert_eq!(Recipients::player(edict(1)).players(), [1]);
	}

	unsafe extern "C" fn edict_of_index(
		_: *mut sys::IVEngineServer,
		index: c_int,
	) -> *mut sys::edict_t {
		let (table, len) = TABLE.get();

		match usize::try_from(index) {
			// SAFETY: The slot lies within the table.
			Ok(slot) if slot < len => unsafe { table.add(slot) },

			_ => null_mut(),
		}
	}

	unsafe extern "C" fn global_vars(_: *mut sys::IPlayerInfoManager) -> *mut sys::CGlobalVars {
		GLOBALS.get()
	}

	unsafe extern "C" fn player_info(
		_: *mut sys::IPlayerInfoManager,
		edict: *mut sys::edict_t,
	) -> *mut sys::IPlayerInfo {
		// SAFETY: The wrapper passes an edict of the mock table, and the players
		// are leaked.
		unsafe {
			let index = (*edict)._base.m_EdictIndex;

			usize::try_from(index)
				.ok()
				.and_then(|index| (&*PLAYERS.get()).get(index))
				.map_or(null_mut(), |&player| player.cast())
		}
	}

	unsafe extern "C" fn team_index(this: *mut sys::IPlayerInfo) -> c_int {
		// SAFETY: Every player info the mock returns is a `MockPlayer`.
		unsafe { (*this.cast::<MockPlayer>()).team }
	}

	#[test]
	fn teams_are_the_players_reporting_them() {
		// SAFETY: The vtables hold only function pointers.
		let (player_vtable, engine_vtable, manager_vtable) = unsafe {
			(
				mock_vtable::<sys::IPlayerInfo__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| (&raw mut (*vtable).IPlayerInfo_GetTeamIndex).write(team_index),
				),
				mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
					},
				),
				mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
						(&raw mut (*vtable).IPlayerInfoManager_GetPlayerInfo).write(player_info);
					},
				),
			)
		};
		let player = |team| {
			Box::into_raw(Box::new(MockPlayer {
				interface: sys::IPlayerInfo {
					vtable_: &raw const *player_vtable,
				},
				team,
			}))
		};

		// Slot 3 is a client still connecting, which has no player, slot 5 is
		// free, and slot 6 lies past the client limit.
		let players = vec![
			null_mut(),
			player(2),
			player(3),
			null_mut(),
			player(2),
			player(2),
			player(2),
		];
		let mut table = [0, 1, 2, 3, 4, 5, 6].map(|index| mock_edict(index, index == 5));
		let mut globals = MaybeUninit::<sys::CGlobalVars>::zeroed();

		// SAFETY: The globals are zeroed, which is valid for every field.
		unsafe { (&raw mut (*globals.as_mut_ptr())._base.maxClients).write(5) };
		GLOBALS.set(globals.as_mut_ptr());
		TABLE.set((table.as_mut_ptr(), table.len()));
		PLAYERS.set(Vec::leak(players));

		let mut engine = sys::IVEngineServer {
			vtable_: &raw const *engine_vtable,
		};
		let mut manager = sys::IPlayerInfoManager {
			vtable_: &raw const *manager_vtable,
		};

		export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
		export(
			Module::GameServer,
			PlayerInfoManager::VERSION,
			&raw mut manager,
		);

		let scope = ();
		let server = mock_server(&scope);
		let team = |team| Recipients::team(server, team).unwrap();

		assert_eq!(team(2).players(), [1, 4]);
		assert_eq!(team(3).players(), [2]);
		assert!(team(0).is_empty());
		assert!(!team(2).reliable);
	}
}
