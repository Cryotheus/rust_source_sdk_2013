//! The crate's entry point, through which every other interface is reached.

#[cfg(test)]
#[path = "tests/server.rs"]
mod tests;

use crate::NotThreadSafe;

use crate::interfaces::{
	BotManager, Cvar, EngineReplay, EngineSound, EngineTrace, GameEventManager, ModelInfo,
	NetworkStringTables, PlayerInfoManager, PluginHelpers, ServerGameClients, ServerGameDll,
	ServerGameEnts, ServerGameTags, ServerTools, ValveEngine, VoiceServer,
};

use sdk_raw::entities::TeleportSlot;
use sdk_raw::interfaces::{CreateInterfaceFn, create_interface};

#[cfg(test)]
use std::cell::Cell;

use std::ffi::{CStr, CString};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The game a server runs, which decides the ABI details that the SDK headers
/// alone cannot describe, such as virtual methods added under `TF_DLL` and
/// `NEXT_BOT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Game {
	/// Team Fortress 2, whose game DLL is built with `TF_DLL` and `NEXT_BOT`.
	TeamFortress2,

	/// A Source SDK 2013 mod whose game DLL is built with neither `TF_DLL` nor
	/// `NEXT_BOT`, as `server_hl2.vpc`, `server_episodic.vpc`, and
	/// `server_lostcoast.vpc` build it, or with `NEXT_BOT` from sources older
	/// than `IsNextBot`, which Valve's `baseentity.h` has declared only since
	/// March 2025. A mod built from the current `server_hl2mp.vpc`, which
	/// defines `NEXT_BOT`, is [`Game::SourceSdk2013NextBot`].
	SourceSdk2013,

	/// A Source SDK 2013 mod whose game DLL is built with `NEXT_BOT` but not
	/// `TF_DLL`, from sources that declare `IsNextBot`, as `server_hl2mp.vpc`
	/// builds Half-Life 2: Deathmatch and the mods based on it.
	SourceSdk2013NextBot,
}

impl Game {
	/// `CBaseEntity::Teleport` in the game DLL's primary `CBaseEntity` vtable.
	pub(crate) const fn teleport_vtable_slot(self) -> TeleportSlot {
		match self {
			Self::TeamFortress2 => TeleportSlot::TeamFortress2,
			Self::SourceSdk2013 => TeleportSlot::SourceSdk2013,
			Self::SourceSdk2013NextBot => TeleportSlot::SourceSdk2013NextBot,
		}
	}
}

/// An interface a factory exports, and the handle type wrapping it.
///
/// # Safety
///
/// `Raw` must be the C++ class that [`Self::MODULE`] exports under
/// [`Self::VERSION`], and `bind` may only wrap the pointer it is given.
pub(crate) unsafe trait Interface<'s>: Sized {
	/// The C++ class of the exported object, such as `sys::IVEngineServer`.
	type Raw;

	/// The module whose factory exports the interface.
	const MODULE: Module;

	/// The exact version string the interface is requested by.
	const VERSION: &'static CStr;

	/// Wraps the object the factory returned in its handle.
	///
	/// # Safety
	///
	/// `raw` must be the live object exported under [`Self::VERSION`], alive
	/// for `'s`.
	unsafe fn bind(raw: NonNull<Self::Raw>, server: &Server<'s>) -> Self;
}

/// A module does not export an interface at the version these bindings expect.
///
/// Interfaces are looked up by exact version, since another version may lay
/// out its vtable differently.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the {module} does not export `{}`; the server may not match the SDK the bindings were generated from", .version.to_string_lossy())]
pub struct InterfaceError {
	module: Module,
	version: CString,
}

impl InterfaceError {
	/// The module that was asked for the interface.
	pub const fn module(&self) -> Module {
		self.module
	}

	/// The version string the interface was requested by.
	pub fn version(&self) -> &CStr {
		&self.version
	}
}

/// One module's `CreateInterface` function.
///
/// Holding a factory grants nothing on its own; [`Server::new`] is where the
/// caller vouches that it belongs to the running server.
#[doc(alias("CreateInterface", "CreateInterfaceFn"))]
#[derive(Debug, Clone, Copy)]
pub struct InterfaceFactory(CreateInterfaceFn);

