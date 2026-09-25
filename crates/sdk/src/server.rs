//! The crate's entry point, through which every other interface is reached.

use crate::ffi::NotThreadSafe;
use crate::interfaces::{
	BotManager, Cvar, EngineSound, EngineTrace, GameEventManager, ModelInfo, NetworkStringTables,
	PlayerInfoManager, PluginHelpers, ServerGameClients, ServerGameDll, ServerGameEnts,
	ServerTools, ValveEngine, VoiceServer,
};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt::{self, Debug, Display, Formatter};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The signature of `CreateInterface`, which every Source module exports to
/// hand out the interfaces it implements.
pub type RawInterfaceFactory =
	unsafe extern "C" fn(name: *const c_char, return_code: *mut c_int) -> *mut c_void;

/// One module's `CreateInterface` function.
///
/// Holding a factory grants nothing on its own; [`Server::new`] is where the
/// caller vouches that it belongs to the running server.
#[doc(alias = "CreateInterfaceFn")]
#[derive(Clone, Copy)]
pub struct InterfaceFactory(RawInterfaceFactory);

impl InterfaceFactory {
	pub const fn new(factory: RawInterfaceFactory) -> Self {
		Self(factory)
	}

	/// Converts the SDK's nullable `CreateInterfaceFn`, rejecting null.
	pub const fn from_raw(factory: sys::CreateInterfaceFn) -> Option<Self> {
		match factory {
			Some(factory) => Some(Self(factory)),
			None => None,
		}
	}

	pub const fn as_raw(self) -> RawInterfaceFactory {
		self.0
	}
}

impl Debug for InterfaceFactory {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "InterfaceFactory({:p})", self.0)
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
	pub const fn module(&self) -> Module {
		self.module
	}

	pub fn version(&self) -> &CStr {
		&self.version
	}
}

/// The game a server runs, which decides the ABI details that the SDK headers
/// alone cannot describe, such as virtual methods added under `TF_DLL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Game {
	/// Team Fortress 2, whose game DLL is built with `TF_DLL`.
	TeamFortress2,

	/// A Source SDK 2013 mod built without game-specific virtual methods.
	SourceSdk2013,
}

impl Game {
	/// `CBaseEntity::Teleport` in the game DLL's primary `CBaseEntity` vtable.
	pub(crate) const fn teleport_vtable_slot(self) -> usize {
		match self {
			Self::TeamFortress2 => sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT,
			Self::SourceSdk2013 => sys::CBASEENTITY_TELEPORT_VTABLE_SLOT,
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
	type Raw;

	const MODULE: Module;
	const VERSION: &'static CStr;

	/// # Safety
	///
	/// `raw` must be the live object exported under [`Self::VERSION`], alive
	/// for `'s`.
	unsafe fn bind(raw: NonNull<Self::Raw>, server: &Server<'s>) -> Self;
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
	/// Conditions 1, 2 and 4 of [`Server::new`] must hold during every call
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
		// SAFETY: `new` vouched for conditions 1, 2 and 4, and the caller for 3.
		unsafe { Server::new(self.engine, self.game_server, self.game, scope) }
	}
}

/// The running server, as seen from one call the engine makes into a plugin.
///
/// Every other type in this crate is reached from here. A `Server` holds the
/// engine's and game server's interface factories, the only objects in Source
/// from which every interface is reachable, and resolves an interface each
/// time an accessor is called. Interfaces are process-wide singletons, so each
/// accessor returns the same object; resolving is a short string search.
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
	///    `RemoveEdict`).
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

	/// The game the server runs.
	pub const fn game(&self) -> Game {
		self.game
	}

	pub const fn engine_factory(&self) -> InterfaceFactory {
		self.engine
	}

	pub const fn game_server_factory(&self) -> InterfaceFactory {
		self.game_server
	}

	/// Prints to the server console, which rcon's redirection also receives.
	///
	/// This goes through tier0's `Msg`, as the game's own console commands do.
	/// If tier0 cannot be found, it falls back to `ICvar::ConsolePrintf`, which
	/// only listen servers display.
	#[doc(alias = "Msg")]
	pub fn console_print(&self, message: &CStr) {
		if crate::tier0::print(message) {
			return;
		}

		if let Ok(cvar) = self.cvar() {
			cvar.console_printf(message);
		}
	}

	/// Looks up an interface this crate does not wrap.
	///
	/// The pointer is valid for `'s` if `version` names an interface that the
	/// module implements as a singleton, which almost all are. Dereferencing
	/// it requires `T` to match that interface.
	pub fn find_interface<T>(
		&self,
		module: Module,
		version: &CStr,
	) -> Result<NonNull<T>, InterfaceError> {
		let factory = match module {
			Module::Engine => self.engine,
			Module::GameServer => self.game_server,
		};

		let mut return_code = 0;

		// SAFETY: `new` guarantees the factory is the module's `CreateInterface`.
		let interface = unsafe { factory.as_raw()(version.as_ptr(), &mut return_code) };

		NonNull::new(interface.cast()).ok_or_else(|| InterfaceError {
			module,
			version: version.to_owned(),
		})
	}

	fn interface<I: Interface<'s>>(&self) -> Result<I, InterfaceError> {
		let raw = self.find_interface::<I::Raw>(I::MODULE, I::VERSION)?;

		// SAFETY: `I::Raw` is the class exported under `I::VERSION`, and `new`
		// guarantees the module keeps the singleton alive for `'s`.
		Ok(unsafe { I::bind(raw, self) })
	}

