//! Lets the `god` command run outside player-versus-environment modes, by
//! patching Windows servers' game code.
//!
//! TF2's `god` toggles god mode, given `sv_cheats`, only while
//! `CTFGameRules::IsPVEModeActive` answers yes, as TF2 always sets
//! `gpGlobals->deathmatch`. Shipping builds compile out the PvE modes other
//! than Mann vs. Machine that the function checks in the SDK's sources, so the
//! command runs only there. [`GodPatch`] has the command skip that check.
//!
//! Nothing else changes: `IsPVEModeActive` still answers as before to
//! everything else that asks it, such as TFBots, team changes, Strange
//! counters, automatic scrambles and VScript. `notarget`, which checks
//! `deathmatch` alone, stays refused, as it is in Mann vs. Machine.
//!
//! Linux servers have no patch: [`GodPatch::locate`] fails there.

use crate::interfaces::Cvar;
use sdk_raw::tf2::god::GodError;

#[cfg(target_os = "windows")]
use sdk_raw::tf2::god::Patch;

use sdk_raw::util::patch::{self, PatchOperation};

#[cfg(target_os = "windows")]
use sdk_raw::vcall;

use std::marker::PhantomData;
use std::rc::Rc;

/// Owns the single-byte Windows patch that lets the `god` command run outside
/// PvE modes, until it is explicitly restored.
///
/// The server module remains loaded while this guard exists. Unknown
/// instruction sequences are refused. `Drop` attempts restoration, but callers
/// should call [`Self::restore`] to observe and handle errors before unloading.
#[doc(alias("CC_God_f", "IsPVEModeActive"))]
#[must_use = "keep the patch guard until the plugin restores the god command's gate"]
pub struct GodPatch {
	#[cfg(target_os = "windows")]
	patch: Patch,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl GodPatch {
	/// Locates the gate of the `god` command in the currently loaded Windows
	/// server, validating it without changing any instructions.
	///
	/// The signatures were verified in TF2's 64-bit Windows `CC_God_f` and
	/// `CTFGameRules::IsPVEModeActive`. The registered `god` must be a command
	/// with a plain callback in `server.dll`'s code, which must hold exactly
	/// one gate within its first bytes that reads the game rules and the
	/// globals, and calls `IsPVEModeActive`, whose code must match.
	///
	/// Fails with [`PatchError::UnsupportedPlatform`] on Linux.
	///
	/// # Safety
	///
	/// The caller must be on the server's main thread in a TF2 dedicated-server
	/// callback, outside the `god` command. No other thread may execute or patch
	/// the command's callback during installation or restoration. The guard
	/// must be restored and dropped under the same conditions, before the server
	/// shuts down. `cvar` must belong to that server's engine.
	pub unsafe fn locate(cvar: Cvar<'_>) -> Result<Self, PatchError> {
		#[cfg(target_os = "windows")]
		{
			let base = cvar
				.find_command_base(c"god")
				.ok_or(PatchError::MissingCommand)?;

			// SAFETY: The registry holds live commands and variables, whose
			// code stays loaded, as `Cvar` guarantees for its scope.
			if !unsafe { vcall!(base.as_ptr() => ConCommandBase_IsCommand()) } {
				return Err(PatchError::MissingCommand);
			}

			let command = base.as_ptr().cast::<sys::ConCommand>();

			// SAFETY: `IsCommand` reported a `ConCommand`, which is live, as
			// above. Its bitfields tell which member of the callback's union it
			// holds, and are read without forming a reference.
			let callback = unsafe {
				if sys::ConCommand::m_bUsingNewCommandCallback_raw(command)
					|| sys::ConCommand::m_bUsingCommandCallbackInterface_raw(command)
				{
					return Err(PatchError::UnsupportedGame);
				}

				(&raw const (*command).__bindgen_anon_1.m_fnCommandCallbackV1).read()
			}
			.ok_or(PatchError::UnsupportedGame)?;

			// SAFETY: The caller upholds the main-thread and timing contract
			// for locating, enabling, restoring and dropping, which the guard
			// forwards to its patch.
			let patch = unsafe { Patch::locate(callback as usize) }?;

			Ok(Self {
				patch,
				_not_thread_safe: PhantomData,
			})
		}

		#[cfg(not(target_os = "windows"))]
		{
			let _ = cvar;

			Err(PatchError::UnsupportedPlatform)
		}
	}

	/// Lets the `god` command run outside PvE modes, retaining restoration
	/// ownership on any error.
	///
	/// If a Windows operation fails after writing the byte, this guard still
	/// owns the patch and any pending page-protection or cache cleanup. Call
	/// [`Self::restore`] before releasing it. Calling this again after
	/// successful enabling is idempotent while its installed instruction
	/// remains unchanged.
	///
	/// # Safety
	///
	/// As for [`Self::locate`]: run on the server's main thread while nobody
	/// executes or patches the `god` command's callback.
	pub unsafe fn enable(&mut self) -> Result<(), PatchError> {
		#[cfg(target_os = "windows")]
		{
			// SAFETY: The caller upholds the main-thread and timing contract.
			Ok(unsafe { self.patch.enable() }?)
		}

		#[cfg(not(target_os = "windows"))]
		Ok(())
	}

	/// Whether this guard has an outstanding installed patch.
	pub fn is_active(&self) -> bool {
		#[cfg(target_os = "windows")]
		return self.patch.is_active();

		#[cfg(not(target_os = "windows"))]
		false
	}

	/// Restores the `god` command's own gate: the original instruction, page
	/// protection, and instruction cache.
	///
	/// This is idempotent after successful restoration. If another component
	/// changed the byte, it is left alone and an error is returned. A failed
	/// Windows operation can be retried while the guard remains alive.
	///
	/// # Safety
	///
	/// As for [`Self::locate`]: run on the server's main thread while nobody
	/// executes or patches the `god` command's callback.
	pub unsafe fn restore(&mut self) -> Result<(), PatchError> {
		#[cfg(target_os = "windows")]
		{
			// SAFETY: The caller upholds the main-thread and timing contract.
			Ok(unsafe { self.patch.restore() }?)
		}

		#[cfg(not(target_os = "windows"))]
		Ok(())
	}
}

/// Why the `god` command's gate could not be patched or restored.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
	/// More than one gate in the `god` command passed every check, so none is
	/// patched.
	#[error("the god command holds more than one verified TF2 gate")]
	AmbiguousGate,

