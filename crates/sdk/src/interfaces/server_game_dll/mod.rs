//! `IServerGameDLL`, the game's half of the engine interface, which the game
//! DLL implements as `CServerGameDLL`.
//!
//! The engine drives the game through most of this interface, so a plugin
//! should hook those methods rather than call them. Every method is covered:
//!
//! | Method | Wrapper |
//! | --- | --- |
//! | `GetTickInterval` | [`ServerGameDll::tick_interval`] |
//! | `GetAllServerClasses` | [`ServerGameDll::server_classes`], [`ServerGameDll::server_class`] |
//! | `GetGameDescription` | [`ServerGameDll::game_description`] |
//! | `GetSaveComment` | [`ServerGameDll::save_comment`] |
//! | `IsRestoring` | [`ServerGameDll::is_restoring`] |
//! | `GetUserMessageInfo` | [`ServerGameDll::user_message`], [`ServerGameDll::user_messages`] |
//! | `GetStandardSendProxies` | [`ServerGameDll::standard_send_proxies`], [`ServerGameDll::net_prop`] |
//! | `ShouldHideServer` | [`ServerGameDll::should_hide_server`] |
//! | `InvalidateMdlCache` | [`ServerGameDll::invalidate_mdl_cache`] |
//! | `GetServerGCLobby` | [`ServerGameDll::gc_lobby`] |
//! | `GetServerBrowserMapOverride` | [`ServerGameDll::server_browser_map_override`] |
//! | `GetServerBrowserGameData` | [`ServerGameDll::server_browser_game_data`] |
//! | `Status` | [`ServerGameDll::status`] |
//! | `PrepareLevelResources` | [`ServerGameDll::prepare_level_resources`] |
//! | `AsyncPrepareLevelResources` | [`ServerGameDll::async_prepare_level_resources`] |
//! | `CanProvideLevel` | [`ServerGameDll::can_provide_level`] |
//! | `IsManualMapChangeOkay` | [`ServerGameDll::is_manual_map_change_okay`] |
//! | `GetWorkshopMap` | [`ServerGameDll::workshop_map`], [`ServerGameDll::workshop_maps`] |
//! | `DLLInit`, `ReplayInit`, `GameInit`, `LevelInit`, `ServerActivate`, `GameFrame`, `PreClientUpdate`, `LevelShutdown`, `GameShutdown`, `DLLShutdown`, `CreateNetworkStringTables`, `PostInit`, `Think` | Not wrapped: the engine calls these to advance the game's lifecycle, which a plugin re-entering would corrupt. |
//! | `OnQueryCvarValueFinished`, `GameServerSteamAPIActivated`, `GameServerSteamAPIShutdown`, `SetServerHibernation` | Not wrapped: engine notifications, which a plugin would fake by calling. |
//! | `SaveInit`, `SaveWriteFields`, `SaveReadFields`, `SaveGlobalState`, `RestoreGlobalState`, `PreSave`, `Save`, `WriteSaveHeaders`, `ReadRestoreHeaders`, `Restore`, `CreateEntityTransitionList`, `BuildAdjacentMapList`, `PreSaveGameLoaded` | Not wrapped: the single-player save system, driven by the engine. |

