//! A hook on the temporary entities the server plays back, which can block
//! each one before the engine queues it for clients.
//!
//! Temporary entities are one-off effects that each client makes for itself
//! from a few networked fields, such as TF2's explosions (`DT_TETFExplosion`),
//! each of which plays an explosion's sound and particles, and its bullets
//! (`DT_TEFireBullets`), whose impacts, tracers and impact sounds each client
//! traces for itself. Blood, particle effects and decals are others.
//!
//! The hook patches `IVEngineServer::PlaybackTempEntity` on the engine's
//! interface, through which the game queues each one, as
//! `CBaseTempEntity::Create` does. The game's `ITempEntsSystem` creates its
//! effects that way too. The callback sees the name of the entity's send
//! table, which tells its kind, and the clients the game sends it to.
//!
//! # When to install
//!
//! The interface lives as long as the engine, so install while loading. The
//! hook stops calling back while the plugin is paused and when it unloads, and
//! Metamod removes it after unloading the plugin. With Metamod 2.0, when the
//! function is already detoured by another plugin, KHook adds the hook from its
//! worker thread, so the temporary entities just after an install can pass
//! unseen.
//!
//! # What gets through
//!
//! The callback runs only for temporary entities played back on the server's
//! main thread, and not while the plugin is paused or after it unloads. It does
//! not see:
//!
//! - effects clients make without one, such as those of their own predicted
//!   shots, which the game leaves out of the recipients;
//! - a temporary entity a hook running before this one blocked.
//!
//! A blocked temporary entity is never queued, so no client receives it.

#[cfg(test)]
#[path = "../tests/hooks/temp_entity.rs"]
mod tests;

use crate::MetamodApi;
use crate::recipients::Recipients;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::interfaces::ValveEngine;
use source_sdk_2013::raw::util::cstr::borrow_cstr;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

use source_sdk_2013::raw::interfaces::valve_engine::{
	PLAYBACK_TEMP_ENTITY_SLOT, PlaybackTempEntityFn as PlaybackTempEntity,
};

/// Decides whether a temporary entity the server plays back is sent. A panic
/// is contained by the hook dispatcher, and lets the temporary entity through.
pub type PlaybackTempEntityFn = for<'s> fn(Server<'s>, &PlayedTempEntity<'_>) -> TempEntityAction;

/// `IVEngineServer::PlaybackTempEntity`.
const PLAYBACK_TEMP_ENTITY: VirtualFunction<PlaybackTempEntity> =
	VirtualFunction::new(PLAYBACK_TEMP_ENTITY_SLOT);

static ROUTE: TempEntityRoute = TempEntityRoute::new();

/// A temporary entity the server is playing back, as a
/// [`PlaybackTempEntityFn`] sees it, with the arguments of
/// `PlaybackTempEntity`.
#[derive(Debug, Clone, Copy)]
pub struct PlayedTempEntity<'a> {
	/// The clients the engine sends it to.
	pub recipients: Recipients<'a>,

	/// The seconds clients wait after receiving it before they play it.
	pub delay: f32,

	/// The name of its send table, which tells its kind, such as
	/// `DT_TETFExplosion` or `DT_TEFireBullets`.
	pub table: &'a CStr,

	/// The index of its server class.
	pub class_id: c_int,
}

/// What to do with a temporary entity the server plays back.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TempEntityAction {
	/// Send it.
	#[default]
	Continue,

	/// Do not queue it for any client.
	Block,
}

#[derive(Clone, Copy)]
struct RoutedTempEntities {
	binding: ServerBinding,
	callback: PlaybackTempEntityFn,
	hook: HookId,
}

struct TempEntityRoute {
	state: Cell<Option<RoutedTempEntities>>,
}

impl TempEntityRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<PlaybackTempEntity> for TempEntityRoute {
	fn call(&self, call: &HookCall<'_, PlaybackTempEntity>) -> HookAction<()> {
		// An earlier hook blocked it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(routed) = self.state.get() else {
			return HookAction::Ignore;
		};

		let (filter, delay, _sender, table, class_id) = call.args();

		let (Some(filter), Some(table)) = (NonNull::new(filter), NonNull::new(table.cast_mut()))
		else {
			return HookAction::Ignore;
		};

		// SAFETY: Send tables and their names are statics of the game DLL. The
		// field is read without forming a reference.
		let Some(table) =
			(unsafe { borrow_cstr((&raw const (*table.as_ptr()).m_pNetTableName).read()) })
		else {
			return HookAction::Ignore;
		};

		let played = PlayedTempEntity {
			// SAFETY: The game's filter is live for the call, on the main thread,
			// and the temporary entity does not outlive the call.
			recipients: unsafe { Recipients::new(filter) },
			delay,
			table,
			class_id,
		};

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread, during one call
		// from the engine. The plugin supplied the binding with the hook.
		let server = unsafe { routed.binding.server(&scope) };

		match (routed.callback)(server, &played) {
			TempEntityAction::Continue => HookAction::Ignore,
			TempEntityAction::Block => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for TempEntityRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` before each temporary entity the server plays back
	/// through `engine`, which decides whether it is sent; see the
	/// [module documentation](crate::hooks::temp_entity).
	///
	/// This hooks `IVEngineServer::PlaybackTempEntity`. Installing again while
	/// the hook is installed returns [`HookError::AlreadyInstalled`].
	pub fn hook_playback_temp_entity(
		self,
		engine: ValveEngine<'_>,
		binding: ServerBinding,
		callback: PlaybackTempEntityFn,
	) -> Result<(), HookError> {
		let engine = NonNull::new(engine.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: The engine's interface lives as long as the engine, which
		// outlives the plugin, and has `PlaybackTempEntity` at the slot, which
		// the generated binding checks.
		unsafe { self.install_playback_temp_entity(engine, binding, callback) }
	}

	/// Hooks `PlaybackTempEntity` on `engine`.
	///
	/// # Safety
	///
	/// `engine` must be live, and its vtable must hold a function of the
	/// signature [`PlaybackTempEntity`] at [`PLAYBACK_TEMP_ENTITY_SLOT`] until
	/// Metamod unloads the plugin.
	unsafe fn install_playback_temp_entity(
		self,
		engine: NonNull<sys::IVEngineServer>,
		binding: ServerBinding,
		callback: PlaybackTempEntityFn,
	) -> Result<(), HookError> {
		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hook))
		{
			return Err(HookError::AlreadyInstalled);
		}

		// SAFETY: As the caller promises; a `MetamodApi` only exists on the main
		// thread.
		let hook = unsafe {
			self.add_hook(
				PLAYBACK_TEMP_ENTITY,
				HookTarget::instance(engine),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.state.set(Some(RoutedTempEntities {
			binding,
			callback,
			hook,
		}));
		Ok(())
	}
}
