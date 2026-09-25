//! How commands are linked into the engine's registry.

use crate::interfaces::Cvar;
use std::ptr::NonNull;

/// Links console commands into, and unlinks them from, the engine's `ICvar`.
///
/// # Safety
///
/// `link` must pass `command` unchanged to `ICvar::RegisterConCommand`, and
/// `unlink` to `ICvar::UnregisterConCommand`, directly or through a host such
/// as Metamod:Source. Neither may keep the pointer except as the engine or
/// that host does.
pub unsafe trait CommandRegistrar {
	/// # Safety
	///
	/// `command` is a pinned, `'static` [`ConsoleCommand`](super::ConsoleCommand)
	/// ready to be linked, and this runs on the server's main thread.
	unsafe fn link(&self, command: NonNull<sys::ConCommandBase>);

	/// # Safety
	///
	/// As for [`Self::link`].
	unsafe fn unlink(&self, command: NonNull<sys::ConCommandBase>);
}

/// A registrar whose host unlinks every command linked through it before
/// unloading the module that contains the command's code.
///
/// This is what makes [`ConsoleCommand::register`] safe: the engine can never
/// call a command whose code is gone, even when the plugin forgets to
/// unregister it or its host unloads it by force.
///
/// # Safety
///
/// Every command linked through this registrar must be unlinked after the
/// plugin's last callback and before its library is unloaded, including when
/// the host refuses the plugin's load or unloads it by force.
///
/// [`ConsoleCommand::register`]: super::ConsoleCommand::register
pub unsafe trait UnlinksBeforeUnload: CommandRegistrar {}

// SAFETY: These are direct calls to the engine's registry. Nothing unlinks the
// commands on unload, so `Cvar` is not `UnlinksBeforeUnload`.
unsafe impl CommandRegistrar for Cvar<'_> {
	unsafe fn link(&self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: The caller upholds the contract, which covers the registry's.
		unsafe { self.register_con_command(command) };
	}

	unsafe fn unlink(&self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: As for `link`.
		unsafe { self.unregister_con_command(command) };
	}
}