impl InterfaceFactory {
	/// Wraps a module's `CreateInterface` function.
	pub const fn new(factory: CreateInterfaceFn) -> Self {
		Self(factory)
	}

	/// Converts the SDK's nullable `CreateInterfaceFn`, rejecting null.
	pub const fn from_raw(factory: sys::CreateInterfaceFn) -> Option<Self> {
		match factory {
			Some(factory) => Some(Self(factory)),
			None => None,
		}
	}

	/// Returns the wrapped `CreateInterface` function.
	pub const fn as_raw(self) -> CreateInterfaceFn {
		self.0
	}
}

/// The module whose factory exports an interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Module {
	/// The engine library, `engine.dll` or `engine.so`.
	Engine,

	/// The game's server library, `server.dll` or `server.so`.
	GameServer,
}

impl Display for Module {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Engine => "engine",
			Self::GameServer => "game server",
		})
	}
}

/// The running server, as seen from one call the engine makes into a plugin.
///
/// Every other type in this crate is reached from here. A `Server` holds the
/// engine's and game server's interface factories, the only objects in Source
/// from which every interface is reachable, and resolves an interface each
/// time an accessor is called. Interfaces are process-wide singletons, so each
/// accessor returns the same object; resolving is a short string search. An
/// accessor returns an [`InterfaceError`] if the module does not export its
/// interface at the version these bindings were generated for.
///
/// The lifetime `'s` is the scope [`Server::new`] vouches for. Handles derived
/// from a `Server` carry it, so none can outlive the callback that created it.
#[derive(Debug, Clone, Copy)]
pub struct Server<'s> {
	engine: InterfaceFactory,
	game_server: InterfaceFactory,
	game: Game,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> Server<'s> {
	/// Wraps the running server's interface factories for the scope `'s`.
	///
	/// This is the only function the rest of the crate's safety rests on.
	///
	/// # Safety
	///
	/// For all of `'s`:
	///
	/// 1. `engine` and `game_server` are the `CreateInterface` functions of the
	///    running server's engine and game-server modules, neither of which is
	///    unloaded.
	/// 2. The game DLL was built for `game`.
	/// 3. `'s` lies within a single call from the engine into the plugin, on
	///    the server's main thread. No frame, level change, or shutdown completes
	///    during it, so the edict table, entity list, and string pools stay
	///    allocated.
	/// 4. Code that runs during `'s`, including engine, game, and other
	///    plugins' code reached through calls made in `'s`, frees entities only
	///    through the engine's deferred deletion (`UTIL_Remove`), never
	///    immediately (`UTIL_RemoveImmediate`, `RemoveEntityImmediate`, or
	///    `RemoveEdict`). This includes map scripts (VScript) the game runs
	///    during `'s`, and rules out restarting the round, which frees nearly
	///    every entity. The crate's safe functions refuse the calls known to
	///    break this whatever the map does, such as inputs that run code the
	///    caller chooses or spawn entity templates.
	/// 5. Even if it is unregistered, a console command or variable that the
	///    `ICvar` registry lists at any point during `'s` stays allocated, with
	///    its name unchanged, until `'s` ends, and the module that declared it
	///    stays loaded. As in 4, this includes code reached through calls made
	///    in `'s`, such as another plugin's change callback that unloads a
	///    plugin: unloading a Metamod:Source plugin unmaps the commands and
	///    variables it declared, and unloading a SourceMod plugin can free the
	///    commands it created, with their names.
	pub const unsafe fn new<S: ?Sized>(
		engine: InterfaceFactory,
		game_server: InterfaceFactory,
		game: Game,
		_scope: &'s S,
	) -> Self {
		Self {
			engine,
			game_server,
			game,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// `IBotManager`, which creates bots.
	pub fn bot_manager(&self) -> Result<BotManager<'s>, InterfaceError> {
		self.interface()
	}

	/// Prints to the server console, which rcon's redirection also receives.
	///
	/// This goes through tier0's `Msg`, as the game's own console commands do.
	/// If tier0 cannot be found, it falls back to `ICvar::ConsolePrintf`, which
	/// only listen servers display.
	#[doc(alias("Msg"))]
	pub fn console_print(&self, message: &CStr) {
		if tier0_print(message) {
			return;
		}

		if let Ok(cvar) = self.cvar() {
			cvar.console_printf(message);
		}
	}

	/// `ICvar`, the console variable and command registry.
	pub fn cvar(&self) -> Result<Cvar<'s>, InterfaceError> {
		self.interface()
	}

	/// The engine module's interface factory, as passed to [`Server::new`].
	pub const fn engine_factory(&self) -> InterfaceFactory {
		self.engine
	}

	/// `IEngineReplay`, the engine's services for its replay system, which
	/// recalculate the server's tags.
	///
	/// That the engine exports it as `EngineReplay001` is inferred from the
	/// header, which declares that version, and from TF2's client library,
	/// which requests it from the engine; the engine is not public. Like every
	/// interface, it fails should the engine not export it.
	pub fn engine_replay(&self) -> Result<EngineReplay<'s>, InterfaceError> {
		self.interface()
	}

	/// `IEngineSound`, the server's sound system.
	pub fn engine_sound(&self) -> Result<EngineSound<'s>, InterfaceError> {
		self.interface()
	}

	/// `IEngineTrace`, which traces rays and queries world contents.
	pub fn engine_trace(&self) -> Result<EngineTrace<'s>, InterfaceError> {
		self.interface()
	}

	/// Looks up an interface this crate does not wrap.
	///
	/// The pointer is valid for `'s` if `version` names an interface that the
	/// module implements as a singleton, which almost all are. Dereferencing
	/// it requires `T` to match that interface. Returns an [`InterfaceError`]
	/// if the module does not export `version`.
	pub fn find_interface<T>(
		&self,
		module: Module,
		version: &CStr,
	) -> Result<NonNull<T>, InterfaceError> {
		let factory = match module {
			Module::Engine => self.engine,
			Module::GameServer => self.game_server,
		};

		// SAFETY: `new` guarantees the factory is the module's `CreateInterface`,
		// which stays loaded for `'s`, on the server's main thread.
		let interface = unsafe { create_interface(factory.as_raw(), version) };

		interface.map(NonNull::cast).ok_or_else(|| InterfaceError {
			module,
			version: version.to_owned(),
		})
	}

	/// The game the server runs.
	pub const fn game(&self) -> Game {
		self.game
	}

	/// `IGameEventManager2`, which creates, fires, and listens for game events.
	pub fn game_events(&self) -> Result<GameEventManager<'s>, InterfaceError> {
		self.interface()
	}

	/// The game server module's interface factory, as passed to
	/// [`Server::new`].
	pub const fn game_server_factory(&self) -> InterfaceFactory {
		self.game_server
	}

	/// Looks up the interface `I` wraps and binds its handle to `'s`.
	fn interface<I: Interface<'s>>(&self) -> Result<I, InterfaceError> {
		let raw = self.find_interface::<I::Raw>(I::MODULE, I::VERSION)?;

		// SAFETY: `I::Raw` is the class exported under `I::VERSION`, and `new`
		// guarantees the module keeps the singleton alive for `'s`.
		Ok(unsafe { I::bind(raw, self) })
	}

	/// `IVModelInfo`, the server's model registry.
	pub fn model_info(&self) -> Result<ModelInfo<'s>, InterfaceError> {
		self.interface()
	}

	/// `INetworkStringTableContainer`, the server's network string tables.
	pub fn network_string_tables(&self) -> Result<NetworkStringTables<'s>, InterfaceError> {
		self.interface()
	}

	/// `IPlayerInfoManager`, which exposes player state and the engine globals.
	pub fn player_info_manager(&self) -> Result<PlayerInfoManager<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerPluginHelpers`, services for server plugins.
	pub fn plugin_helpers(&self) -> Result<PluginHelpers<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerGameClients`, the game's handling of connected clients.
	pub fn server_game_clients(&self) -> Result<ServerGameClients<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerGameDLL`, implemented by the game's `CServerGameDLL`.
	pub fn server_game_dll(&self) -> Result<ServerGameDll<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerGameEnts`, which converts between entities and edicts.
	pub fn server_game_ents(&self) -> Result<ServerGameEnts<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerGameTags`, the game's list of the console variables that tag
	/// the server.
	pub fn server_game_tags(&self) -> Result<ServerGameTags<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerTools`, which enumerates and manipulates entities.
	pub fn server_tools(&self) -> Result<ServerTools<'s>, InterfaceError> {
		self.interface()
	}

	/// `IVEngineServer`, the engine's services for the game server.
	pub fn valve_engine(&self) -> Result<ValveEngine<'s>, InterfaceError> {
		self.interface()
	}

	/// `IVoiceServer`, which routes voice between clients.
	pub fn voice_server(&self) -> Result<VoiceServer<'s>, InterfaceError> {
		self.interface()
	}

	/// Prints a foreground-colored console message through tier0. Color travels
	/// separately from the text; replicated/RCON strings contain no ANSI escapes.
	/// Engine windows may choose their own presentation of the requested color.
	/// Falls back to ordinary console output when tier0 lacks `ConColorMsg`.
	#[doc(alias("ConColorMsg"))]
	pub fn console_color_print(&self, color: sdk_raw::tier0::SpewColor, message: &CStr) {
		// SAFETY: Server's callback/main-thread contract keeps tier0 live here.
		if !unsafe { sdk_raw::tier0::color_print(color, message) } {
			self.console_print(message);
		}
	}
}

