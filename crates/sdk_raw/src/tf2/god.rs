//! The `god` command's gate, and the code patch that lets the command run
//! outside player-versus-environment modes.
//!
//! TF2's `god` toggles god mode, given `sv_cheats`, only while there are no
//! game rules, `CTFGameRules::IsPVEModeActive` answers yes, or
//! `gpGlobals->deathmatch` is clear. TF2 always sets `deathmatch`, and
//! shipping builds compile out the Raid and Boss Battle modes the function
//! also checks in the SDK's sources, so the command runs only in Mann vs.
//! Machine.
//!
//! The gate is located in the command's callback and patched in place: its
//! `jz` past the checks, taken while there are no game rules, becomes `jmp`,
//! so the command always runs. Nothing else changes: `IsPVEModeActive` still
//! answers every other caller as before. Linux has no patch, so the `Patch`
//! exists on Windows only.

#[cfg(test)]
#[path = "../tests/tf2/god.rs"]
mod tests;

#[cfg(any(target_os = "windows", test))]
use crate::entities::flags::FL_GODMODE;

#[cfg(target_os = "windows")]
use crate::util::Module;

#[cfg(target_os = "windows")]
use crate::util::patch::{BytePatch, PatchError};

#[cfg(any(target_os = "windows", test))]
use crate::util::{self, Image, Section, SignaturePattern, exact_u32, sig};

#[cfg(any(target_os = "windows", test))]
use std::mem::offset_of;

// The gate reads `gpGlobals->deathmatch`, and toggles `FL_GODMODE`, at these
// values.
#[cfg(any(target_os = "windows", test))]
const _: () = {
	assert!(matches!(
		GATE[DEATHMATCH_OFFSET],
		SignaturePattern::Exact(0x65)
	));
	assert!(offset_of!(sys::CGlobalVars, deathmatch) == 0x65);
	assert!(matches!(exact_u32(&GATE, TOGGLE_OFFSET + 1), Some(0x8000)));
	assert!(FL_GODMODE == 0x8000);
};

// The patched byte is the gate's original `jz`, whose displacement leads to
// the toggle.
#[cfg(any(target_os = "windows", test))]
const _: () = {
	assert!(matches!(
		GATE[PATCH_OFFSET],
		SignaturePattern::Exact(ORIGINAL)
	));
	assert!(matches!(
		GATE[PATCH_OFFSET + 1],
		SignaturePattern::Exact(0x16)
	));
	assert!(PATCH_OFFSET + 2 + 0x16 == TOGGLE_OFFSET);
	assert!(matches!(GATE[TOGGLE_OFFSET], SignaturePattern::Exact(0xba)));
};

/// Offset of the `deathmatch` field's displacement in [`GATE`].
#[cfg(any(target_os = "windows", test))]
const DEATHMATCH_OFFSET: usize = 30;

/// The `god` command's gate, in its callback, TF2 Windows x64. It skips to
/// the toggle while there are no game rules or a PvE mode is active, and
/// returns while `gpGlobals->deathmatch` is set otherwise. The patch turns its
/// first `jz` into `jmp`, so it always skips to the toggle.
/// Relocation-dependent displacements are masked, then checked semantically.
///
/// ```text
///  +0  mov rcx, [rip+rules]      ; g_pGameRules
///  +7  test rcx, rcx
/// +10  jz +34 (rel8 0x16)        ; PATCH_OFFSET
/// +12  call IsPVEModeActive
/// +17  test al, al
/// +19  jnz +34 (rel8 0x0d)
/// +21  mov rax, [rip+globals]    ; gpGlobals
/// +28  cmp byte [rax+0x65], 0    ; deathmatch
/// +32  jnz reject
/// +34  mov edx, 0x8000           ; TOGGLE_OFFSET: FL_GODMODE, to toggle
/// ```
#[cfg(any(target_os = "windows", test))]
const GATE: [SignaturePattern; GATE_LEN] = sig![
	0x48 0x8b 0x0d ? ? ? ?
	0x48 0x85 0xc9
	0x74 0x16
	0xe8 ? ? ? ?
	0x84 0xc0
	0x75 0x0d
	0x48 0x8b 0x05 ? ? ? ?
	0x80 0x78 0x65 0x00
	0x75 ?
	0xba 0x00 0x80 0x00 0x00
];

/// Length of [`GATE`].
#[cfg(any(target_os = "windows", test))]
const GATE_LEN: usize = 39;

/// How far into the `god` command's callback the gate may start.
#[cfg(any(target_os = "windows", test))]
const GATE_SEARCH_LEN: usize = 0x80;

