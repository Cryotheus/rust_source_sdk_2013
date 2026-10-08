//! TF2's answer to whether a player-versus-environment mode is active, and the
//! code patch that has it always answer yes.
//!
//! `CTFGameRules::IsPVEModeActive` decides, among other things, whether the
//! `god` command may run while `gpGlobals->deathmatch` is set, as it always is
//! in TF2. Shipping builds compile out the Raid and Boss Battle modes it also
//! checks in the SDK's sources, so it answers whether the level plays Mann vs.
//! Machine (`m_bPlayingMannVsMachine`), and nothing has it answer yes without
//! turning that mode on.
//!
//! The function is located through the `god` command's callback, which calls
//! it, and patched in place: its `setnz al`, after `cmp byte [rcx+flag], 0`,
//! becomes `setae al`, which that comparison always satisfies. Only the
//! function's callers see the patch, not the game rules' own checks, which
//! read the flag inline, and neither do clients. The linker may have folded
//! other functions of identical code into it, which then answer yes too: in
//! TF2's 64-bit Windows build, `CTFGameRules::ShouldSkipAutoScramble` is the
//! same function. Linux has no patch, so the `Patch` exists on Windows only.

#[cfg(test)]
#[path = "../tests/tf2/pve_mode.rs"]
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
	assert!(matches!(exact_u32(&GATE, 35), Some(0x8000)));
	assert!(FL_GODMODE == 0x8000);
};

// The patched byte is the answer's original `setnz` condition.
#[cfg(any(target_os = "windows", test))]
const _: () = assert!(matches!(
	ANSWER[PATCH_OFFSET],
	SignaturePattern::Exact(ORIGINAL)
));

/// `CTFGameRules::IsPVEModeActive`, TF2 Windows x64, once its flag's offset
/// is masked. The patch changes `setnz` into `setae`.
///
/// ```text
///  +0  cmp byte [rcx+flag], 0    ; m_bPlayingMannVsMachine
///  +7  setnz al                  ; PATCH_OFFSET is its condition
/// +10  ret
/// ```
#[cfg(any(target_os = "windows", test))]
const ANSWER: [SignaturePattern; ANSWER_LEN] = sig![
	0x80 0xb9 ? ? ? ? 0x00
	0x0f 0x95 0xc0
	0xc3
];

/// Length of [`ANSWER`], which is also the block a patch keeps verifying.
#[cfg(any(target_os = "windows", test))]
const ANSWER_LEN: usize = 11;

/// Offset of the `deathmatch` field's displacement in [`GATE`].
#[cfg(any(target_os = "windows", test))]
const DEATHMATCH_OFFSET: usize = 30;

/// The `god` command's gate, in its callback, TF2 Windows x64. It skips to
/// the toggle while there are no game rules or the answer is yes, and returns
/// while `gpGlobals->deathmatch` is set otherwise. Relocation-dependent
/// displacements are masked, then checked semantically.
///
/// ```text
///  +0  mov rcx, [rip+rules]      ; g_pGameRules
///  +7  test rcx, rcx
/// +10  jz +34 (rel8 0x16)
/// +12  call answer               ; IsPVEModeActive
/// +17  test al, al
/// +19  jnz +34 (rel8 0x0d)
/// +21  mov rax, [rip+globals]    ; gpGlobals
/// +28  cmp byte [rax+0x65], 0    ; deathmatch
/// +32  jnz reject
/// +34  mov edx, 0x8000           ; FL_GODMODE, to toggle
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

/// The answer's original `setnz` condition.
#[cfg(any(target_os = "windows", test))]
const ORIGINAL: u8 = 0x95;

/// Offset of the patched condition from the start of [`ANSWER`].
#[cfg(any(target_os = "windows", test))]
const PATCH_OFFSET: usize = 8;

/// The `setae` condition that replaces [`ORIGINAL`]. The unsigned comparison
/// with zero before it never sets the carry flag, so it always answers yes.
#[cfg(any(target_os = "windows", test))]
const REPLACEMENT: u8 = 0x93;

/// The name of the module holding the answer.
#[cfg(target_os = "windows")]
const SERVER: &str = "server.dll";

/// The located answer in the loaded `server.dll`, and the patch that has it
/// always answer yes.
///
/// The server module stays loaded while this exists. Dropping it attempts
/// [`Self::restore`], ignoring failures; call that first to observe and
/// handle them.
#[cfg(target_os = "windows")]
#[cfg_attr(docsrs, doc(cfg(target_os = "windows")))]
#[derive(Debug)]
pub struct Patch {
	/// The patch, which is dropped, restoring the answer, before the module
	/// is released.
	patch: BytePatch,

