//! The game server's half of the Steamworks API, as far as the game uses it
//! to reach the Steam Game Coordinator (GC): `steam_api`'s exports, found at
//! runtime so that no Steam library is linked, `ISteamClient` and
//! `ISteamGameCoordinator`'s vtable slots, and the values of their headers.
//!
//! # Libraries
//!
//! `steam_api64.dll` (`libsteam_api.so` on Linux) is the Steamworks API the
//! engine and the game link against. Both import it statically, so it is
//! loaded before any plugin and unloaded after every one. It loads Steam's
//! client library, `steamclient64.dll` (`steamclient.so`), when the engine
//! starts the Steam game server, and frees it again when the engine shuts the
//! game server down, which it does when a `map` command starts a new game, when
//! `sv_lan` changes, and when the server quits. The interfaces Steam hands out
//! live in the client library, so their objects and vtables can be freed while
//! the server runs; see [`crate::util::pin_module`].
//!
//! # The game's coordinator
//!
//! The game reaches the GC through `ISteamClient::GetISteamGenericInterface`
//! with the game server's Steam user and pipe, and the version
//! [`STEAM_GAME_COORDINATOR_INTERFACE_VERSION`].
//! `SteamInternal_FindOrCreateGameServerInterface` makes the same call with
//! the same user and pipe, and the client library keeps the interfaces it
//! creates per user, pipe and version, so [`game_server_coordinator`] returns
//! the very object the game sends its messages through. The client library
//! frees it when the game server's pipe or user is released, so it must not
//! be kept past the callback it was found in.
//!
//! # Threads
//!
//! `steam_api` keeps the game server's user, pipe and `ISteamClient` in
//! globals it reads and writes without locking. Call this module's functions
//! only on the thread that runs the Steam game server: the server's main
//! thread.

use crate::util::loaded_symbol;
use std::ffi::{CStr, c_char, c_void};
use std::mem::offset_of;
use std::ptr::NonNull;
use std::sync::OnceLock;

/// `EGCResults`, the result of the coordinator's calls: one of the
/// `GC_RESULT_*` values, from Steamworks' `isteamgamecoordinator.h`.
pub type EGCResults = i32;

/// `SteamInternal_FindOrCreateGameServerInterface`, from
/// `public/steam/steam_api_internal.h`: the interface `version` of the game
/// server's `user`, through the game server's `ISteamClient` and pipe, or null
/// while the game server is not running.
#[doc(alias("SteamInternal_FindOrCreateGameServerInterface"))]
pub type FindOrCreateGameServerInterfaceFn =
	unsafe extern "C" fn(user: HSteamUser, version: *const c_char) -> *mut c_void;

/// `SteamGameServer_GetHSteamPipe`, which returns the game server's pipe to
/// the Steam client, or 0 while the game server is not running.
#[doc(alias("SteamGameServer_GetHSteamPipe"))]
pub type GetHSteamPipeFn = unsafe extern "C" fn() -> HSteamPipe;

/// `SteamGameServer_GetHSteamUser`, which returns the game server's Steam
/// user: 0 before the game server first starts, and left unchanged by
/// `SteamGameServer_Shutdown`, so it is not a sign that the game server runs;
/// [`GetHSteamPipeFn`]'s pipe, which shutdown resets to 0, is.
#[doc(alias("SteamGameServer_GetHSteamUser"))]
pub type GetHSteamUserFn = unsafe extern "C" fn() -> HSteamUser;

/// `ISteamClient::GetISteamGenericInterface`, from
/// `public/steam/isteamclient.h`: the interface `version` of `user`, through
/// `pipe`, or null.
#[doc(alias("GetISteamGenericInterface"))]
pub type GetISteamGenericInterfaceFn = unsafe extern "C" fn(
	this: *mut ISteamClient,
	user: HSteamUser,
	pipe: HSteamPipe,
	version: *const c_char,
) -> *mut c_void;

/// `HSteamPipe`, a handle to a connection to the Steam client; 0 is none.
pub type HSteamPipe = i32;

/// `HSteamUser`, a handle to a Steam user of a pipe; 0 is none.
pub type HSteamUser = i32;