use crate::datatables::{NetProp, NetPropError, ServerClass, ServerClasses, StandardSendProxies};
use crate::entities::Entity;
use crate::ffi::{NotThreadSafe, buffer_from_cstr, copy_cstr, cstring_from_buffer, vcall};
use std::cell::RefCell;
use std::ffi::{CStr, CString, VaList, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

/// `MAX_PATH` from `public/tier0/platform.h`, which sizes map name buffers.
const MAX_PATH: usize = 260;

/// The longest user message name read, including its terminator.
const USER_MESSAGE_NAME_CAPACITY: usize = 256;

/// The longest save comment read, including its terminator.
const SAVE_COMMENT_CAPACITY: usize = 256;

/// The longest formatted `Status` line read, including its terminator.
const STATUS_LINE_CAPACITY: usize = 1024;

/// The most user messages [`ServerGameDll::user_messages`] asks for, far more
/// than games register.
const MAX_USER_MESSAGES: usize = 1024;

/// The most workshop maps [`ServerGameDll::workshop_maps`] asks for.
const MAX_WORKSHOP_MAPS: u32 = 1 << 16;

interface! {
	/// The game's half of the engine interface (`IServerGameDLL`), implemented
	/// by the game's `CServerGameDLL`.
	///
	/// See the [module documentation](self) for how each method is exposed.
	#[doc(alias = "IServerGameDLL")]
	#[doc(alias = "CServerGameDLL")]
	pub struct ServerGameDll(sys::IServerGameDLL) = GameServer c"ServerGameDLL012";
}

impl<'s> ServerGameDll<'s> {
	/// Seconds per simulation tick.
	#[doc(alias = "GetTickInterval")]
	pub fn tick_interval(self) -> f32 {
		// SAFETY: `Server::new` guarantees the interface is live.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_GetTickInterval()) }
	}

	/// Every networked entity class, sorted by name.
	#[doc(alias = "GetAllServerClasses")]
	pub fn server_classes(self) -> ServerClasses<'s> {
		// SAFETY: As for `tick_interval`.
		let head = unsafe { vcall!(self.as_ptr() => IServerGameDLL_GetAllServerClasses()) };

		// SAFETY: The game returned the head of its server class list.
		unsafe { ServerClasses::new(head) }
	}

	/// Finds a networked entity class by name, such as `CTFPlayer`.
	pub fn server_class(self, name: &CStr) -> Option<ServerClass<'s>> {
		self.server_classes().find(|class| class.name() == name)
	}

	/// A description of the game, such as `Team Fortress`.
	#[doc(alias = "GetGameDescription")]
	pub fn game_description(self) -> CString {
		// SAFETY: As for `tick_interval`. Game rules may build the description,
		// so it is copied immediately.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IServerGameDLL_GetGameDescription())) }
			.unwrap_or_default()
	}

	/// The comment the game would give a save made after the given play time.
	#[doc(alias = "GetSaveComment")]
	pub fn save_comment(self, minutes: f32, seconds: f32, include_time: bool) -> CString {
		let mut comment = [0 as c_char; SAVE_COMMENT_CAPACITY];

		// SAFETY: As for `tick_interval`, and the buffer length is passed.
		unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_GetSaveComment(comment.as_mut_ptr(), SAVE_COMMENT_CAPACITY as c_int, minutes, seconds, !include_time))
		};

		cstring_from_buffer(&comment)
	}

	/// Whether a single-player save is being restored.
	#[doc(alias = "IsRestoring")]
	pub fn is_restoring(self) -> bool {
		// SAFETY: As for `tick_interval`.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_IsRestoring()) }
	}

	/// The user message the game registered at an index.
	#[doc(alias = "GetUserMessageInfo")]
	pub fn user_message(self, index: usize) -> Option<UserMessage> {
		let message_type = c_int::try_from(index).ok()?;
		let mut name = [0 as c_char; USER_MESSAGE_NAME_CAPACITY];
		let mut size = 0;

		// SAFETY: As for `tick_interval`, and the buffer length is passed.
		let found = unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_GetUserMessageInfo(message_type, name.as_mut_ptr(), USER_MESSAGE_NAME_CAPACITY as c_int, &mut size))
		};

		found.then(|| UserMessage {
			index,
			name: cstring_from_buffer(&name),
			size: usize::try_from(size).ok(),
		})
	}

	/// Every user message the game registered.
	pub fn user_messages(self) -> impl Iterator<Item = UserMessage> + use<'s> {
		(0..MAX_USER_MESSAGES).map_while(move |index| self.user_message(index))
	}

	/// The game DLL's standard send proxies.
	#[doc(alias = "GetStandardSendProxies")]
	pub fn standard_send_proxies(self) -> Option<StandardSendProxies<'s>> {
		// SAFETY: As for `tick_interval`.
		let proxies = NonNull::new(unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_GetStandardSendProxies())
		})?;

		// SAFETY: The game returned its `g_StandardSendProxies`.
		Some(unsafe { StandardSendProxies::from_raw(proxies) })
	}

	/// Resolves a networked variable of a class by name.
	///
	/// The first variable with the name is found, searching the class's table
	/// and the tables nested within it depth first, which includes those of
	/// base classes and embedded structures such as `m_Local`.
	pub fn net_prop(
		self,
		class: ServerClass<'s>,
		name: &CStr,
	) -> Result<NetProp<'s>, NetPropError> {
		let table = class.table().ok_or_else(|| NetPropError::NoTable {
			class: class.name().to_string_lossy().into_owned(),
		})?;

		let proxies = self
			.standard_send_proxies()
			.ok_or(NetPropError::NoStandardProxies)?;

		NetProp::resolve(table, name, proxies)
	}

	/// Resolves a networked variable of an entity's class by name.
	pub fn entity_net_prop(
		self,
		entity: Entity<'s>,
		name: &CStr,
	) -> Result<NetProp<'s>, NetPropError> {
		let class = entity
			.server_class()
			.ok_or_else(|| NetPropError::NotNetworked {
				class_name: entity.class_name().to_string_lossy().into_owned(),
			})?;

		self.net_prop(class, name)
	}

	/// Whether the game asks not to list the server publicly.
	#[doc(alias = "ShouldHideServer")]
	pub fn should_hide_server(self) -> bool {
		// SAFETY: As for `tick_interval`.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_ShouldHideServer()) }
	}

	/// Makes the game drop its cached model data, as the engine does after
	/// flushing the model cache.
	#[doc(alias = "InvalidateMdlCache")]
	pub fn invalidate_mdl_cache(self) {
		// SAFETY: As for `tick_interval`.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_InvalidateMdlCache()) };
	}

	/// The game coordinator's lobby for this server, if the game has one.
	#[doc(alias = "GetServerGCLobby")]
	pub fn gc_lobby(self) -> Option<ServerGcLobby<'s>> {
		// SAFETY: As for `tick_interval`.
		let lobby =
			NonNull::new(unsafe { vcall!(self.as_ptr() => IServerGameDLL_GetServerGCLobby()) })?;

		Some(ServerGcLobby {
			raw: lobby,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}

	/// What the server browser's map column shows instead of the map name.
	#[doc(alias = "GetServerBrowserMapOverride")]
	pub fn server_browser_map_override(self) -> Option<CString> {
		// SAFETY: As for `game_description`.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IServerGameDLL_GetServerBrowserMapOverride())) }
	}

	/// The game data string sent to the master server.
	#[doc(alias = "GetServerBrowserGameData")]
	pub fn server_browser_game_data(self) -> Option<CString> {
		// SAFETY: As for `game_description`.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IServerGameDLL_GetServerBrowserGameData())) }
	}

	/// The lines the game adds to the `status` command's output.
	///
	/// Each line is formatted as the game prints it, truncated to 1023 bytes.
	#[doc(alias = "Status")]
	pub fn status(self) -> CString {
		let previous = STATUS_OUTPUT.replace(Some(Vec::new()));

		// SAFETY: As for `tick_interval`. The callback only runs during the call.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_Status(Some(append_status))) };

		let output = STATUS_OUTPUT.replace(previous).unwrap_or_default();

		// SAFETY: `append_status` only appends the bytes before each terminator.
		unsafe { CString::from_vec_unchecked(output) }
	}

	/// Resolves a map name, and has the game prepare its resources, such as
	/// downloading a workshop map. This may block for a long time.
	#[doc(alias = "PrepareLevelResources")]
	pub fn prepare_level_resources(self, map: &CStr) -> Result<LevelResources, MapNameTooLong> {
		let (mut map_name, mut map_file) = level_buffers(map)?;

		// SAFETY: As for `tick_interval`, and the buffer lengths are passed.
		unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_PrepareLevelResources(map_name.as_mut_ptr(), MAX_PATH, map_file.as_mut_ptr(), MAX_PATH))
		};

		Ok(LevelResources::from_buffers(&map_name, &map_file))
	}

	/// Starts or polls preparing a level's resources without blocking.
	#[doc(alias = "AsyncPrepareLevelResources")]
	pub fn async_prepare_level_resources(
		self,
		map: &CStr,
	) -> Result<LevelPreparation, MapNameTooLong> {
		let (mut map_name, mut map_file) = level_buffers(map)?;
		let mut progress = 0.0;

		// SAFETY: As for `prepare_level_resources`, and the progress is a local.
		let result = unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_AsyncPrepareLevelResources(map_name.as_mut_ptr(), MAX_PATH, map_file.as_mut_ptr(), MAX_PATH, &mut progress))
		};

		let resources = LevelResources::from_buffers(&map_name, &map_file);

		Ok(match result {
			sys::IServerGameDLL_ePrepareLevelResourcesResult_ePrepareLevelResources_InProgress => {
				LevelPreparation::InProgress {
					resources,
					progress,
				}
			}
			_ => LevelPreparation::Prepared(resources),
		})
	}

	/// What the game would do with a map name if asked to prepare it, without
	/// blocking.
	#[doc(alias = "CanProvideLevel")]
	pub fn can_provide_level(self, map: &CStr) -> Result<LevelProvision, MapNameTooLong> {
		let (mut map_name, _) = level_buffers(map)?;

		// SAFETY: As for `prepare_level_resources`.
		let result = unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_CanProvideLevel(map_name.as_mut_ptr(), MAX_PATH as c_int))
		};

		Ok(match result {
			sys::IServerGameDLL_eCanProvideLevelResult_eCanProvideLevel_CanProvide => {
				LevelProvision::CanProvide(cstring_from_buffer(&map_name))
			}
			sys::IServerGameDLL_eCanProvideLevelResult_eCanProvideLevel_Possibly => {
				LevelProvision::Possibly
			}
			_ => LevelProvision::CannotProvide,
		})
	}

	/// Whether the game allows the `map` or `changelevel` commands right now.
	#[doc(alias = "IsManualMapChangeOkay")]
	pub fn is_manual_map_change_okay(self) -> Result<(), MapChangeRefused> {
		let mut reason = ptr::null();

		// SAFETY: As for `tick_interval`, and the reason is a local.
		let okay =
			unsafe { vcall!(self.as_ptr() => IServerGameDLL_IsManualMapChangeOkay(&mut reason)) };

		if okay {
			return Ok(());
		}

		Err(MapChangeRefused {
			// SAFETY: The game points the reason at a string it keeps.
			reason: unsafe { copy_cstr(reason) }
				.map(|reason| reason.to_string_lossy().into_owned()),
		})
	}

	/// The workshop map the game knows at an index.
	#[doc(alias = "GetWorkshopMap")]
	pub fn workshop_map(self, index: u32) -> Option<WorkshopMap> {
		// SAFETY: The description is plain data, for which zeroes are valid.
		let mut description: sys::WorkshopMapDesc_t = unsafe { std::mem::zeroed() };

		// SAFETY: As for `tick_interval`, and the description is a local.
		let found = unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_GetWorkshopMap(index, &mut description))
		};

		found.then(|| WorkshopMap {
			name: cstring_from_buffer(&description.szMapName),
			original_name: cstring_from_buffer(&description.szOriginalMapName),
			timestamp: description.uTimestamp,
			downloaded: description.bDownloaded,
		})
	}

	/// Every workshop map the game knows.
	pub fn workshop_maps(self) -> impl Iterator<Item = WorkshopMap> + use<'s> {
		(0..MAX_WORKSHOP_MAPS).map_while(move |index| self.workshop_map(index))
	}
}