	/// Keeps `server.dll`, and so the patched code, loaded.
	_module: Module,
}

#[cfg(target_os = "windows")]
impl Patch {
	/// Locates the answer in the loaded `server.dll`, through the gate of the
	/// `god` command's callback at `god`, validating it without changing any
	/// instructions.
	///
	/// The signatures were verified in TF2's 64-bit Windows `CC_God_f` and
	/// `CTFGameRules::IsPVEModeActive`. The callback must lie in the module's
	/// executable sections, and hold exactly one gate within its first bytes,
	/// whose references to the game rules and the globals lie in its data
	/// sections, and whose call leads to the answer's code.
	///
	/// # Safety
	///
	/// The patch changes the game's code when enabled, restored, or dropped.
	/// Each of those, and this call, must happen while no thread executes or
	/// modifies the answer, such as on the server's main thread outside the
	/// game's calls, with no other thread patching it.
	pub unsafe fn locate(god: usize) -> Result<Self, PveModeError> {
		let module = Module::loaded(SERVER).map_err(|error| match error {
			util::Error::Io(error) => PveModeError::ModuleNotLoaded(error),
			util::Error::InvalidImage => PveModeError::InvalidImage,
		})?;

		let image = Image::from_module(&module).map_err(|_| PveModeError::InvalidImage)?;
		let answer = find_answer(&image, god)?;

		let verified = code(&image, answer, ANSWER_LEN).ok_or(PveModeError::InvalidImage)?;
		let block = std::ptr::with_exposed_provenance_mut::<u8>(answer);
		let block = std::ptr::NonNull::new(block).ok_or(PveModeError::InvalidImage)?;

		// SAFETY: The block is the verified code in `server.dll`, which the
		// patch keeps loaded for as long as the byte patch, which it drops
		// first, exists. The caller excludes execution and modification of
		// the answer whenever the patch is dropped.
		let patch = unsafe { BytePatch::new(block, verified.into(), PATCH_OFFSET, REPLACEMENT) }
			.ok_or(PveModeError::InvalidImage)?;

		Ok(Self {
			patch,
			_module: module,
		})
	}

	/// Has the answer always be yes, retaining restoration ownership on any
	/// error.
	///
	/// If a Windows call fails after writing the byte, the patch still owns
	/// it and any pending page-protection or cache cleanup; call
	/// [`Self::restore`] before releasing it. Calling this again after
	/// successful enabling is idempotent while the installed instruction
	/// remains unchanged.
	///
	/// # Safety
	///
	/// No thread may execute or modify the answer during the call.
	pub unsafe fn enable(&mut self) -> Result<(), PatchError> {
		// SAFETY: As the caller promises.
		unsafe { self.patch.enable() }
	}

	/// Whether the answer has an outstanding installed patch.
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

/// Why the answer could not be located.
#[derive(Debug, thiserror::Error)]
pub enum PveModeError {
	/// More than one gate passed every check, so none is patched.
	#[error("the god command's callback holds more than one verified PvE gate")]
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
	/// another game build or an already patched answer.
	#[error("server.dll does not contain the verified TF2 PvE gate")]
	UnsupportedGame,
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

/// Finds the address of the answer the single gate in the `god` command's
/// callback at `god` calls, in `image`.
///
/// A gate must start within [`GATE_SEARCH_LEN`] bytes of the callback, in the
/// callback's executable section, load the game rules and the globals from
/// `image`'s readable non-executable sections, and call code that matches
/// [`ANSWER`]. Fails unless exactly one gate passes every check.
#[cfg(any(target_os = "windows", test))]
fn find_answer(image: &Image, god: usize) -> Result<usize, PveModeError> {
	let (section, start) = code_section(image, god).ok_or(PveModeError::UnsupportedGame)?;
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
		let Some(address) = god.checked_add(index) else {
			continue;
		};
		let Some(answer) = util::relative(address, bytes, 13) else {
			continue;
		};

		if !util::relative(address, bytes, 3).is_some_and(|rules| in_data(image, rules))
			|| !util::relative(address, bytes, 24).is_some_and(|globals| in_data(image, globals))
			|| !code(image, answer, ANSWER_LEN).is_some_and(|code| util::pattern(code, &ANSWER))
		{
			continue;
		}

		if found.replace(answer).is_some() {
			return Err(PveModeError::AmbiguousGate);
		}
	}

	found.ok_or(PveModeError::UnsupportedGame)
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
