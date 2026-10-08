//! A hook on the sounds the server emits, which can block each one before the
//! engine sends it to clients.
//!
//! The hook patches the overload of `IEngineSound::EmitSound` that takes a
//! sound level, on the engine's sound interface. The game emits its sound
//! script entries through it, such as those of `CBaseEntity::EmitSound`, so
//! the callback sees the sample a script entry chose, and the clients the game
//! sends it to.
//!
//! # When to install
//!
//! The interface lives as long as the engine, so install while loading. The
//! hook stops calling back while the plugin is paused and when it unloads, and
//! Metamod removes it after unloading the plugin. With Metamod 2.0, when the
//! function is already detoured by another plugin, KHook adds the hook from its
//! worker thread, so the sounds just after an install can pass unseen.
//!
//! # What gets through
//!
//! The callback runs only for sounds emitted on the server's main thread, and
//! not while the plugin is paused or after it unloads. It does not see:
//!
//! - sounds emitted through the overload that takes an attenuation instead of
//!   a sound level, sentences, or ambient sounds, such as an
//!   `ambient_generic`'s;
//! - sounds clients play on their own, such as predicted weapon sounds, and
//!   those clients play for networked state or game events, such as TF2's
//!   announcer lines from a `teamplay_broadcast_audio` event;
//! - a sound a hook running before this one blocked.
//!
//! The game sends the closed caption of a script entry apart from its sound,
//! so a client with captions on can still read a blocked sound's caption.

#[cfg(test)]
#[path = "tests/sound_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::recipients::Recipients;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::interfaces::EngineSound;
use source_sdk_2013::math::Vector;
use source_sdk_2013::raw::interfaces::engine_sound::{EMIT_SOUND_SLOT, EmitSoundFn as EmitSound};
use source_sdk_2013::raw::util::cstr::borrow_cstr;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

/// Decides whether a sound the server emits is sent. A panic is contained by
/// the hook dispatcher, and lets the sound through.
pub type EmitSoundFn = for<'s> fn(Server<'s>, &EmittedSound<'_>) -> EmitSoundAction;

/// `IEngineSound::EmitSound`, the overload that takes a sound level.
const EMIT_SOUND: VirtualFunction<EmitSound> = VirtualFunction::new(EMIT_SOUND_SLOT);

static ROUTE: SoundRoute = SoundRoute::new();

/// What to do with a sound the server emits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EmitSoundAction {
	/// Send it.
	#[default]
	Continue,

	/// Do not send it to any client.
	Block,
}

/// A sound the server is emitting, as an [`EmitSoundFn`] sees it, with the
/// arguments of `EmitSound`.
#[derive(Debug, Clone, Copy)]
pub struct EmittedSound<'a> {
	/// The clients the engine sends the sound to.
	pub recipients: SoundRecipients<'a>,

	/// The index of the entity the sound plays from, 0 for the world.
	pub entity: c_int,

	/// The channel the sound plays on, of the entity's.
	pub channel: c_int,

	/// The sample: a path under `sound/`, which can start with characters that
	/// tell clients how to play it, such as `*` for a sound streamed from disk
	/// or `)` for spatialized stereo.
	pub sample: &'a CStr,

	/// The volume, from 0 to 1.
	pub volume: f32,

	/// The sound level, in decibels.
	pub level: sys::soundlevel_t,

	/// The `SND_*` flags.
	pub flags: c_int,

	/// The pitch, where 100 is normal.
	pub pitch: c_int,

	/// The special DSP effect.
	pub special_dsp: c_int,

	/// Where the sound plays, instead of the entity's origin.
	pub origin: Option<Vector>,

	/// The direction the sound plays in.
	pub direction: Option<Vector>,

	/// The server time the sound starts at, for a delayed sound, or 0.
	pub sound_time: f32,

	/// The index of the entity that speaks the sound, or -1.
	pub speaker: c_int,
}

#[derive(Clone, Copy)]
struct RoutedSounds {
	binding: ServerBinding,
	callback: EmitSoundFn,
	hook: HookId,
}

/// The recipient filter of a sound the server is emitting, which lists the
/// clients the engine sends it to. Valid only for the call.
pub type SoundRecipients<'a> = Recipients<'a>;

struct SoundRoute {
	state: Cell<Option<RoutedSounds>>,
}

impl SoundRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<EmitSound> for SoundRoute {
	fn call(&self, call: &HookCall<'_, EmitSound>) -> HookAction<()> {
		// An earlier hook blocked it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(routed) = self.state.get() else {
			return HookAction::Ignore;
		};

		let (
			filter,
			entity,
			channel,
			sample,
			volume,
			level,
			flags,
			pitch,
			special_dsp,
			origin,
			direction,
			_origins,
			_update_positions,
			sound_time,
			speaker,
		) = call.args();

		// SAFETY: The game passes a terminated sample, which outlives the call.
		let (Some(filter), Some(sample)) = (NonNull::new(filter), unsafe { borrow_cstr(sample) })
		else {
			return HookAction::Ignore;
		};

		// SAFETY: The game passes null or a live vector, which outlives the call.
		let vector =
			|vector: *const sys::Vector| unsafe { vector.as_ref() }.map(|&vector| vector.into());

		let sound = EmittedSound {
			// SAFETY: The game's filter is live for the call, on the main thread,
			// and the sound does not outlive the call.
			recipients: unsafe { Recipients::new(filter) },
			entity,
			channel,
			sample,
			volume,
			level,
			flags,
			pitch,
			special_dsp,
			origin: vector(origin),
			direction: vector(direction),
			sound_time,
			speaker,
		};

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread, during one call
		// from the engine. The plugin supplied the binding with the hook.
		let server = unsafe { routed.binding.server(&scope) };

		match (routed.callback)(server, &sound) {
			EmitSoundAction::Continue => HookAction::Ignore,
			EmitSoundAction::Block => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for SoundRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` before each sound the server emits through
	/// `engine_sound`, which decides whether the sound is sent; see the
	/// [module documentation](crate::sound_hooks).
	///
	/// This hooks `IEngineSound::EmitSound`, the overload that takes a sound
	/// level. Installing again while the hook is installed returns
	/// [`HookError::AlreadyInstalled`].
	pub fn hook_emit_sound(
		self,
		engine_sound: EngineSound<'_>,
		binding: ServerBinding,
		callback: EmitSoundFn,
	) -> Result<(), HookError> {
		let engine_sound = NonNull::new(engine_sound.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: The interface is the engine's, which outlives the plugin, and
		// has `EmitSound` at the slot.
		unsafe { self.install_emit_sound(engine_sound, binding, callback) }
	}

	/// Hooks `EmitSound` on `engine_sound`.
	///
	/// # Safety
	///
	/// `engine_sound` must be live, and its vtable must hold a function of the
	/// signature [`EmitSound`] at [`EMIT_SOUND_SLOT`] until Metamod unloads the
	/// plugin.
	unsafe fn install_emit_sound(
		self,
		engine_sound: NonNull<sys::IEngineSound>,
		binding: ServerBinding,
		callback: EmitSoundFn,
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
				EMIT_SOUND,
				HookTarget::instance(engine_sound),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.state.set(Some(RoutedSounds {
			binding,
			callback,
			hook,
		}));
		Ok(())
	}
}