#[cfg(test)]
thread_local! {
	/// Stands in for tier0's `Msg` on this thread, since tests load no tier0.
	pub(crate) static TEST_MSG: Cell<Option<sdk_raw::tier0::MsgFn>> = const { Cell::new(None) };
}

/// The running server's interface factories, kept between the engine's calls
/// into a plugin.
///
/// Callbacks this crate implements for the engine, such as console commands,
/// use a binding to produce a [`Server`] scoped to each call. Creating a
/// binding is where the caller vouches for the factories once; each call site
/// that turns it into a [`Server`] vouches for its own scope.
#[derive(Debug, Clone, Copy)]
pub struct ServerBinding {
	engine: InterfaceFactory,
	game_server: InterfaceFactory,
	game: Game,
	_not_thread_safe: NotThreadSafe,
}

impl ServerBinding {
	/// Keeps the running server's interface factories for later calls.
	///
	/// # Safety
	///
	/// Conditions 1, 2, 4 and 5 of [`Server::new`] must hold during every call
	/// from the engine into the plugin in which this binding, or a copy, is
	/// turned into a [`Server`]. In practice: the factories belong to the
	/// running server, which the plugin is unloaded from before those modules
	/// are, and the game DLL was built for `game`.
	pub const unsafe fn new(
		engine: InterfaceFactory,
		game_server: InterfaceFactory,
		game: Game,
	) -> Self {
		Self {
			engine,
			game_server,
			game,
			_not_thread_safe: PhantomData,
		}
	}