/// A user message the game registered, from [`ServerGameDll::user_message`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserMessage {
	/// The message type, which `IVEngineServer::UserMessageBegin` takes.
	pub index: usize,

	pub name: CString,

	/// The message's size in bytes, or `None` if it varies.
	pub size: Option<usize>,
}

/// A map name and file the game resolved for loading.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LevelResources {
	/// The canonical map name, such as `workshop/cp_example.ugc123`.
	pub map_name: CString,

	/// The file the map loads from.
	pub map_file: CString,
}

impl LevelResources {
	fn from_buffers(map_name: &[c_char], map_file: &[c_char]) -> Self {
		Self {
			map_name: cstring_from_buffer(map_name),
			map_file: cstring_from_buffer(map_file),
		}
	}
}

/// How far preparing a level has come, from
/// [`ServerGameDll::async_prepare_level_resources`].
#[derive(Debug, Clone, PartialEq)]
pub enum LevelPreparation {
	Prepared(LevelResources),

	/// The game is still preparing resources, `progress` of the way from 0 to 1.
	InProgress {
		resources: LevelResources,
		progress: f32,
	},
}

/// What the game would do with a map name, from [`ServerGameDll::can_provide_level`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LevelProvision {
	/// The game does not know the map, so the engine would load it from `maps/`.
	CannotProvide,

	/// The game can provide the map, under this canonical name.
	CanProvide(CString),

	/// The game may be able to provide the map, which only preparing it tells.
	Possibly,
}