/// `ISteamGameCoordinator::IsMessageAvailable`: whether a message from the GC
/// is waiting, and if so its size.
#[doc(alias("IsMessageAvailable"))]
pub type IsMessageAvailableFn =
	unsafe extern "C" fn(this: *mut ISteamGameCoordinator, size: *mut u32) -> bool;

/// `ISteamGameCoordinator::RetrieveMessage`: takes the waiting message from the
/// GC, copying its type, and its body into `destination` if it fits in
/// `capacity` bytes. A message once retrieved is gone, so whoever calls this
/// takes it from the game.
#[doc(alias("RetrieveMessage"))]
pub type RetrieveMessageFn = unsafe extern "C" fn(
	this: *mut ISteamGameCoordinator,
	message_type: *mut u32,
	destination: *mut c_void,
	capacity: u32,
	size: *mut u32,
) -> EGCResults;

/// `ISteamGameCoordinator::SendMessage`: sends the GC a message of
/// `message_type`, whose body is the `len` bytes at `data`. Protobuf messages
/// carry [`PROTOBUF_FLAG`] in their type.
#[doc(alias("SendMessage"))]
pub type SendMessageFn = unsafe extern "C" fn(
	this: *mut ISteamGameCoordinator,
	message_type: u32,
	data: *const c_void,
	len: u32,
) -> EGCResults;

// `ISteamGameCoordinator` has three virtual functions, in this order, and no
// destructor, so its slots are the same on both ABIs. TF2's 64-bit Windows
// `steamclient64.dll` implements it with `CAdapterSteamGameCoordinator001`,
// whose vtable has exactly three entries, the first of which forwards
// `(uint32, const void *, uint32)` to the client's own coordinator and returns
// its 32-bit result. The game's `server.dll` calls slot 0 to send each
// message.
const _: () = assert!(
	SEND_MESSAGE_SLOT == 0
		&& IS_MESSAGE_AVAILABLE_SLOT == 1
		&& RETRIEVE_MESSAGE_SLOT == 2
		&& size_of::<ISteamGameCoordinatorVtable>() == 3 * size_of::<usize>()
);

/// `k_EGCResultBufferTooSmall`: the destination cannot hold the message.
#[doc(alias("k_EGCResultBufferTooSmall"))]
pub const GC_RESULT_BUFFER_TOO_SMALL: EGCResults = 2;

/// `k_EGCResultInvalidMessage`: the message is malformed.
#[doc(alias("k_EGCResultInvalidMessage"))]
pub const GC_RESULT_INVALID_MESSAGE: EGCResults = 4;

/// `k_EGCResultNoMessage`: no message is waiting.
#[doc(alias("k_EGCResultNoMessage"))]
pub const GC_RESULT_NO_MESSAGE: EGCResults = 1;

/// `k_EGCResultNotLoggedOn`: the user is not logged on to Steam.
#[doc(alias("k_EGCResultNotLoggedOn"))]
pub const GC_RESULT_NOT_LOGGED_ON: EGCResults = 3;

/// `k_EGCResultOK`: the call succeeded.
#[doc(alias("k_EGCResultOK"))]
pub const GC_RESULT_OK: EGCResults = 0;

/// `ISteamClient::GetISteamGenericInterface`'s slot, the 13th of the 0-based
/// `ISteamClient` vtable, which has no destructor, from
/// `public/steam/isteamclient.h`. TF2's `server.dll` calls it at `+0x60` to
/// create its coordinator.
#[doc(alias("GetISteamGenericInterface"))]
pub const GET_ISTEAM_GENERIC_INTERFACE_SLOT: usize = 12;

/// `ISteamGameCoordinator::IsMessageAvailable`'s slot.
#[doc(alias("IsMessageAvailable"))]
pub const IS_MESSAGE_AVAILABLE_SLOT: usize =
	offset_of!(ISteamGameCoordinatorVtable, is_message_available) / size_of::<usize>();