	/// The game the server runs.
	pub const fn game(&self) -> Game {
		self.game
	}

	/// Produces a [`Server`] for the scope `'s`.
	///
	/// # Safety
	///
	/// Condition 3 of [`Server::new`]: `'s` lies within a single call from the
	/// engine into the plugin, on the server's main thread.
	pub const unsafe fn server<'s, S: ?Sized>(&self, scope: &'s S) -> Server<'s> {
		// SAFETY: `new` vouched for conditions 1, 2, 4 and 5, and the caller for
		// 3.
		unsafe { Server::new(self.engine, self.game_server, self.game, scope) }
	}
}

/// Prints through tier0's `Msg`, or in tests through the stand-in in
/// [`TEST_MSG`] if there is one. Returns `false` if tier0 is not loaded.
fn tier0_print(message: &CStr) -> bool {
	#[cfg(test)]
	if let Some(msg) = TEST_MSG.get() {
		// SAFETY: Tests install a `printf`-style stand-in.
		unsafe { sdk_raw::tier0::print_through(msg, message) };
		return true;
	}

	// SAFETY: Under `Server::new`'s contract, this runs on the server's main
	// thread inside an engine callback, where the engine's tier0 is loaded and
	// may print.
	unsafe { sdk_raw::tier0::print(message) }
}
