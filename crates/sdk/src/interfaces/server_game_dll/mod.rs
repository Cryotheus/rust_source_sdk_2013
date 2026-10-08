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

use crate::NotThreadSafe;
use crate::datatables::{NetProp, NetPropError, ServerClass, ServerClasses, StandardSendProxies};
use crate::entities::Entity;
use sdk_raw::tier0::MAX_PATH;
use sdk_raw::util::cstr::{buffer_from_cstr, copy_cstr, cstring_from_buffer};
use sdk_raw::util::printf::capture_printf;
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

/// The most user messages [`ServerGameDll::user_messages`] asks for, far more
/// than games register.
const MAX_USER_MESSAGES: usize = 1024;

/// The most workshop maps [`ServerGameDll::workshop_maps`] asks for.
const MAX_WORKSHOP_MAPS: u32 = 1 << 16;

/// The longest save comment read, including its terminator.
const SAVE_COMMENT_CAPACITY: usize = 256;

/// The longest user message name read, including its terminator.
const USER_MESSAGE_NAME_CAPACITY: usize = 256;

interface! {
	/// The game's half of the engine interface (`IServerGameDLL`), implemented
	/// by the game's `CServerGameDLL`.
	///
	/// See the [module documentation](self) for how each method is exposed.
	#[doc(alias("IServerGameDLL", "CServerGameDLL"))]
	pub struct ServerGameDll(sys::IServerGameDLL) = GameServer sdk_raw::interfaces::server_game_dll::VERSION;
}

/// How far preparing a level has come, from
/// [`ServerGameDll::async_prepare_level_resources`].
#[doc(alias("ePrepareLevelResourcesResult"))]
#[derive(Debug, Clone, PartialEq)]
pub enum LevelPreparation {
	/// The game has prepared the level's resources, resolved to this map name
	/// and file.
	#[doc(alias("ePrepareLevelResources_Prepared"))]
	Prepared(LevelResources),

	/// The game is still preparing resources, `progress` of the way from 0 to 1.
	#[doc(alias("ePrepareLevelResources_InProgress"))]
	InProgress {
		/// The map name and file as the game left them in this call.
		resources: LevelResources,

		/// The fraction of the work done, from 0 to 1, or 0 if the game did not
		/// report it.
		progress: f32,
	},
}

/// What the game would do with a map name, from [`ServerGameDll::can_provide_level`].
#[doc(alias("eCanProvideLevelResult"))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LevelProvision {
	/// The game does not know the map, so the engine would load it from `maps/`.
	#[doc(alias("eCanProvideLevel_CannotProvide"))]
	CannotProvide,

	/// The game can provide the map, under this canonical name.
	#[doc(alias("eCanProvideLevel_CanProvide"))]
	CanProvide(CString),

	/// The game may be able to provide the map, which only preparing it tells.
	#[doc(alias("eCanProvideLevel_Possibly"))]
	Possibly,
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
	/// Copies the map name and file the game wrote into the buffers from
	/// [`level_buffers`].
	fn from_buffers(map_name: &[c_char], map_file: &[c_char]) -> Self {
		Self {
			map_name: cstring_from_buffer(map_name),
			map_file: cstring_from_buffer(map_file),
		}
	}
}

/// The game refused a manual map change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the game refuses map changes right now{}", .reason.as_deref().map(|reason| format!(": {reason}")).unwrap_or_default())]
pub struct MapChangeRefused {
	/// The reason the game gave, converted lossily to UTF-8, or `None` if it
	/// gave none.
	pub reason: Option<String>,
}

/// A map name does not fit in the buffers the game resolves it in.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("map name `{name}` does not fit in the game's {MAX_PATH}-byte buffer")]
pub struct MapNameTooLong {
	name: String,
}