/// The names `steam_api` has: on Windows, and in 64-bit Linux servers.
const LIBRARIES: &[&CStr] = cfg_select! {
	windows => &[c"steam_api64.dll"],
	target_os = "linux" => &[c"libsteam_api.so"],
};

/// `k_EMsgProtoBufFlag`, from `public/gcsdk/msgbase.h`: set in the type of a
/// message whose body is a protobuf message after a protobuf header, which is
/// how the game sends nearly every message.
#[doc(alias("k_EMsgProtoBufFlag"))]
pub const PROTOBUF_FLAG: u32 = 0x8000_0000;

/// `ISteamGameCoordinator::RetrieveMessage`'s slot.
#[doc(alias("RetrieveMessage"))]
pub const RETRIEVE_MESSAGE_SLOT: usize =
	offset_of!(ISteamGameCoordinatorVtable, retrieve_message) / size_of::<usize>();

/// `ISteamGameCoordinator::SendMessage`'s slot.
#[doc(alias("SendMessage"))]
pub const SEND_MESSAGE_SLOT: usize =
	offset_of!(ISteamGameCoordinatorVtable, send_message) / size_of::<usize>();

/// `k_EMsgGCServerHello`: the game server's greeting to the GC, which it
/// sends as a protobuf message every 30 seconds or so while logged on to
/// Steam but not yet welcomed. TF2's `server.dll` builds it with this type;
/// the SDK's `base_gcmessages.proto` lists it only in a comment.
#[doc(alias("k_EMsgGCServerHello"))]
pub const SERVER_HELLO: u32 = 4007;

/// `k_EMsgGCServerWelcome`: the GC's reply to [`SERVER_HELLO`], which starts
/// the game server's GC session. The SDK's `base_gcmessages.proto` lists it
/// only in a comment.
#[doc(alias("k_EMsgGCServerWelcome"))]
pub const SERVER_WELCOME: u32 = 4005;

/// `STEAMCLIENT_INTERFACE_VERSION`, from `public/steam/isteamclient.h`: the
/// `ISteamClient` version the game creates.
#[doc(alias("STEAMCLIENT_INTERFACE_VERSION"))]
pub const STEAM_CLIENT_INTERFACE_VERSION: &CStr = c"SteamClient020";

/// The file name of Steam's client library, which implements the interfaces
/// `steam_api` hands out.
pub const STEAM_CLIENT_LIBRARY: &str = cfg_select! {
	windows => "steamclient64.dll",
	target_os = "linux" => "steamclient.so",
};

/// `STEAMGAMECOORDINATOR_INTERFACE_VERSION`: the `ISteamGameCoordinator`
/// version the game creates.
#[doc(alias("STEAMGAMECOORDINATOR_INTERFACE_VERSION"))]
pub const STEAM_GAME_COORDINATOR_INTERFACE_VERSION: &CStr = c"SteamGameCoordinator001";

/// The game-server functions `steam_api` exports.
#[derive(Clone, Copy)]
struct GameServerApi {
	find_or_create_interface: FindOrCreateGameServerInterfaceFn,
	pipe: GetHSteamPipeFn,
	user: GetHSteamUserFn,
}

/// `ISteamClient`, the root of the Steamworks interfaces, through whose
/// [`GET_ISTEAM_GENERIC_INTERFACE_SLOT`] the game creates its coordinator.
/// This module reaches the same coordinator through
/// [`find_or_create_game_server_interface`] instead, and never calls it.
#[repr(C)]
pub struct ISteamClient {
	/// The pointer to the object's vtable.
	pub vtable_: *const *const c_void,
}

/// `ISteamGameCoordinator`, the game server's connection to the GC, which
/// Steamworks declares in `isteamgamecoordinator.h`, a header the SDK does not
/// ship.
#[repr(C)]
pub struct ISteamGameCoordinator {
	/// The pointer to the object's vtable.
	pub vtable_: *const ISteamGameCoordinatorVtable,
}

/// `ISteamGameCoordinator`'s vtable.
#[repr(C)]
pub struct ISteamGameCoordinatorVtable {
	/// `SendMessage`.
	pub send_message: SendMessageFn,

	/// `IsMessageAvailable`.
	pub is_message_available: IsMessageAvailableFn,