/// `CTFGameRules::IsPVEModeActive`, TF2 Windows x64, once its flag's offset
/// is masked, which the gate must call. It is only checked, never patched.
///
/// ```text
///  +0  cmp byte [rcx+flag], 0    ; m_bPlayingMannVsMachine
///  +7  setnz al
/// +10  ret
/// ```
#[cfg(any(target_os = "windows", test))]
const IS_PVE_MODE_ACTIVE: [SignaturePattern; IS_PVE_MODE_ACTIVE_LEN] = sig![
	0x80 0xb9 ? ? ? ? 0x00
	0x0f 0x95 0xc0
	0xc3
];

/// Length of [`IS_PVE_MODE_ACTIVE`].
#[cfg(any(target_os = "windows", test))]
const IS_PVE_MODE_ACTIVE_LEN: usize = 11;

/// The gate's original `jz` opcode, taken while there are no game rules.
#[cfg(any(target_os = "windows", test))]
const ORIGINAL: u8 = 0x74;

/// Offset of the patched opcode from the start of [`GATE`].
#[cfg(any(target_os = "windows", test))]
const PATCH_OFFSET: usize = 10;

/// The `jmp` opcode that replaces [`ORIGINAL`], keeping its displacement, so
/// the gate always skips to the toggle.
#[cfg(any(target_os = "windows", test))]
const REPLACEMENT: u8 = 0xeb;

/// The name of the module holding the gate.
#[cfg(target_os = "windows")]
const SERVER: &str = "server.dll";

/// Offset of the toggle, where the gate lets the command through, from the
/// start of [`GATE`].
#[cfg(any(target_os = "windows", test))]
const TOGGLE_OFFSET: usize = 34;

/// Why the `god` command's gate could not be located.
#[derive(Debug, thiserror::Error)]
pub enum GodError {
	/// More than one gate passed every check, so none is patched.
	#[error("the god command's callback holds more than one verified gate")]
	AmbiguousGate,

	/// `server.dll`'s headers or mapped pages are not the readable 64-bit PE
	/// image the scan expects.
	#[error("server.dll has an unsupported or inaccessible PE image")]
	InvalidImage,

	/// Windows could not find a loaded `server.dll`, and reported this error.
	#[error("server.dll is not loaded: {0}")]
	ModuleNotLoaded(#[source] std::io::Error),

	/// The `god` command's callback is not in `server.dll`'s code, or no
	/// gate in it passed every signature and reference check, as with
	/// another game build or an already patched gate.
	#[error("server.dll does not contain the verified TF2 god gate")]
	UnsupportedGame,
}

/// The located gate of the `god` command in the loaded `server.dll`, and the
/// patch that has it always let the command through.
///
/// The server module stays loaded while this exists. Dropping it attempts
/// [`Self::restore`], ignoring failures; call that first to observe and
/// handle them.
#[cfg(target_os = "windows")]
#[cfg_attr(docsrs, doc(cfg(target_os = "windows")))]
#[derive(Debug)]
pub struct Patch {
	/// The patch, which is dropped, restoring the gate, before the module is
	/// released.
	patch: BytePatch,

	/// Keeps `server.dll`, and so the patched code, loaded.
	_module: Module,
}

#[cfg(target_os = "windows")]
impl Patch {
	/// Locates the gate of the `god` command's callback at `god` in the loaded
	/// `server.dll`, validating it without changing any instructions.
	///
	/// The signatures were verified in TF2's 64-bit Windows `CC_God_f` and
	/// `CTFGameRules::IsPVEModeActive`. The callback must lie in the module's
	/// executable sections, and hold exactly one gate within its first bytes,
	/// whose references to the game rules and the globals lie in its data
	/// sections, and whose call leads to `IsPVEModeActive`'s code.
	///
	/// # Safety
	///
	/// The patch changes the game's code when enabled, restored, or dropped.
	/// Each of those, and this call, must happen while no thread executes or
	/// modifies the gate, such as on the server's main thread outside the
	/// game's calls, with no other thread patching it.
	pub unsafe fn locate(god: usize) -> Result<Self, GodError> {
		let module = Module::loaded(SERVER).map_err(|error| match error {
			util::Error::Io(error) => GodError::ModuleNotLoaded(error),
			util::Error::InvalidImage => GodError::InvalidImage,
		})?;

		let image = Image::from_module(&module).map_err(|_| GodError::InvalidImage)?;
		let gate = find_gate(&image, god)?;

		let verified = code(&image, gate, GATE_LEN).ok_or(GodError::InvalidImage)?;
		let block = std::ptr::with_exposed_provenance_mut::<u8>(gate);
		let block = std::ptr::NonNull::new(block).ok_or(GodError::InvalidImage)?;

		// SAFETY: The block is the verified code in `server.dll`, which the
		// patch keeps loaded for as long as the byte patch, which it drops
		// first, exists. The caller excludes execution and modification of
		// the gate whenever the patch is dropped.
		let patch = unsafe { BytePatch::new(block, verified.into(), PATCH_OFFSET, REPLACEMENT) }
			.ok_or(GodError::InvalidImage)?;

		Ok(Self {
			patch,
			_module: module,
		})
	}