/// The game coordinator's lobby for this server (`IServerGCLobby`).
#[doc(alias("IServerGCLobby"))]
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
	#[doc(alias("HasLobby"))]
	pub fn has_lobby(self) -> bool {
		// SAFETY: The lobby interface is a singleton of the game DLL.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_HasLobby()) }
	}

	/// Whether the lobby's match lets players change their names.
	#[doc(alias("MatchAllowsNameChanges"))]
	pub fn match_allows_name_changes(self) -> bool {
		// SAFETY: As for `has_lobby`.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_MatchAllowsNameChanges()) }
	}

	/// The name the game coordinator gave the player with a 64-bit Steam ID in
	/// the lobby's match, or `None` if the game reports none.
	#[doc(alias("GetPlayerGCMatchName"))]
	pub fn player_gc_match_name(self, steam_id: u64) -> Option<CString> {
		let steam_id = steam_id_of(steam_id);
		let mut name = [0 as c_char; MAX_PATH];

		// SAFETY: As for `has_lobby`, and the buffer length is passed.
		let found = unsafe {
			vcall!(self.as_ptr() => IServerGCLobby_GetPlayerGCMatchName(&steam_id, name.as_mut_ptr(), MAX_PATH))
		};

		found.then(|| cstring_from_buffer(&name))
	}

	/// Whether the lobby lets the server hibernate.
	#[doc(alias("ShouldHibernate"))]
	pub fn should_hibernate(self) -> bool {
		// SAFETY: As for `has_lobby`.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_ShouldHibernate()) }
	}

	/// Whether the lobby admits the player with a 64-bit Steam ID.
	#[doc(alias("SteamIDAllowedToConnect"))]
	pub fn steam_id_allowed_to_connect(self, steam_id: u64) -> bool {
		let steam_id = steam_id_of(steam_id);

		// SAFETY: As for `has_lobby`, and the ID is a local.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_SteamIDAllowedToConnect(&steam_id)) }
	}

	/// Sends the server's details to the game coordinator.
	#[doc(alias("UpdateServerDetails"))]
	pub fn update_server_details(self) {
		// SAFETY: As for `has_lobby`.
		unsafe { vcall!(self.as_ptr() => IServerGCLobby_UpdateServerDetails()) };
	}
}

/// A user message the game registered, from [`ServerGameDll::user_message`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserMessage {
	/// The message type, which `IVEngineServer::UserMessageBegin` takes.
	pub index: usize,

	/// The name the game registered the message under, such as `SayText2`.
	pub name: CString,

	/// The message's size in bytes, or `None` if it varies.
	pub size: Option<usize>,
}

/// A workshop map the game knows, from [`ServerGameDll::workshop_map`]. The
/// engine reads these mainly to list maps.
#[doc(alias("WorkshopMapDesc_t"))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkshopMap {
	/// The map's name, as the game reports it.
	#[doc(alias("szMapName"))]
	pub name: CString,

	/// The map's original name, as the game reports it.
	#[doc(alias("szOriginalMapName"))]
	pub original_name: CString,

	/// The map's timestamp, as the game reports it.
	#[doc(alias("uTimestamp"))]
	pub timestamp: u32,

	/// Whether the game reports the map as downloaded.
	#[doc(alias("bDownloaded"))]
	pub downloaded: bool,
}

impl<'s> ServerGameDll<'s> {
	/// Starts or polls preparing a level's resources without blocking.
	///
	/// The game starts from `map` and the file `maps/<map>.bsp`, as for
	/// [`Self::prepare_level_resources`]. Fails if either does not fit in the
	/// game's buffers.
	#[doc(alias("AsyncPrepareLevelResources"))]
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
	/// blocking. Fails if the name, or the file `maps/<map>.bsp`, does not fit
	/// in the game's buffers, as for [`Self::prepare_level_resources`].
	#[doc(alias("CanProvideLevel"))]
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

	/// Resolves a networked variable of an entity's class by name, as
	/// [`Self::net_prop`] does.
	///
	/// Fails with [`NetPropError::NotNetworked`] for an entity without a
	/// server class.
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