	/// `RetrieveMessage`.
	pub retrieve_message: RetrieveMessageFn,
}

/// The `steam_api` exports the process has loaded, kept once found.
fn api() -> Option<GameServerApi> {
	static API: OnceLock<GameServerApi> = OnceLock::new();

	if let Some(api) = API.get() {
		return Some(*api);
	}

	let api = find_api()?;

	Some(*API.get_or_init(|| api))
}

/// Looks up the `steam_api` exports in the library the process has already
/// loaded. Returns `None` if `steam_api` is not loaded or lacks any of them,
/// and under Miri.
fn find_api() -> Option<GameServerApi> {
	// Miri cannot call the platform's loader.
	if cfg!(miri) {
		return None;
	}

	LIBRARIES.iter().find_map(|library| {
		let user = loaded_symbol(library, c"SteamGameServer_GetHSteamUser")?;
		let pipe = loaded_symbol(library, c"SteamGameServer_GetHSteamPipe")?;
		let find_or_create_interface =
			loaded_symbol(library, c"SteamInternal_FindOrCreateGameServerInterface")?;

		// SAFETY: `steam_api` exports these with C linkage and these signatures,
		// from `public/steam/steam_api_internal.h`.
		Some(unsafe {
			GameServerApi {
				find_or_create_interface: std::mem::transmute::<
					*mut c_void,
					FindOrCreateGameServerInterfaceFn,
				>(find_or_create_interface.as_ptr()),
				pipe: std::mem::transmute::<*mut c_void, GetHSteamPipeFn>(pipe.as_ptr()),
				user: std::mem::transmute::<*mut c_void, GetHSteamUserFn>(user.as_ptr()),
			}
		})
	})
}

/// The game server's interface `version`, created if it does not exist yet,
/// or `None` while the game server is not running, or if `steam_api` is not
/// loaded.
///
/// The interface lives until the game server shuts down, which releases its
/// user and pipe; see the [module documentation](self).
///
/// # Safety
///
/// Any loaded `steam_api` library must be Steamworks', which stays loaded for
/// the call, and the call must be made on the thread that runs the Steam game
/// server.
#[doc(alias("SteamInternal_FindOrCreateGameServerInterface"))]
pub unsafe fn find_or_create_game_server_interface(version: &CStr) -> Option<NonNull<c_void>> {
	let api = api()?;

	// SAFETY: As the caller promises.
	let (user, _) = unsafe { game_server_user() }?;

	// SAFETY: As the caller promises; the version is a C string, and the
	// function returns null if the game server has no `ISteamClient` or pipe.
	NonNull::new(unsafe { (api.find_or_create_interface)(user, version.as_ptr()) })
}

/// The game server's `ISteamGameCoordinator`, the object the game sends its
/// GC messages through, or `None` while the game server is not running, or if
/// `steam_api` is not loaded.
///
/// The coordinator lives until the game server shuts down; see the
/// [module documentation](self).
///
/// # Safety
///
/// As for [`find_or_create_game_server_interface`].
pub unsafe fn game_server_coordinator() -> Option<NonNull<ISteamGameCoordinator>> {
	// SAFETY: As the caller promises.
	unsafe { find_or_create_game_server_interface(STEAM_GAME_COORDINATOR_INTERFACE_VERSION) }
		.map(NonNull::cast)
}

/// The game server's Steam user and pipe, or `None` while the game server is
/// not running, or if `steam_api` is not loaded.
///
/// # Safety
///
/// As for [`find_or_create_game_server_interface`].
#[doc(alias("SteamGameServer_GetHSteamUser", "SteamGameServer_GetHSteamPipe"))]
pub unsafe fn game_server_user() -> Option<(HSteamUser, HSteamPipe)> {
	let api = api()?;

	// SAFETY: As the caller promises.
	let (user, pipe) = unsafe { ((api.user)(), (api.pipe)()) };

	// Shutdown resets the pipe but leaves the user, so the pipe tells whether
	// the game server runs.
	(user != 0 && pipe != 0).then_some((user, pipe))
}