	/// `IVEngineServer`, the engine's services for the game server.
	pub fn valve_engine(&self) -> Result<ValveEngine<'s>, InterfaceError> {
		self.interface()
	}

	/// `IGameEventManager2`, which creates, fires, and listens for game events.
	pub fn game_events(&self) -> Result<GameEventManager<'s>, InterfaceError> {
		self.interface()
	}

	/// `ICvar`, the console variable and command registry.
	pub fn cvar(&self) -> Result<Cvar<'s>, InterfaceError> {
		self.interface()
	}

	/// `IEngineTrace`, which traces rays and queries world contents.
	pub fn engine_trace(&self) -> Result<EngineTrace<'s>, InterfaceError> {
		self.interface()
	}

	/// `IEngineSound`, the server's sound system.
	pub fn engine_sound(&self) -> Result<EngineSound<'s>, InterfaceError> {
		self.interface()
	}

	/// `INetworkStringTableContainer`, the server's network string tables.
	pub fn network_string_tables(&self) -> Result<NetworkStringTables<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerPluginHelpers`, services for server plugins.
	pub fn plugin_helpers(&self) -> Result<PluginHelpers<'s>, InterfaceError> {
		self.interface()
	}

	/// `IVModelInfo`, the server's model registry.
	pub fn model_info(&self) -> Result<ModelInfo<'s>, InterfaceError> {
		self.interface()
	}

	/// `IVoiceServer`, which routes voice between clients.
	pub fn voice_server(&self) -> Result<VoiceServer<'s>, InterfaceError> {
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

	/// `IServerGameClients`, the game's handling of connected clients.
	pub fn server_game_clients(&self) -> Result<ServerGameClients<'s>, InterfaceError> {
		self.interface()
	}

	/// `IPlayerInfoManager`, which exposes player state and the engine globals.
	pub fn player_info_manager(&self) -> Result<PlayerInfoManager<'s>, InterfaceError> {
		self.interface()
	}

	/// `IBotManager`, which creates bots.
	pub fn bot_manager(&self) -> Result<BotManager<'s>, InterfaceError> {
		self.interface()
	}

	/// `IServerTools`, which enumerates and manipulates entities.
	pub fn server_tools(&self) -> Result<ServerTools<'s>, InterfaceError> {
		self.interface()
	}
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use std::cell::RefCell;

	thread_local! {
		static INTERFACES: RefCell<Vec<(Module, CString, usize)>> = const { RefCell::new(Vec::new()) };
	}

	fn find(module: Module, name: *const c_char) -> *mut c_void {
		// SAFETY: Factories are called with NUL-terminated names.
		let name = unsafe { CStr::from_ptr(name) };

		INTERFACES.with_borrow(|interfaces| {
			interfaces
				.iter()
				.find(|(owner, version, _)| *owner == module && version.as_c_str() == name)
				.map_or(std::ptr::null_mut(), |&(_, _, address)| {
					address as *mut c_void
				})
		})
	}

	unsafe extern "C" fn engine_factory(
		name: *const c_char,
		_return_code: *mut c_int,
	) -> *mut c_void {
		find(Module::Engine, name)
	}

	unsafe extern "C" fn game_server_factory(
		name: *const c_char,
		_return_code: *mut c_int,
	) -> *mut c_void {
		find(Module::GameServer, name)
	}

	/// Makes the mock factories of [`mock_server`] export an interface.
	pub(crate) fn export<T>(module: Module, version: &CStr, interface: *mut T) {
		INTERFACES.with_borrow_mut(|interfaces| {
			interfaces.push((module, version.to_owned(), interface as usize))
		});
	}

	/// A server whose factories export only what [`export`] registered on this thread.
	pub(crate) fn mock_server<S: ?Sized>(scope: &S) -> Server<'_> {
		// SAFETY: Tests only export objects that outlive the scope they pass.
		unsafe { mock_binding().server(scope) }
	}

	/// A binding to the factories of [`mock_server`].
	pub(crate) fn mock_binding() -> ServerBinding {
		// SAFETY: Tests only export objects that outlive their use of the binding.
		unsafe {
			ServerBinding::new(
				InterfaceFactory::new(engine_factory),
				InterfaceFactory::new(game_server_factory),
				Game::TeamFortress2,
			)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::test_support::{export, mock_server};
	use super::*;

	#[test]
	fn interfaces_resolve_by_exact_version_from_their_own_module() {
		let mut engine = sys::IVEngineServer {
			vtable_: std::ptr::null(),
		};
		let engine_pointer = &raw mut engine;
		let scope = ();

		// Exported by the wrong module, and at a newer version: neither may bind.
		export(Module::GameServer, c"VEngineServer023", engine_pointer);
		export(Module::Engine, c"VEngineServer024", engine_pointer);

		let server = mock_server(&scope);
		let error = server.valve_engine().unwrap_err();

		assert_eq!(error.module(), Module::Engine);
		assert_eq!(error.version(), c"VEngineServer023");
		assert_eq!(
			error.to_string(),
			"the engine does not export `VEngineServer023`; the server may not match the SDK the bindings were generated from"
		);

		export(Module::Engine, c"VEngineServer023", engine_pointer);

		assert_eq!(server.valve_engine().unwrap().as_ptr(), engine_pointer);
		assert_eq!(
			server
				.find_interface::<sys::IVEngineServer>(Module::Engine, c"VEngineServer023")
				.unwrap()
				.as_ptr(),
			engine_pointer
		);
		assert!(server.server_game_dll().is_err());
	}
}