	/// A description of the game, such as `Team Fortress`, or empty if the game
	/// returns none.
	#[doc(alias("GetGameDescription"))]
	pub fn game_description(self) -> CString {
		// SAFETY: As for `tick_interval`. Game rules may build the description,
		// so it is copied immediately.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IServerGameDLL_GetGameDescription())) }
			.unwrap_or_default()
	}

	/// The game coordinator's lobby for this server, if the game has one.
	#[doc(alias("GetServerGCLobby"))]
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

	/// Makes the game drop its cached model data, as the engine does after
	/// flushing the model cache.
	#[doc(alias("InvalidateMdlCache"))]
	pub fn invalidate_mdl_cache(self) {
		// SAFETY: As for `tick_interval`.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_InvalidateMdlCache()) };
	}

	/// Whether the game allows the `map` or `changelevel` commands right now.
	#[doc(alias("IsManualMapChangeOkay"))]
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

	/// Whether a single-player save is being restored.
	#[doc(alias("IsRestoring"))]
	pub fn is_restoring(self) -> bool {
		// SAFETY: As for `tick_interval`.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_IsRestoring()) }
	}

	/// Resolves a networked variable of a class by name.
	///
	/// The first variable with the name is found, searching the class's table
	/// and the tables nested within it depth first, which includes those of
	/// base classes and embedded structures such as `m_Local`.
	///
	/// Fails if the class has no send table, the game provides no standard
	/// send proxies, or the variable is missing or cannot be addressed.
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

