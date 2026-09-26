//! Handles to the interfaces a server plugin can request from the engine and
//! game server, each obtained from a [`Server`](crate::Server).
//!
//! A handle is a copyable, non-owning pointer to a process-wide singleton,
//! bound to the scope of the `Server` that produced it. Methods take `self`
//! and never create Rust references into the object, since the engine mutates
//! it through its own pointers at any time.

/// Declares the handle type of an interface that needs no state beyond its
/// pointer.
macro_rules! interface {
	(
		$(#[$meta:meta])*
		$vis:vis struct $Name:ident($Raw:ty) = $Module:ident $version:literal;
	) => {
		$(#[$meta])*
		#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
		$vis struct $Name<'s> {
			raw: ::core::ptr::NonNull<$Raw>,
			_scope: ::core::marker::PhantomData<&'s ()>,
			_not_thread_safe: $crate::ffi::NotThreadSafe,
		}

		impl<'s> $Name<'s> {
			/// The version string this interface is requested by.
			pub const VERSION: &'static ::core::ffi::CStr = $version;

			/// # Safety
			///
			/// `raw` must be the live object exported under
			/// [`Self::VERSION`], alive for `'s`, and every call must happen
			/// on the server's main thread.
			pub(crate) const unsafe fn from_raw(raw: ::core::ptr::NonNull<$Raw>) -> Self {
				Self {
					raw,
					_scope: ::core::marker::PhantomData,
					_not_thread_safe: ::core::marker::PhantomData,
				}
			}

			/// Returns the interface pointer, for calls this crate does not wrap.
			pub const fn as_ptr(self) -> *mut $Raw {
				self.raw.as_ptr()
			}
		}

		// SAFETY: `$Raw` is the class exported under `$version`, and `bind`
		// only wraps the pointer.
		unsafe impl<'s> $crate::server::Interface<'s> for $Name<'s> {
			type Raw = $Raw;

			const MODULE: $crate::server::Module = $crate::server::Module::$Module;
			const VERSION: &'static ::core::ffi::CStr = $version;

			unsafe fn bind(raw: ::core::ptr::NonNull<$Raw>, _server: &$crate::server::Server<'s>) -> Self {
				// SAFETY: The caller upholds the same contract.
				unsafe { Self::from_raw(raw) }
			}
		}
	};
}

pub mod bot_manager;
pub mod cvar;
pub mod engine_sound;
pub mod engine_trace;
pub mod game_event;
pub mod game_server;
pub mod model_info;
pub mod network_string_tables;
pub mod player_info_manager;
pub mod plugin_helpers;
pub mod server_game_clients;
pub mod server_game_dll;
pub mod server_game_ents;
pub mod server_tools;
pub mod valve_engine;
pub mod voice_server;

pub use bot_manager::BotManager;
pub use cvar::Cvar;
pub use engine_sound::EngineSound;
pub use engine_trace::EngineTrace;
pub use game_event::GameEventManager;
pub use game_server::{GameClient, GameServer};
pub use model_info::ModelInfo;
pub use network_string_tables::NetworkStringTables;
pub use player_info_manager::PlayerInfoManager;
pub use plugin_helpers::PluginHelpers;
pub use server_game_clients::ServerGameClients;
pub use server_game_dll::ServerGameDll;
pub use server_game_ents::ServerGameEnts;
pub use server_tools::ServerTools;
pub use valve_engine::ValveEngine;
pub use voice_server::VoiceServer;
