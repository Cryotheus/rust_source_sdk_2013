//! The Steam Game Coordinator (GC), Valve's backend for items and matchmaking,
//! as the game server reaches it through Steam.
//!
//! [`Server::game_coordinator`] returns the game server's
//! `ISteamGameCoordinator`, the very object the game sends its GC messages
//! through, so that a plugin can hook what the game sends. It exists only
//! while the Steam game server runs: from just before the first map starts,
//! until the server quits or a `map` command starts a new game, which also
//! restarts Steam. A plugin loaded with the server, before any map, finds
//! none, and can look again when a level starts.
//!
//! # Not offered
//!
//! - Reading messages: a message the GC sends is taken from the queue by
//!   whoever retrieves it first, so retrieving one would steal it from the
//!   game.
//! - Sending messages: the GC expects messages framed and sequenced by the
//!   game's GC client, and a malformed one can disrupt the game server's GC
//!   session. Nothing needs it yet.
//!
//! # Lifetime of Steam's client library
//!
//! Steam's client library, which implements the coordinator, is freed when the
//! Steam game server shuts down, and loaded again when it restarts. A hook on
//! the coordinator's vtable must keep the library loaded, through
//! [`GameCoordinator::pin_steam_client`], or it would be lost, or patch freed
//! memory, at the next restart.

use crate::{NotThreadSafe, Server};
use sdk_raw::steam as raw;
use sdk_raw::util::{self, PinnedModule, rtti};
use std::ffi::{CStr, OsStr};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::ptr::NonNull;

/// The game server's connection to the GC, `ISteamGameCoordinator`, for the
/// scope of one callback.
///
/// Steam frees the object when the game server shuts down, and creates another
/// when it starts again, so a handle is never kept past its callback.
#[doc(alias("ISteamGameCoordinator"))]
#[derive(Debug, Clone, Copy)]
pub struct GameCoordinator<'s> {
	ptr: NonNull<raw::ISteamGameCoordinator>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> GameCoordinator<'s> {
	/// The `ISteamGameCoordinator` object, such as for a hook on its class.
	pub fn as_ptr(self) -> *mut raw::ISteamGameCoordinator {
		self.ptr.as_ptr()
	}

	/// The decorated name of the object's class, from run-time type
	/// information, such as `.?AVCAdapterSteamGameCoordinator001@@` on
	/// Windows, or `None` if Steam's client library has none.
	///
	/// This is informational: whether the Linux client library carries
	/// run-time type information is unverified.
	pub fn class_name(self) -> Option<&'s CStr> {
		// SAFETY: The coordinator is a live polymorphic object for 's, whose
		// vtable Steam's client library, loaded for as long as the object
		// lives, emitted with run-time type information or none.
		unsafe { rtti::dynamic_type(self.ptr.as_ptr().cast()) }.map(|(_, name)| name)
	}

	/// Keeps the library that implements the coordinator, Steam's client
	/// library, loaded until the process exits, and returns it. Fails, without
	/// pinning, if the library cannot be found, and after pinning it, if it is
	/// not Steam's client library.
	///
	/// This is a commitment for the life of the process: Steam's client library
	/// is never unloaded again, even when the Steam game server restarts, and
	/// it then starts again in the library that stayed loaded. A hook on the
	/// coordinator's vtable needs it, as the vtable lies in the library, and
	/// hooking libraries keep vtable patches by address.
	pub fn pin_steam_client(self) -> Result<PinnedModule, SteamClientError> {
		// SAFETY: The coordinator is live for 's, and so is its vtable, which
		// lies in the library that implements it.
		let vtable = unsafe { self.ptr.as_ptr().cast::<usize>().read() };

		// SAFETY: As above, the library stays loaded for 's.
		let module = unsafe { util::pin_module(vtable) }?;

		if !is_steam_client(module.path().file_name()) {
			return Err(SteamClientError::NotSteamClient(module.path().to_owned()));
		}

		Ok(module)
	}
}