/// A workshop map the game knows, from [`ServerGameDll::workshop_map`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkshopMap {
	pub name: CString,
	pub original_name: CString,
	pub timestamp: u32,
	pub downloaded: bool,
}

/// A map name does not fit in the buffers the game resolves it in.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("map name `{name}` does not fit in the game's {MAX_PATH}-byte buffer")]
pub struct MapNameTooLong {
	name: String,
}

/// The game refused a manual map change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the game refuses map changes right now{}", .reason.as_deref().map(|reason| format!(": {reason}")).unwrap_or_default())]
pub struct MapChangeRefused {
	pub reason: Option<String>,
}

/// The game coordinator's lobby for this server (`IServerGCLobby`).
#[doc(alias = "IServerGCLobby")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerGcLobby<'s> {
	raw: NonNull<sys::IServerGCLobby>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ServerGcLobby<'s> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::IServerGCLobby {
		self.raw.as_ptr()
	}

	/// Whether the server is hosting a matchmade lobby.
	#[doc(alias = "HasLobby")]
	pub fn has_lobby(self) -> bool {
		// SAFETY: The lobby interface is a singleton of the game DLL.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_HasLobby()) }
	}

	/// Whether the lobby admits the player with a 64-bit Steam ID.
	#[doc(alias = "SteamIDAllowedToConnect")]
	pub fn steam_id_allowed_to_connect(self, steam_id: u64) -> bool {
		let steam_id = steam_id_of(steam_id);

		// SAFETY: As for `has_lobby`, and the ID is a local.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_SteamIDAllowedToConnect(&steam_id)) }
	}

	/// Sends the server's details to the game coordinator.
	#[doc(alias = "UpdateServerDetails")]
	pub fn update_server_details(self) {
		// SAFETY: As for `has_lobby`.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_UpdateServerDetails()) };
	}

	/// Whether the lobby lets the server hibernate.
	#[doc(alias = "ShouldHibernate")]
	pub fn should_hibernate(self) -> bool {
		// SAFETY: As for `has_lobby`.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_ShouldHibernate()) }
	}

	/// Whether the lobby's match lets players change their names.
	#[doc(alias = "MatchAllowsNameChanges")]
	pub fn match_allows_name_changes(self) -> bool {
		// SAFETY: As for `has_lobby`.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_MatchAllowsNameChanges()) }
	}

	/// The name the game coordinator gave a player in the lobby's match.
	#[doc(alias = "GetPlayerGCMatchName")]
	pub fn player_gc_match_name(self, steam_id: u64) -> Option<CString> {
		let steam_id = steam_id_of(steam_id);
		let mut name = [0 as c_char; MAX_PATH];

		// SAFETY: As for `has_lobby`, and the buffer length is passed.
		let found = unsafe {
			vcall!(self.as_ptr() => IServerGCLobby_GetPlayerGCMatchName(&steam_id, name.as_mut_ptr(), MAX_PATH))
		};

		found.then(|| cstring_from_buffer(&name))
	}
}

