//! Has TF2 answer that a player-versus-environment mode is active without
//! playing Mann vs. Machine, by patching Windows servers' game code.
//!
//! `CTFGameRules::IsPVEModeActive` answers whether the level plays Mann vs.
//! Machine (`m_bPlayingMannVsMachine`): shipping builds compile out the other
//! PvE modes it checks in the SDK's sources, so no variable or tag has it
//! answer yes otherwise. [`PveModePatch`] has it always answer yes. In TF2's
//! 64-bit Windows build, these ask it:
//!
//! - The `god` command, which runs, given `sv_cheats`, only in a PvE mode, as
//!   TF2 always sets `gpGlobals->deathmatch`.
//! - `CTFPlayer::HandleCommand_JoinTeam`, which changes bots' teams silently.
//! - `CTFPlayer::ClientCommand`, which lets dead players spend currency on
//!   `td_buyback`, a respawn, which nobody affords without Mann vs. Machine's
//!   currency.
//! - TFBots: Engineers go straight to their sentry's spot without building a
//!   teleporter entrance first, never move the sentry, and have unlimited
//!   metal while they build, as long as the cheat
//!   `tf_raid_engineer_infinte_metal` is on, as it is by default. Medics know
//!   where every teammate is as they choose whom to heal, and wait rather than
//!   retreat when nobody is left.
//! - Jarate, Mad Milk and the Gas Passer, whose hits stop counting for their
//!   strange counters, other than as robots slowed in Mann vs. Machine.
//! - VScript's `IsPVEModeActive`, and Mann vs. Machine's statistics, which
//!   exist only in that mode.
//! - `CTFGameRules::ShouldSkipAutoScramble`, which the linker folded into the
//!   same code, so automatic scrambles are skipped.
//!
//! The game rules' own checks read the flag inline, and so are not affected:
//! respawn times and waves, damage between teammates, the win panel, team
//! balancing, switching and scrambling, dominations, name changes, the tags of
//! `sv_tags`, and who counts as the enemy team for critical hits
//! (`IsPVEModeControlled`). Neither are clients, which only see the flag.
//!
//! Linux servers have no patch: [`PveModePatch::locate`] fails there.

use crate::interfaces::Cvar;

#[cfg(target_os = "windows")]
use sdk_raw::tf2::pve_mode::Patch;

use sdk_raw::tf2::pve_mode::PveModeError;
use sdk_raw::util::patch::{self, PatchOperation};

#[cfg(target_os = "windows")]
use sdk_raw::vcall;

use std::marker::PhantomData;
use std::rc::Rc;

/// Why TF2's PvE answer could not be patched or restored.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
	/// More than one gate in the `god` command passed every check, so none is
	/// patched.
	#[error("the god command holds more than one verified TF2 PvE gate")]
	AmbiguousGate,

	/// The game's code no longer matches what the guard verified or wrote. It
	/// is left unchanged.
	#[error("another component changed the PvE answer's instruction")]
	InstructionChanged,

	/// `server.dll`'s headers or mapped pages are not the committed, readable
	/// 64-bit PE image the scan expects.
	#[error("server.dll has an unsupported or inaccessible PE image")]
	InvalidImage,

	/// The engine has no `god` command, through which the answer is found.
	#[error("the engine does not register the `god` command")]
	MissingCommand,

	/// The `god` command does not have the callback, or the callback the gate
	/// and answer, verified in TF2's 64-bit Windows build, as with another
	/// game build or an already patched answer.
	#[error("server.dll does not contain the verified TF2 PvE gate")]
	UnsupportedGame,

	/// The server runs on Linux, which has no patch.
	#[error("TF2's PvE answer is only patched on Windows")]
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

impl From<PveModeError> for PatchError {
	fn from(error: PveModeError) -> Self {
		match error {
			PveModeError::AmbiguousGate => Self::AmbiguousGate,
			PveModeError::InvalidImage => Self::InvalidImage,

			PveModeError::ModuleNotLoaded(source) => Self::Windows {
				operation: "GetModuleHandleExW(server.dll)",
				source,
			},

			PveModeError::UnsupportedGame => Self::UnsupportedGame,
		}
	}
}

impl From<patch::PatchError> for PatchError {
	fn from(error: patch::PatchError) -> Self {
		match error {
			patch::PatchError::InstructionChanged => Self::InstructionChanged,

			patch::PatchError::Os { operation, source } => Self::Windows {
				operation: match operation {
					PatchOperation::FlushInstructionCache => "FlushInstructionCache(PvE answer)",
					PatchOperation::Reprotect => "VirtualProtect(PvE answer, original protection)",
					PatchOperation::Unprotect => "VirtualProtect(PvE answer, writable)",
				},
				source,
			},
		}
	}
}

/// Owns the single-byte Windows patch that has TF2 answer that a PvE mode is
/// active, until it is explicitly restored.
///
/// The server module remains loaded while this guard exists. Unknown
/// instruction sequences are refused. `Drop` attempts restoration, but callers
/// should call [`Self::restore`] to observe and handle errors before unloading.
#[doc(alias("IsPVEModeActive", "CC_God_f"))]
#[must_use = "keep the patch guard until the plugin restores TF2's own answer"]
pub struct PveModePatch {
	#[cfg(target_os = "windows")]
	patch: Patch,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl PveModePatch {
	/// Locates `CTFGameRules::IsPVEModeActive` in the currently loaded Windows
	/// server, through the `god` command that calls it, validating it without
	/// changing any instructions.
	///
	/// The signatures were verified in TF2's 64-bit Windows `CC_God_f` and
	/// `CTFGameRules::IsPVEModeActive`. The registered `god` must be a command
	/// with a plain callback in `server.dll`'s code, which must hold exactly
	/// one gate within its first bytes that reads the game rules and the
	/// globals, and calls the function, whose code must match.
	///
	/// Fails with [`PatchError::UnsupportedPlatform`] on Linux.
	///
	/// # Safety
	///
	/// The caller must be on the server's main thread in a TF2 dedicated-server
	/// callback, outside the game's calls of `IsPVEModeActive`. No other
	/// thread may execute or patch that function during installation or
	/// restoration. The guard must be restored and dropped under the same
	/// conditions, before the server shuts down. `cvar` must belong to that
	/// server's engine.
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

	/// Has TF2 answer that a PvE mode is active, retaining restoration
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
	/// executes or patches `IsPVEModeActive`.
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

	/// Restores TF2's own answer: the original instruction, page protection,
	/// and instruction cache.
	///
	/// This is idempotent after successful restoration. If another component
	/// changed the byte, it is left alone and an error is returned. A failed
	/// Windows operation can be retried while the guard remains alive.
	///
	/// # Safety
	///
	/// As for [`Self::locate`]: run on the server's main thread while nobody
	/// executes or patches `IsPVEModeActive`.
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
