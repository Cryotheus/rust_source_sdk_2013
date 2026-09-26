//! The engine's server and its clients (`IServer` and `IClient`).

use crate::ffi::{NotThreadSafe, copy_cstr, vcall};
use crate::interfaces::ValveEngine;
use crate::net::NetChannel;
use crate::players::UserId;
use std::ffi::{CStr, CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The engine's game server (`IServer`), which owns a client object for every
/// player slot.
///
/// The engine keeps the server for as long as it runs, and its clients until
/// the player limit changes, which only happens between levels.
#[doc(alias = "IServer")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameServer<'s> {
	raw: NonNull<sys::IServer>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ValveEngine<'s> {
	/// The engine's game server.
	#[doc(alias = "GetIServer")]
	pub fn game_server(self) -> Option<GameServer<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let raw = NonNull::new(unsafe { vcall!(self.as_ptr() => IVEngineServer_GetIServer()) })?;

		Some(GameServer {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}
}

impl<'s> GameServer<'s> {
	pub const fn as_ptr(self) -> *mut sys::IServer {
		self.raw.as_ptr()
	}

	const fn as_const(self) -> *const sys::IServer {
		self.raw.as_ptr().cast_const()
	}

	/// The client in a player slot, from 0, whether anyone uses it or not.
	#[doc(alias = "GetClient")]
	pub fn client(self, slot: c_int) -> Option<GameClient<'s>> {
		if !(0..self.client_count()).contains(&slot) {
			return None;
		}

		// SAFETY: The engine's server lives as long as it runs, and the slot is
		// one of its clients.
		let raw = NonNull::new(unsafe { vcall!(self.as_ptr() => IServer_GetClient(slot)) })?;

		// SAFETY: The engine keeps its clients until the player limit changes,
		// between levels.
		Some(unsafe { GameClient::from_raw(raw) })
	}

	/// Every client, one per player slot.
	pub fn clients(self) -> impl Iterator<Item = GameClient<'s>> {
		(0..self.client_count()).filter_map(move |slot| self.client(slot))
	}

	/// The number of client objects, one per player slot.
	#[doc(alias = "GetClientCount")]
	pub fn client_count(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_ptr() => IServer_GetClientCount()) }
	}

	/// The most players the server takes.
	#[doc(alias = "GetMaxClients")]
	pub fn max_clients(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetMaxClients()) }
	}

	/// Connected clients, including fake ones.
	#[doc(alias = "GetNumClients")]
	pub fn connected_clients(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetNumClients()) }
	}

	/// Fake clients, such as bots.
	#[doc(alias = "GetNumFakeClients")]
	pub fn fake_clients(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetNumFakeClients()) }
	}

	/// SourceTV and replay proxies.
	#[doc(alias = "GetNumProxies")]
	pub fn proxies(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetNumProxies()) }
	}

	/// The server's current tick.
	#[doc(alias = "GetTick")]
	pub fn tick(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetTick()) }
	}

	/// The server's time, in seconds.
	#[doc(alias = "GetTime")]
	pub fn time(self) -> f32 {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetTime()) }
	}

	/// The server's name, as `hostname` sets it.
	#[doc(alias = "GetName")]
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `client`, and the name is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => IServer_GetName())) }
	}

	/// The name of the current map.
	#[doc(alias = "GetMapName")]
	pub fn map_name(self) -> Option<CString> {
		// SAFETY: As for `name`.
		unsafe { copy_cstr(vcall!(self.as_const() => IServer_GetMapName())) }
	}

	/// Levels the server has loaded since it started.
	#[doc(alias = "GetSpawnCount")]
	pub fn spawn_count(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetSpawnCount()) }
	}

	/// The UDP port the server listens on.
	#[doc(alias = "GetLocalUDPPort")]
	pub fn udp_port(self) -> c_int {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_GetLocalUDPPort()) }
	}

	/// Whether a level is running.
	#[doc(alias = "IsActive")]
	pub fn is_active(self) -> bool {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_IsActive()) }
	}

	/// Whether a level is loading.
	#[doc(alias = "IsLoading")]
	pub fn is_loading(self) -> bool {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_IsLoading()) }
	}

	#[doc(alias = "IsPaused")]
	pub fn is_paused(self) -> bool {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_IsPaused()) }
	}

	#[doc(alias = "IsDedicated")]
	pub fn is_dedicated(self) -> bool {
		// SAFETY: As for `client`.
		unsafe { vcall!(self.as_const() => IServer_IsDedicated()) }
	}
}