/// The result of a GC call, `EGCResults`.
#[doc(alias("EGCResults"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GcResult {
	/// The call succeeded: `k_EGCResultOK`.
	#[doc(alias("k_EGCResultOK"))]
	Ok = raw::GC_RESULT_OK,

	/// No message is waiting: `k_EGCResultNoMessage`.
	#[doc(alias("k_EGCResultNoMessage"))]
	NoMessage = raw::GC_RESULT_NO_MESSAGE,

	/// The destination cannot hold the message: `k_EGCResultBufferTooSmall`.
	#[doc(alias("k_EGCResultBufferTooSmall"))]
	BufferTooSmall = raw::GC_RESULT_BUFFER_TOO_SMALL,

	/// The game server is not logged on to Steam: `k_EGCResultNotLoggedOn`.
	#[doc(alias("k_EGCResultNotLoggedOn"))]
	NotLoggedOn = raw::GC_RESULT_NOT_LOGGED_ON,

	/// The message is malformed: `k_EGCResultInvalidMessage`.
	#[doc(alias("k_EGCResultInvalidMessage"))]
	InvalidMessage = raw::GC_RESULT_INVALID_MESSAGE,
}

impl GcResult {
	/// The `EGCResults` value.
	pub const fn as_raw(self) -> i32 {
		self as i32
	}
}

/// The type of a GC message, its `EMsg` id, without the flag that marks a
/// protobuf message.
#[doc(alias("MsgType_t", "EMsg"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageType(u32);

impl MessageType {
	/// `k_EMsgGCServerHello`: the game server's greeting to the GC, which it
	/// sends every 30 seconds or so while logged on to Steam but not yet
	/// welcomed.
	#[doc(alias("k_EMsgGCServerHello"))]
	pub const SERVER_HELLO: Self = Self::new(raw::SERVER_HELLO);

	/// `k_EMsgGCServerWelcome`: the GC's reply to [`Self::SERVER_HELLO`].
	#[doc(alias("k_EMsgGCServerWelcome"))]
	pub const SERVER_WELCOME: Self = Self::new(raw::SERVER_WELCOME);

	/// The type with the id `id`, ignoring the protobuf flag.
	pub const fn new(id: u32) -> Self {
		Self(id & !raw::PROTOBUF_FLAG)
	}

	/// The type of a message as sent, and whether it is a protobuf message.
	pub const fn from_wire(wire: u32) -> (Self, bool) {
		(Self::new(wire), wire & raw::PROTOBUF_FLAG != 0)
	}

	/// The message id.
	pub const fn id(self) -> u32 {
		self.0
	}
}

/// Why [`GameCoordinator::pin_steam_client`] failed.
#[derive(Debug, thiserror::Error)]
pub enum SteamClientError {
	/// The library holding the coordinator's vtable could not be found or
	/// pinned.
	#[error("could not pin the library implementing the game coordinator: {0}")]
	Module(#[from] util::Error),

	/// The coordinator's vtable lies in this library, which is not Steam's
	/// client library. It was pinned all the same.
	#[error("the game coordinator is implemented by {0:?}, not Steam's client library")]
	NotSteamClient(PathBuf),
}

impl<'s> Server<'s> {
	/// The game server's connection to the GC, or `None` while the Steam game
	/// server is not running, such as before the first map starts.
	///
	/// Steam creates the connection when the game server first asks for it, as
	/// the game does once logged on to Steam, and then returns the same object
	/// until it shuts down; see the [module documentation](crate::steam).
	#[doc(alias(
		"SteamGameCoordinator",
		"SteamInternal_FindOrCreateGameServerInterface"
	))]
	pub fn game_coordinator(&self) -> Option<GameCoordinator<'s>> {
		// SAFETY: `Server::new` puts this on the server's main thread, which
		// runs the Steam game server, while the engine is loaded, and with it
		// `steam_api`, which the engine imports.
		let ptr = unsafe { raw::game_server_coordinator() }?;

		Some(GameCoordinator {
			ptr,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}
}

/// Whether `name`, a library's file name, is Steam's client library's.
fn is_steam_client(name: Option<&OsStr>) -> bool {
	let Some(name) = name.and_then(OsStr::to_str) else {
		return false;
	};

	match cfg!(windows) {
		true => name.eq_ignore_ascii_case(raw::STEAM_CLIENT_LIBRARY),
		false => name == raw::STEAM_CLIENT_LIBRARY,
	}
}