fn steam_id_of(steam_id: u64) -> sys::CSteamID {
	sys::CSteamID {
		m_steamid: sys::CSteamID_SteamID_t {
			m_unAll64Bits: steam_id,
		},
	}
}

/// Fills the map name and file buffers the game resolves levels in.
fn level_buffers(map: &CStr) -> Result<([c_char; MAX_PATH], [c_char; MAX_PATH]), MapNameTooLong> {
	let too_long = || MapNameTooLong {
		name: map.to_string_lossy().into_owned(),
	};

	let file =
		CString::new(format!("maps/{}.bsp", map.to_string_lossy())).map_err(|_| too_long())?;

	Ok((
		buffer_from_cstr(map).ok_or_else(too_long)?,
		buffer_from_cstr(&file).ok_or_else(too_long)?,
	))
}

thread_local! {
	/// Output of the `Status` call in progress on this thread.
	static STATUS_OUTPUT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

unsafe extern "C" {
	/// The UCRT's formatter, which its inline `vsnprintf` calls.
	#[cfg(target_os = "windows")]
	fn __stdio_common_vsprintf(
		options: u64,
		buffer: *mut c_char,
		count: usize,
		format: *const c_char,
		locale: *mut std::ffi::c_void,
		arguments: VaList<'_>,
	) -> c_int;

	#[cfg(not(target_os = "windows"))]
	fn vsnprintf(
		buffer: *mut c_char,
		count: usize,
		format: *const c_char,
		arguments: VaList<'_>,
	) -> c_int;
}

/// Formats a `printf` call into `buffer`, truncating and terminating it.
///
/// # Safety
///
/// `format` and `arguments` must be a valid `printf` format and its arguments.
unsafe fn format_into(
	buffer: &mut [c_char],
	format: *const c_char,
	arguments: VaList<'_>,
) -> c_int {
	cfg_select! {
		target_os = "windows" => {
			/// `_CRT_INTERNAL_PRINTF_STANDARD_SNPRINTF_BEHAVIOR`, which makes the
			/// formatter behave as C99's `vsnprintf`.
			const STANDARD_SNPRINTF_BEHAVIOR: u64 = 1 << 1;

			// SAFETY: The caller upholds the contract, and the length is passed.
			unsafe {
				__stdio_common_vsprintf(
					STANDARD_SNPRINTF_BEHAVIOR,
					buffer.as_mut_ptr(),
					buffer.len(),
					format,
					ptr::null_mut(),
					arguments,
				)
			}
		}

		_ => {
			// SAFETY: The caller upholds the contract, and the length is passed.
			unsafe { vsnprintf(buffer.as_mut_ptr(), buffer.len(), format, arguments) }
		}
	}
}

/// The `print` callback `Status` receives, appending to [`STATUS_OUTPUT`].
unsafe extern "C" fn append_status(format: *const c_char, arguments: ...) {
	if format.is_null() {
		return;
	}

	let mut line = [0 as c_char; STATUS_LINE_CAPACITY];

	// SAFETY: The game passes a `printf` format and matching arguments.
	if unsafe { format_into(&mut line, format, arguments) } < 0 {
		return;
	}

	let line = cstring_from_buffer(&line);

	STATUS_OUTPUT.with_borrow_mut(|output| {
		if let Some(output) = output {
			output.extend_from_slice(line.as_bytes());
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ffi::test_support::{mock_vtable, unexpected_call};

	unsafe extern "C" fn tick_interval(_: *const sys::IServerGameDLL) -> f32 {
		0.015
	}

	unsafe extern "C" fn status(
		_: *mut sys::IServerGameDLL,
		print: Option<unsafe extern "C" fn(*const c_char, ...)>,
	) {
		let print = print.unwrap();

		unsafe {
			print(c"Blue Team Wins: %d\n".as_ptr(), 3 as c_int);
			print(c"%-8s %6.1f\n".as_ptr(), c"Scout".as_ptr(), 2.5f64);
		}
	}

	unsafe extern "C" fn user_message_info(
		_: *mut sys::IServerGameDLL,
		index: c_int,
		name: *mut c_char,
		capacity: c_int,
		size: *mut c_int,
	) -> bool {
		let (message, message_size): (&CStr, c_int) = match index {
			0 => (c"Geiger", 1),
			1 => (c"SayText2", -1),
			_ => return false,
		};

		assert!(capacity as usize > message.count_bytes());
		unsafe {
			ptr::copy_nonoverlapping(message.as_ptr(), name, message.count_bytes() + 1);
			size.write(message_size);
		}
		true
	}

	unsafe extern "C" fn can_provide_level(
		_: *mut sys::IServerGameDLL,
		name: *mut c_char,
		capacity: c_int,
	) -> sys::IServerGameDLL_eCanProvideLevelResult {
		assert_eq!(capacity as usize, MAX_PATH);

		if unsafe { CStr::from_ptr(name) } != c"workshop/123" {
			return sys::IServerGameDLL_eCanProvideLevelResult_eCanProvideLevel_CannotProvide;
		}

		unsafe { ptr::copy_nonoverlapping(c"workshop/cp_example.ugc123".as_ptr(), name, 27) };
		sys::IServerGameDLL_eCanProvideLevelResult_eCanProvideLevel_CanProvide
	}

	unsafe extern "C" fn manual_map_change(
		_: *mut sys::IServerGameDLL,
		reason: *mut *const c_char,
	) -> bool {
		unsafe { reason.write(c"Tournament in progress".as_ptr()) };
		false
	}

	#[test]
	fn methods_convert_the_games_answers() {
		let vtable = unsafe {
			mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerGameDLL_GetTickInterval).write(tick_interval);
					(&raw mut (*vtable).IServerGameDLL_Status).write(status);
					(&raw mut (*vtable).IServerGameDLL_GetUserMessageInfo).write(user_message_info);
					(&raw mut (*vtable).IServerGameDLL_CanProvideLevel).write(can_provide_level);
					(&raw mut (*vtable).IServerGameDLL_IsManualMapChangeOkay)
						.write(manual_map_change);
				},
			)
		};

		let mut interface = sys::IServerGameDLL {
			vtable_: &raw const *vtable,
		};
		let game = unsafe { ServerGameDll::from_raw(NonNull::from(&mut interface)) };

		assert_eq!(game.tick_interval(), 0.015);
		assert_eq!(
			game.status().as_c_str(),
			c"Blue Team Wins: 3\nScout       2.5\n"
		);

		let messages = game.user_messages().collect::<Vec<_>>();
		assert_eq!(messages.len(), 2);
		assert_eq!(
			(messages[0].name.as_c_str(), messages[0].size),
			(c"Geiger", Some(1))
		);
		assert_eq!((messages[1].index, messages[1].size), (1, None));

		assert_eq!(
			game.can_provide_level(c"workshop/123"),
			Ok(LevelProvision::CanProvide(
				c"workshop/cp_example.ugc123".to_owned()
			))
		);
		assert_eq!(
			game.can_provide_level(c"ctf_2fort"),
			Ok(LevelProvision::CannotProvide)
		);
		assert!(
			game.can_provide_level(&CString::new("x".repeat(MAX_PATH)).unwrap())
				.is_err()
		);

		assert_eq!(
			game.is_manual_map_change_okay().unwrap_err().to_string(),
			"the game refuses map changes right now: Tournament in progress"
		);
	}
}