/// The engine's object for one player slot (`IClient`), whether a player
/// uses it or not.
#[doc(alias = "IClient")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameClient<'s> {
	raw: NonNull<sys::IClient>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> GameClient<'s> {
	/// # Safety
	///
	/// `raw` must be one of the engine's clients, which stays allocated for
	/// `'s`.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::IClient>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	pub const fn as_ptr(self) -> *mut sys::IClient {
		self.raw.as_ptr()
	}

	const fn as_const(self) -> *const sys::IClient {
		self.raw.as_ptr().cast_const()
	}

	/// The client's player slot, from 0.
	#[doc(alias = "GetPlayerSlot")]
	pub fn slot(self) -> c_int {
		// SAFETY: The client is live for `'s`.
		unsafe { vcall!(self.as_const() => IClient_GetPlayerSlot()) }
	}

	/// The entity index of the client's player, which is its slot plus 1.
	pub fn entity_index(self) -> c_int {
		self.slot() + 1
	}

	/// The player's user ID, or `None` for an empty slot.
	#[doc(alias = "GetUserID")]
	pub fn user_id(self) -> Option<UserId> {
		// SAFETY: As for `slot`.
		UserId::from_raw(unsafe { vcall!(self.as_const() => IClient_GetUserID()) }).ok()
	}

	/// The player's name.
	#[doc(alias = "GetClientName")]
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `slot`, and the name is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => IClient_GetClientName())) }
	}

	/// The player's network ID, such as a rendered Steam ID or `BOT`.
	#[doc(alias = "GetNetworkIDString")]
	pub fn network_id(self) -> Option<CString> {
		// SAFETY: As for `name`.
		unsafe { copy_cstr(vcall!(self.as_const() => IClient_GetNetworkIDString())) }
	}

	/// The value the client reported for one of its user settings, the
	/// console variables marked `FCVAR_USERINFO`. Unknown settings are empty.
	#[doc(alias = "GetUserSetting")]
	pub fn user_setting(self, name: &CStr) -> Option<CString> {
		// SAFETY: As for `name`.
		unsafe { copy_cstr(vcall!(self.as_const() => IClient_GetUserSetting(name.as_ptr()))) }
	}

	/// The client's channel, or `None` for fake clients and empty slots.
	#[doc(alias = "GetNetChannel")]
	pub fn net_channel(self) -> Option<NetChannel<'s>> {
		// SAFETY: As for `slot`.
		let channel = NonNull::new(unsafe { vcall!(self.as_ptr() => IClient_GetNetChannel()) })?;

		// SAFETY: A client's channel is freed only when it disconnects, which
		// nothing safe causes during `'s`.
		Some(unsafe { NetChannel::from_raw(channel) })
	}

	/// The rate, in bytes per second, the client asked to receive at most.
	#[doc(alias = "GetRate")]
	pub fn rate(self) -> c_int {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_GetRate()) }
	}

	/// The snapshots per second the client asked for.
	#[doc(alias = "GetUpdateRate")]
	pub fn update_rate(self) -> c_int {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_GetUpdateRate()) }
	}

	/// Whether a player has connected to the slot, even if not yet spawned.
	#[doc(alias = "IsConnected")]
	pub fn is_connected(self) -> bool {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_IsConnected()) }
	}

	/// Whether the player has spawned.
	#[doc(alias = "IsSpawned")]
	pub fn is_spawned(self) -> bool {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_IsSpawned()) }
	}

	/// Whether the player is fully in the game.
	#[doc(alias = "IsActive")]
	pub fn is_active(self) -> bool {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_IsActive()) }
	}

	/// Whether the player is a bot or proxy rather than a remote client.
	#[doc(alias = "IsFakeClient")]
	pub fn is_fake(self) -> bool {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_IsFakeClient()) }
	}

	/// Whether the client is SourceTV.
	#[doc(alias = "IsHLTV")]
	pub fn is_hltv(self) -> bool {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_IsHLTV()) }
	}

	/// Whether the client hears the voice of the player in `speaker`'s slot.
	#[doc(alias = "IsHearingClient")]
	pub fn is_hearing(self, speaker: GameClient<'_>) -> bool {
		// SAFETY: As for `slot`.
		unsafe { vcall!(self.as_const() => IClient_IsHearingClient(speaker.slot())) }
	}

	/// Prints a line to the client's console.
	#[doc(alias = "ClientPrintf")]
	pub fn print(self, message: &CStr) {
		// SAFETY: As for `slot`, with a format that prints its one argument.
		unsafe { vcall!(self.as_ptr() => IClient_ClientPrintf(c"%s".as_ptr(), message.as_ptr())) };
	}

	/// Runs a command as though the client sent it, through the engine's own
	/// commands and then the game's. Returns whether a command handled it.
	///
	/// # Safety
	///
	/// The command must not disconnect the client or free entities
	/// immediately, as `disconnect` and some game commands do.
	#[doc(alias = "ExecuteStringCommand")]
	pub unsafe fn execute_string_command(self, command: &CStr) -> bool {
		// SAFETY: As for `slot`, and the caller vouches for the command.
		unsafe { vcall!(self.as_ptr() => IClient_ExecuteStringCommand(command.as_ptr())) }
	}

	/// Disconnects the client, showing it `reason`.
	///
	/// # Safety
	///
	/// The engine frees the client's channel and its player entity at once, so
	/// nothing may use either during the rest of `'s`, and no other code may be
	/// using them while this runs.
	#[doc(alias = "Disconnect")]
	pub unsafe fn disconnect(self, reason: &CStr) {
		// SAFETY: As for `print`, and the caller vouches for the rest.
		unsafe { vcall!(self.as_ptr() => IClient_Disconnect(c"%s".as_ptr(), reason.as_ptr())) };
	}
}
