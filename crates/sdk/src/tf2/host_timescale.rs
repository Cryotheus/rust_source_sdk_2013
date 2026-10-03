//! A narrowly checked workaround for TF2's Windows `host_timescale` cheat gate.
//!
//! Linux has no corresponding gate. This does not change `sv_cheats`, console
//! flags, or clients' replicated variables. It only bypasses the Windows
//! engine's check immediately before applying a positive `host_timescale`.

use crate::interfaces::Cvar;

#[cfg(target_os = "windows")]
use sdk_raw::tf2::host_timescale::Gate;

use sdk_raw::tf2::host_timescale::GateError;
use sdk_raw::util::patch::{self, PatchOperation};
use std::marker::PhantomData;
use std::rc::Rc;

/// Owns the single-byte Windows workaround until it is explicitly restored.
///
/// The Windows engine module remains loaded while this guard exists. Unknown
/// instruction sequences are refused. `Drop` attempts restoration, but callers
/// should call [`Self::restore`] to observe and handle errors before unloading.
/// On Linux this is a no-op guard.
#[doc(alias("Host_AccumulateTime"))]
#[must_use = "keep the patch guard until the plugin restores normal time scaling"]
pub struct HostTimescalePatch {
	#[cfg(target_os = "windows")]
	gate: Gate,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl HostTimescalePatch {
	/// Locates the workaround's target in the currently loaded Windows engine.
	/// This validates and retains the image without changing instructions.
	///
	/// The signature was verified in TF2's 64-bit Windows `Host_AccumulateTime`.
	/// Both references to `host_timescale` and the reference to `sv_cheats` must
	/// resolve to the registered variables' actual `m_pParent` fields. The two
	/// rejection branches must lead to the same verified continuation, and the
	/// replacement branch must lead to the float load. Exactly one match is
	/// required across the engine's readable executable sections.
	///
	/// # Safety
	///
	/// The caller must be on the server's main thread in a TF2 dedicated-server
	/// callback, outside execution of `Host_AccumulateTime`. No other thread may
	/// execute or patch that routine during installation or restoration. The
	/// guard must be restored and dropped under the same conditions, before
	/// engine shutdown. `cvar` must belong to that server's engine.
	pub unsafe fn locate(cvar: Cvar<'_>) -> Result<Self, PatchError> {
		#[cfg(target_os = "windows")]
		let gate = {
			let timescale = cvar
				.find_var(c"host_timescale")
				.ok_or(PatchError::MissingVariable("host_timescale"))?;
			let cheats = cvar
				.find_var(c"sv_cheats")
				.ok_or(PatchError::MissingVariable("sv_cheats"))?;

			// SAFETY: The caller upholds the engine-thread and timing contract
			// for locating, enabling, restoring and dropping, which the guard
			// forwards to its gate.
			unsafe { Gate::locate(timescale.as_ptr(), cheats.as_ptr()) }?
		};
		#[cfg(not(target_os = "windows"))]
		let _ = cvar;

		Ok(Self {
			#[cfg(target_os = "windows")]
			gate,
			_not_thread_safe: PhantomData,
		})
	}

	/// Enables the workaround, retaining restoration ownership on any error.
	///
	/// If a Windows operation fails after writing the byte, this guard still
	/// owns the patch and any pending page-protection or cache cleanup. Call
	/// [`Self::restore`] before releasing it. Calling this again after successful
	/// enabling is idempotent while its installed instruction remains unchanged.
	///
	/// # Safety
	///
	/// As for [`Self::locate`]: run on the server's main thread while nobody
	/// executes or patches `Host_AccumulateTime`.
	pub unsafe fn enable(&mut self) -> Result<(), PatchError> {
		#[cfg(target_os = "windows")]
		{
			// SAFETY: The caller upholds the engine-thread and timing contract.
			Ok(unsafe { self.gate.enable() }?)
		}
		#[cfg(not(target_os = "windows"))]
		Ok(())
	}

	/// Whether this guard has an outstanding installed Windows patch.
	///
	/// This is always `false` on Linux.
	pub fn is_active(&self) -> bool {
		#[cfg(target_os = "windows")]
		return self.gate.is_active();
		#[cfg(not(target_os = "windows"))]
		false
	}

	/// Restores the original instruction, page protection, and instruction cache.
	///
	/// This is idempotent after successful restoration. If another component
	/// changed the byte, it is left alone and an error is returned. A failed
	/// Windows operation can be retried while the guard remains alive.
	///
	/// # Safety
	///
	/// As for [`Self::locate`]: run on the server's main thread while nobody
	/// executes or patches `Host_AccumulateTime`.
	pub unsafe fn restore(&mut self) -> Result<(), PatchError> {
		#[cfg(target_os = "windows")]
		{
			// SAFETY: The caller upholds the engine-thread and timing contract.
			Ok(unsafe { self.gate.restore() }?)
		}
		#[cfg(not(target_os = "windows"))]
		Ok(())
	}
}

/// Why the engine's time-scale workaround could not be installed or restored.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
	/// The engine has no console variable with this name, so the gate's
	/// references to it cannot be verified.
	#[error("the engine does not register `{0}`")]
	MissingVariable(&'static str),

	/// `engine.dll`'s headers or mapped pages are not the committed, readable
	/// 64-bit PE image the scan expects.
	#[error("engine.dll has an unsupported or inaccessible PE image")]
	InvalidImage,

	/// `host_timescale` or `sv_cheats` is not in `engine.dll`'s data sections,
	/// or no candidate passed every signature and reference check, as with
	/// another engine build or an already patched gate.
	#[error("engine.dll does not contain the verified TF2 time-scale gate")]
	UnsupportedEngine,

	/// More than one candidate passed every check, so none is patched.
	#[error("engine.dll contains more than one verified TF2 time-scale gate")]
	AmbiguousGate,

	/// The gate's instructions no longer match what the guard verified or
	/// wrote. They are left unchanged.
	#[error("another component changed the time-scale gate's instruction")]
	InstructionChanged,

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

impl From<GateError> for PatchError {
	fn from(error: GateError) -> Self {
		match error {
			GateError::AmbiguousGate => Self::AmbiguousGate,
			GateError::InvalidImage => Self::InvalidImage,

			GateError::ModuleNotLoaded(source) => Self::Windows {
				operation: "GetModuleHandleExW(engine.dll)",
				source,
			},

			GateError::UnsupportedEngine => Self::UnsupportedEngine,
		}
	}
}

impl From<patch::PatchError> for PatchError {
	fn from(error: patch::PatchError) -> Self {
		match error {
			patch::PatchError::InstructionChanged => Self::InstructionChanged,

			patch::PatchError::Os { operation, source } => Self::Windows {
				operation: match operation {
					PatchOperation::FlushInstructionCache => {
						"FlushInstructionCache(time-scale gate)"
					}

					PatchOperation::Reprotect => {
						"VirtualProtect(time-scale gate, original protection)"
					}

					PatchOperation::Unprotect => "VirtualProtect(time-scale gate, writable)",
				},
				source,
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn raw_errors_keep_their_messages() {
		let error = PatchError::from(GateError::ModuleNotLoaded(std::io::Error::other("reason")));
		assert_eq!(
			error.to_string(),
			"GetModuleHandleExW(engine.dll) failed: reason"
		);

		for (operation, message) in [
			(
				PatchOperation::FlushInstructionCache,
				"FlushInstructionCache(time-scale gate) failed: reason",
			),
			(
				PatchOperation::Reprotect,
				"VirtualProtect(time-scale gate, original protection) failed: reason",
			),
			(
				PatchOperation::Unprotect,
				"VirtualProtect(time-scale gate, writable) failed: reason",
			),
		] {
			let source = std::io::Error::other("reason");
			let error = PatchError::from(patch::PatchError::Os { operation, source });
			assert_eq!(error.to_string(), message);
		}

		assert!(matches!(
			PatchError::from(patch::PatchError::InstructionChanged),
			PatchError::InstructionChanged
		));
	}

	#[test]
	fn windows_errors_display_their_os_reason() {
		let error = PatchError::Windows {
			operation: "VirtualProtect",
			source: std::io::Error::other("reason"),
		};
		assert_eq!(error.to_string(), "VirtualProtect failed: reason");
	}
}
