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

use crate::abi::CppDestructors;
use crate::bitbuf::{BitWriter, RawBfWrite};
use crate::edicts::Edict;
use crate::entities::Entity;
use crate::ffi::vcall;
use crate::net::EncodeError;
use crate::net::messages::MAX_MESSAGE_DATA_BYTES;
use crate::server::{InterfaceError, Server};
use std::ffi::{CStr, c_int};
use std::mem::offset_of;
use std::ptr::NonNull;

/// A user message: its registered name and its payload.
pub trait UserMessage {
	/// The name the game registered the message under, such as `Fade`.
	fn name(&self) -> &CStr;

	/// Writes the payload, as the game's own sender does.
	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError>;
}

/// Any user message, from its name and payload.
///
/// Its size is still checked against the size the game registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawUserMessage<'a> {
	pub name: &'a CStr,
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

/// The clients a user message goes to, by player index, for the engine's
/// `IRecipientFilter`.
///
/// The engine skips indices no client in the game owns, and fake clients.
#[doc(alias = "IRecipientFilter")]
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

	/// One client.
	pub fn player(client: Edict<'_>) -> Self {
		let mut recipients = Self::new();

		recipients.add(client);
		recipients
	}

	/// Every player slot in use.
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

	/// Sends the message in each client's reliable stream, in order with the
	/// other reliable messages, instead of dropping it when the packet is full.
	pub fn reliable(mut self) -> Self {
		self.reliable = true;
		self
	}

	/// Adds a client, once.
	pub fn add(&mut self, client: Edict<'_>) {
		let index = client.index();

		if !self.players.contains(&index) {
			self.players.push(index);
		}
	}

	pub fn len(&self) -> usize {
		self.players.len()
	}

	pub fn is_empty(&self) -> bool {
		self.players.is_empty()
	}
}

/// A [`Recipients`] with the layout of an `IRecipientFilter`, which the engine
/// calls through its vtable while it sends a message.
#[repr(C)]
struct RecipientFilter<'a> {
	vtable: &'static RecipientFilterVtable,
	recipients: &'a Recipients,
}

#[repr(C)]
struct RecipientFilterVtable {
	destructor: CppDestructors,
	is_reliable: unsafe extern "C" fn(this: *const sys::IRecipientFilter) -> bool,
	is_init_message: unsafe extern "C" fn(this: *const sys::IRecipientFilter) -> bool,
	recipient_count: unsafe extern "C" fn(this: *const sys::IRecipientFilter) -> c_int,
	recipient_index: unsafe extern "C" fn(this: *const sys::IRecipientFilter, slot: c_int) -> c_int,
}

const _: () = {
	assert!(
		offset_of!(RecipientFilterVtable, is_reliable)
			== offset_of!(
				sys::IRecipientFilter__bindgen_vtable,
				IRecipientFilter_IsReliable
			)
	);
	assert!(
		offset_of!(RecipientFilterVtable, recipient_index)
			== offset_of!(
				sys::IRecipientFilter__bindgen_vtable,
				IRecipientFilter_GetRecipientIndex
			)
	);
	assert!(
		size_of::<RecipientFilterVtable>() == size_of::<sys::IRecipientFilter__bindgen_vtable>()
	);
};

impl<'a> RecipientFilter<'a> {
	/// The engine never deletes a filter it is given.
	const VTABLE: RecipientFilterVtable = RecipientFilterVtable {
		destructor: CppDestructors::new_noop(),
		is_reliable: Self::is_reliable,
		is_init_message: Self::is_init_message,
		recipient_count: Self::recipient_count,
		recipient_index: Self::recipient_index,
	};

	fn new(recipients: &'a Recipients) -> Self {
		Self {
			vtable: &Self::VTABLE,
			recipients,
		}
	}

	fn as_raw(&self) -> *mut sys::IRecipientFilter {
		// The engine only reads through the pointer.
		(&raw const *self).cast_mut().cast()
	}

	/// # Safety
	///
	/// `this` must be a live filter made by [`RecipientFilter::new`].
	unsafe fn recipients<'b>(this: *const sys::IRecipientFilter) -> &'b Recipients {
		// SAFETY: As the caller promises. The filter only lends out what it
		// borrows, for the call.
		unsafe { (*this.cast::<RecipientFilter<'b>>()).recipients }
	}

	unsafe extern "C" fn is_reliable(this: *const sys::IRecipientFilter) -> bool {
		// SAFETY: The engine calls it on the filter it was given.
		unsafe { Self::recipients(this) }.reliable
	}

	unsafe extern "C" fn is_init_message(_: *const sys::IRecipientFilter) -> bool {
		false
	}

	unsafe extern "C" fn recipient_count(this: *const sys::IRecipientFilter) -> c_int {
		// SAFETY: As for `is_reliable`.
		let players = &unsafe { Self::recipients(this) }.players;

		c_int::try_from(players.len()).unwrap_or(c_int::MAX)
	}

	unsafe extern "C" fn recipient_index(this: *const sys::IRecipientFilter, slot: c_int) -> c_int {
		// SAFETY: As for `is_reliable`.
		let players = &unsafe { Self::recipients(this) }.players;

		usize::try_from(slot)
			.ok()
			.and_then(|slot| players.get(slot))
			.copied()
			.unwrap_or(-1)
	}
}

/// Why a user message could not be sent.
#[derive(Debug, thiserror::Error)]
pub enum UserMessageError {
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	#[error(transparent)]
	Encode(#[from] EncodeError),

	#[error("the game registered no user message named {0:?}")]
	Unknown(std::ffi::CString),

	#[error("the game registered {name:?} with {expected} bytes, but the payload has {bytes}")]
	WrongSize {
		name: std::ffi::CString,
		expected: usize,
		bytes: usize,
	},

	#[error("the engine did not start the message")]
	NotStarted,

	#[error("the payload did not fit in the engine's buffer")]
	Overflow,

	#[error("the entity is not networked, so no client knows it")]
	NotNetworked,
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

	match registered.size {
		Some(expected) if data.byte_len() != expected => {
			return Err(UserMessageError::WrongSize {
				name: name.to_owned(),
				expected,
				bytes: data.byte_len(),
			});
		}
		_ => EncodeError::check_len("user message data", data.byte_len(), MAX_MESSAGE_DATA_BYTES)?,
	}

	// User message types fit in a byte, as the net message carrying them
	// sends them in one.
	let id = c_int::try_from(registered.index)
		.ok()
		.filter(|&id| id <= u8::MAX.into())
		.ok_or_else(|| UserMessageError::Unknown(name.to_owned()))?;
	let engine = server.valve_engine()?;
	let filter = RecipientFilter::new(recipients);

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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn filters_report_their_recipients_through_the_vtable() {
		let recipients = Recipients {
			players: vec![3, 7],
			reliable: true,
		};
		let filter = RecipientFilter::new(&recipients);
		let raw = filter.as_raw();

		unsafe {
			let vtable = (*raw).vtable_;

			assert!(((*vtable).IRecipientFilter_IsReliable)(raw));
			assert!(!((*vtable).IRecipientFilter_IsInitMessage)(raw));
			assert_eq!(((*vtable).IRecipientFilter_GetRecipientCount)(raw), 2);
			assert_eq!(((*vtable).IRecipientFilter_GetRecipientIndex)(raw, 1), 7);
			assert_eq!(((*vtable).IRecipientFilter_GetRecipientIndex)(raw, 2), -1);
			assert_eq!(((*vtable).IRecipientFilter_GetRecipientIndex)(raw, -1), -1);
		}
	}
}