	/// The game's code no longer matches what the guard verified or wrote. It
	/// is left unchanged.
	#[error("another component changed the god command's gate")]
	InstructionChanged,

	/// `server.dll`'s headers or mapped pages are not the committed, readable
	/// 64-bit PE image the scan expects.
	#[error("server.dll has an unsupported or inaccessible PE image")]
	InvalidImage,

	/// The engine has no `god` command.
	#[error("the engine does not register the `god` command")]
	MissingCommand,

	/// The `god` command does not have the callback, or the callback the gate,
	/// verified in TF2's 64-bit Windows build, as with another game build or
	/// an already patched gate.
	#[error("server.dll does not contain the verified TF2 god gate")]
	UnsupportedGame,

	/// The server runs on Linux, which has no patch.
	#[error("the god command's gate is only patched on Windows")]
	UnsupportedPlatform,

	/// A Windows API call failed. The OS error is both in the message and the
	/// error's [`source`](std::error::Error::source).
	#[error("{operation} failed: {source}")]
	Windows {
		/// The failed Windows function and what it was applied to.
		operation: &'static str,
		/// The error Windows reported for the call.
		#[source]
		source: std::io::Error,
	},
}

impl From<GodError> for PatchError {
	fn from(error: GodError) -> Self {
		match error {
			GodError::AmbiguousGate => Self::AmbiguousGate,
			GodError::InvalidImage => Self::InvalidImage,

			GodError::ModuleNotLoaded(source) => Self::Windows {
				operation: "GetModuleHandleExW(server.dll)",
				source,
			},

			GodError::UnsupportedGame => Self::UnsupportedGame,
		}
	}
}

impl From<patch::PatchError> for PatchError {
	fn from(error: patch::PatchError) -> Self {
		match error {
			patch::PatchError::InstructionChanged => Self::InstructionChanged,

			patch::PatchError::Os { operation, source } => Self::Windows {
				operation: match operation {
					PatchOperation::FlushInstructionCache => "FlushInstructionCache(god gate)",
					PatchOperation::Reprotect => "VirtualProtect(god gate, original protection)",
					PatchOperation::Unprotect => "VirtualProtect(god gate, writable)",
				},
				source,
			},
		}
	}
}