	/// Resolves a map name, and has the game prepare its resources, such as
	/// downloading a workshop map. This may block for a long time.
	///
	/// The game starts from `map` and the file `maps/<map>.bsp`, and may
	/// replace either. Fails if either does not fit in the game's buffers.
	#[doc(alias("PrepareLevelResources"))]
	pub fn prepare_level_resources(self, map: &CStr) -> Result<LevelResources, MapNameTooLong> {
		let (mut map_name, mut map_file) = level_buffers(map)?;

		// SAFETY: As for `tick_interval`, and the buffer lengths are passed.
		unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_PrepareLevelResources(map_name.as_mut_ptr(), MAX_PATH, map_file.as_mut_ptr(), MAX_PATH))
		};

		Ok(LevelResources::from_buffers(&map_name, &map_file))
	}

	/// The comment the game would give a save made after the given play time.
	#[doc(alias("GetSaveComment"))]
	pub fn save_comment(self, minutes: f32, seconds: f32, include_time: bool) -> CString {
		let mut comment = [0 as c_char; SAVE_COMMENT_CAPACITY];

		// SAFETY: As for `tick_interval`, and the buffer length is passed.
		unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_GetSaveComment(comment.as_mut_ptr(), SAVE_COMMENT_CAPACITY as c_int, minutes, seconds, !include_time))
		};

		cstring_from_buffer(&comment)
	}

	/// The game data string sent to the master server, or `None` if the game
	/// returns none.
	#[doc(alias("GetServerBrowserGameData"))]
	pub fn server_browser_game_data(self) -> Option<CString> {
		// SAFETY: As for `game_description`.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IServerGameDLL_GetServerBrowserGameData())) }
	}

	/// What the server browser's map column shows instead of the map name, or
	/// `None` if it shows the map name.
	#[doc(alias("GetServerBrowserMapOverride"))]
	pub fn server_browser_map_override(self) -> Option<CString> {
		// SAFETY: As for `game_description`.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IServerGameDLL_GetServerBrowserMapOverride())) }
	}

	/// Finds a networked entity class by name, such as `CTFPlayer`, or `None`
	/// if the game has none with the name.
	#[doc(alias("GetAllServerClasses"))]
	pub fn server_class(self, name: &CStr) -> Option<ServerClass<'s>> {
		self.server_classes().find(|class| class.name() == name)
	}

	/// Every networked entity class, sorted by name.
	#[doc(alias("GetAllServerClasses"))]
	pub fn server_classes(self) -> ServerClasses<'s> {
		// SAFETY: As for `tick_interval`.
		let head = unsafe { vcall!(self.as_ptr() => IServerGameDLL_GetAllServerClasses()) };

		// SAFETY: The game returned the head of its server class list.
		unsafe { ServerClasses::new(head) }
	}

	/// Whether the game asks not to list the server publicly.
	#[doc(alias("ShouldHideServer"))]
	pub fn should_hide_server(self) -> bool {
		// SAFETY: As for `tick_interval`.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_ShouldHideServer()) }
	}

	/// The game DLL's standard send proxies, or `None` if the game returns
	/// none.
	#[doc(alias("GetStandardSendProxies"))]
	pub fn standard_send_proxies(self) -> Option<StandardSendProxies<'s>> {
		// SAFETY: As for `tick_interval`.
		let proxies = NonNull::new(unsafe {
			vcall!(self.as_ptr() => IServerGameDLL_GetStandardSendProxies())
		})?;

		// SAFETY: The game returned its `g_StandardSendProxies`.
		Some(unsafe { StandardSendProxies::from_raw(proxies) })
	}

	/// The lines the game adds to the `status` command's output.
	///
	/// Each line is formatted as the game prints it, truncated to 1023 bytes.
	#[doc(alias("Status"))]
	pub fn status(self) -> CString {
		let ((), output) = capture_printf(|print| {
			// SAFETY: As for `tick_interval`. The game only calls the callback
			// during the call, with `printf` formats and their arguments.
			unsafe { vcall!(self.as_ptr() => IServerGameDLL_Status(Some(print))) }
		});

		output
	}

	/// Seconds per simulation tick.
	#[doc(alias("GetTickInterval"))]
	pub fn tick_interval(self) -> f32 {
		// SAFETY: `Server::new` guarantees the interface is live.
		unsafe { vcall!(self.as_ptr() => IServerGameDLL_GetTickInterval()) }
	}

	/// The user message the game registered at an index, or `None` if there is
	/// none.
	#[doc(alias("GetUserMessageInfo"))]
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

	/// Every user message the game registered, in index order, up to the first
	/// index without one.
	#[doc(alias("GetUserMessageInfo"))]
	pub fn user_messages(self) -> impl Iterator<Item = UserMessage> + use<'s> {
		(0..MAX_USER_MESSAGES).map_while(move |index| self.user_message(index))
	}

	/// The workshop map the game knows at an index, or `None` if the index is
	/// invalid.
	#[doc(alias("GetWorkshopMap"))]
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

	/// Every workshop map the game knows, in index order, up to the first
	/// invalid index.
	#[doc(alias("GetWorkshopMap"))]
	pub fn workshop_maps(self) -> impl Iterator<Item = WorkshopMap> + use<'s> {
		(0..MAX_WORKSHOP_MAPS).map_while(move |index| self.workshop_map(index))
	}
}

/// Fills the map name and file buffers the game resolves levels in, with `map`
/// and `maps/<map>.bsp`.
fn level_buffers(map: &CStr) -> Result<([c_char; MAX_PATH], [c_char; MAX_PATH]), MapNameTooLong> {
	let too_long = || MapNameTooLong {
		name: map.to_string_lossy().into_owned(),
	};

	// Built from the bytes, so a name that is not UTF-8 is kept as it is. A
	// `CStr` holds no interior NUL, so `CString::new` cannot fail.
	let file = [b"maps/".as_slice(), map.to_bytes(), b".bsp"].concat();
	let file = CString::new(file).map_err(|_| too_long())?;

	Ok((
		buffer_from_cstr(map).ok_or_else(too_long)?,
		buffer_from_cstr(&file).ok_or_else(too_long)?,
	))
}

/// Wraps a 64-bit Steam ID in the `CSteamID` the lobby interface takes.
fn steam_id_of(steam_id: u64) -> sys::CSteamID {
	sys::CSteamID {
		m_steamid: sys::CSteamID_SteamID_t {
			m_unAll64Bits: steam_id,
		},
	}
}