	/// Has the gate always let the command through, retaining restoration
	/// ownership on any error.
	///
	/// If a Windows call fails after writing the byte, the patch still owns
	/// it and any pending page-protection or cache cleanup; call
	/// [`Self::restore`] before releasing it. Calling this again after
	/// successful enabling is idempotent while the installed instruction
	/// remains unchanged.
	///
	/// # Safety
	///
	/// No thread may execute or modify the gate during the call.
	pub unsafe fn enable(&mut self) -> Result<(), PatchError> {
		// SAFETY: As the caller promises.
		unsafe { self.patch.enable() }
	}

	/// Whether the gate has an outstanding installed patch.
	pub fn is_active(&self) -> bool {
		self.patch.is_active()
	}

	/// Restores the original instruction, page protection, and instruction
	/// cache.
	///
	/// This is idempotent after successful restoration. If another component
	/// changed the instruction, it is left alone and an error is returned. A
	/// failed Windows call can be retried while the patch remains alive.
	///
	/// # Safety
	///
	/// As for [`Self::enable`].
	pub unsafe fn restore(&mut self) -> Result<(), PatchError> {
		// SAFETY: As the caller promises.
		unsafe { self.patch.restore() }
	}
}

/// The `len` bytes of code at `address`, if they lie wholly in one of
/// `image`'s executable sections.
#[cfg(any(target_os = "windows", test))]
fn code(image: &Image, address: usize, len: usize) -> Option<&[u8]> {
	let (section, offset) = code_section(image, address)?;

	section.bytes.get(offset..offset.checked_add(len)?)
}

/// The executable section of `image` holding `address`, and the address's
/// offset in it.
#[cfg(any(target_os = "windows", test))]
fn code_section(image: &Image, address: usize) -> Option<(&Section, usize)> {
	image
		.sections
		.iter()
		.filter(|section| section.executable)
		.find_map(|section| {
			let offset = address.checked_sub(section.address)?;

			(offset < section.bytes.len()).then_some((section, offset))
		})
}

/// Finds the address of the single gate in the `god` command's callback at
/// `god`, in `image`.
///
/// A gate must start within [`GATE_SEARCH_LEN`] bytes of the callback, in the
/// callback's executable section, load the game rules and the globals from
/// `image`'s readable non-executable sections, and call code that matches
/// [`IS_PVE_MODE_ACTIVE`]. Fails unless exactly one gate passes every check.
#[cfg(any(target_os = "windows", test))]
fn find_gate(image: &Image, god: usize) -> Result<usize, GodError> {
	let (section, start) = code_section(image, god).ok_or(GodError::UnsupportedGame)?;
	let end = start
		.saturating_add(GATE_SEARCH_LEN + GATE_LEN - 1)
		.min(section.bytes.len());
	let callback = &section.bytes[start..end];
	let mut found = None;

	for index in util::find_all(callback, &GATE) {
		let Some(bytes) = index
			.checked_add(GATE_LEN)
			.and_then(|gate_end| callback.get(index..gate_end))
		else {
			continue;
		};
		let Some(gate) = god.checked_add(index) else {
			continue;
		};

		if !util::relative(gate, bytes, 3).is_some_and(|rules| in_data(image, rules))
			|| !util::relative(gate, bytes, 24).is_some_and(|globals| in_data(image, globals))
			|| !util::relative(gate, bytes, 13).is_some_and(|called| {
				code(image, called, IS_PVE_MODE_ACTIVE_LEN)
					.is_some_and(|code| util::pattern(code, &IS_PVE_MODE_ACTIVE))
			}) {
			continue;
		}

		if found.replace(gate).is_some() {
			return Err(GodError::AmbiguousGate);
		}
	}

	found.ok_or(GodError::UnsupportedGame)
}

/// Whether a pointer at `address` lies wholly in one of `image`'s readable
/// non-executable sections.
#[cfg(any(target_os = "windows", test))]
fn in_data(image: &Image, address: usize) -> bool {
	let Some(end) = address.checked_add(size_of::<usize>()) else {
		return false;
	};

	image.sections.iter().any(|section| {
		!section.executable
			&& section.address <= address
			&& section
				.address
				.checked_add(section.bytes.len())
				.is_some_and(|section_end| end <= section_end)
	})
}
