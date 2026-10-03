//! The Windows engine's `host_timescale` cheat gate in TF2, and the code patch
//! that bypasses it.
//!
//! TF2's 64-bit Windows `Host_AccumulateTime` applies a positive
//! `host_timescale` only while `sv_cheats` is set or a demo plays. The gate
//! located here is the `jnz` that skips the demo check when `sv_cheats` is
//! set; the patch turns it into a `jmp`, so neither check refuses a positive
//! time scale. Linux has no corresponding gate, so the `Gate` that patches it
//! exists on Windows only.

#[cfg(test)]
#[path = "../tests/tf2/host_timescale.rs"]
mod tests;

#[cfg(target_os = "windows")]
use crate::util::Module;

#[cfg(target_os = "windows")]
use crate::util::patch::{BytePatch, PatchError};

#[cfg(any(target_os = "windows", test))]
use crate::util::{self, Image, SignaturePattern, sig};

#[cfg(target_os = "windows")]
use std::mem::offset_of;

// The patched opcode is the gate's original `jnz rel8`.
#[cfg(any(target_os = "windows", test))]
const _: () = assert!(matches!(
	PATTERN[PATCH_OFFSET],
	SignaturePattern::Exact(ORIGINAL)
));

// The pattern reads `m_pParent`, then its `m_fValue` and `m_nValue`, at these
// offsets.
#[cfg(target_os = "windows")]
const _: () = {
	assert!(offset_of!(sys::ConVar, m_pParent) == 56);
	assert!(offset_of!(sys::ConVar, m_fValue) == 0x54);
	assert!(offset_of!(sys::ConVar, m_nValue) == 0x58);
};

/// The name of the module holding the gate.
#[cfg(target_os = "windows")]
const ENGINE: &str = "engine.dll";

/// The gate's original `jnz rel8` opcode.
#[cfg(any(target_os = "windows", test))]
const ORIGINAL: u8 = 0x75;

/// Offset of the patched opcode from the start of [`PATTERN`].
#[cfg(any(target_os = "windows", test))]
const PATCH_OFFSET: usize = 28;

// Host_AccumulateTime, TF2 Windows x64. Relocation-dependent displacements
// are masked, then checked semantically below. The patch changes the jnz
// into jmp with its existing +0x1c destination (the final movss).
//
//  +0  mov rcx, [rip+timescale]   ; host_timescale's m_pParent
//  +7  comiss xmm6, [rcx+0x54]    ; m_fValue
// +11  jae reject
// +17  mov rax, [rip+cheats]      ; sv_cheats's m_pParent
// +24  cmp dword [rax+0x58], 0    ; m_nValue
// +28  jnz +58 (rel8 0x1c)        ; PATCH_OFFSET
// +30  mov rcx, [rip+demo]
// +37  mov rax, [rcx]
// +40  call [rax+0x30]
// +43  test al, al
// +45  jz reject
// +51  mov rcx, [rip+timescale]
// +58  movss xmm6, [rcx+0x54]
#[cfg(any(target_os = "windows", test))]
const PATTERN: [SignaturePattern; PATTERN_LEN] = sig![
	0x48 0x8b 0x0d ? ? ? ?
	0x0f 0x2f 0x71 0x54
	0x0f 0x83 ? ? ? ?
	0x48 0x8b 0x05 ? ? ? ?
	0x83 0x78 0x58 0x00
	0x75 0x1c
	0x48 0x8b 0x0d ? ? ? ?
	0x48 0x8b 0x01
	0xff 0x50 0x30
	0x84 0xc0
	0x0f 0x84 ? ? ? ?
	0x48 0x8b 0x0d ? ? ? ?
	0xf3 0x0f 0x10 0x71 0x54
];

/// Length of [`PATTERN`], which is also the block a patch keeps verifying.
#[cfg(any(target_os = "windows", test))]
const PATTERN_LEN: usize = 63;

/// Offset of the continuation both rejection branches must target, from
/// the start of [`PATTERN`].
#[cfg(any(target_os = "windows", test))]
const REJECT_OFFSET: usize = 198;

/// The `jmp rel8` opcode that replaces [`ORIGINAL`].
#[cfg(any(target_os = "windows", test))]
const REPLACEMENT: u8 = 0xeb;

/// The located gate in the loaded `engine.dll`, and the patch that bypasses
/// it.
///
/// The engine module stays loaded while this exists. Dropping it attempts
/// [`Self::restore`], ignoring failures; call that first to observe and
/// handle them.
#[cfg(target_os = "windows")]
#[cfg_attr(docsrs, doc(cfg(target_os = "windows")))]
#[derive(Debug)]
pub struct Gate {
	/// The patch, which is dropped, restoring the gate, before the module is
	/// released.
	patch: BytePatch,

	/// Keeps `engine.dll`, and so the patched code, loaded.
	_module: Module,
}

#[cfg(target_os = "windows")]
impl Gate {
	/// Locates the gate in the loaded `engine.dll`, validating it without
	/// changing any instructions.
	///
	/// The signature was verified in TF2's 64-bit Windows
	/// `Host_AccumulateTime`. Both references to `host_timescale` and the
	/// reference to `sv_cheats` must resolve to the variables' `m_pParent`
	/// fields, at `timescale` and `cheats`, which are not read. The two
	/// rejection branches must lead to the same verified continuation, and the
	/// replacement branch must lead to the float load. Exactly one match is
	/// required across the engine's readable executable sections.
	///
	/// # Safety
	///
	/// The gate changes the engine's code when enabled, restored, or dropped.
	/// Each of those, and this call, must happen while no thread executes or
	/// modifies `Host_AccumulateTime`, such as on the engine's main thread
	/// outside that routine with no other thread patching it.
	pub unsafe fn locate(
		timescale: *const sys::ConVar,
		cheats: *const sys::ConVar,
	) -> Result<Self, GateError> {
		let module = Module::loaded(ENGINE).map_err(|error| match error {
			util::Error::Io(error) => GateError::ModuleNotLoaded(error),
			util::Error::InvalidImage => GateError::InvalidImage,
		})?;

		let image = Image::from_module(&module).map_err(|_| GateError::InvalidImage)?;
		let parent = |variable: *const sys::ConVar| {
			variable
				.addr()
				.checked_add(offset_of!(sys::ConVar, m_pParent))
				.ok_or(GateError::InvalidImage)
		};

		let gate = find_gate(&image, parent(timescale)?, parent(cheats)?)?;
		let start = gate
			.checked_sub(PATCH_OFFSET)
			.ok_or(GateError::InvalidImage)?;

		let verified = image
			.read(start, PATTERN_LEN)
			.ok_or(GateError::InvalidImage)?;

		let block = std::ptr::with_exposed_provenance_mut::<u8>(start);
		let block = std::ptr::NonNull::new(block).ok_or(GateError::InvalidImage)?;

		// SAFETY: The block is the verified code in `engine.dll`, which the
		// gate keeps loaded for as long as the patch, which it drops first,
		// exists. The caller excludes execution and modification of the
		// routine whenever the patch is dropped.
		let patch = unsafe { BytePatch::new(block, verified.into(), PATCH_OFFSET, REPLACEMENT) }
			.ok_or(GateError::InvalidImage)?;

		Ok(Self {
			patch,
			_module: module,
		})
	}

	/// Bypasses the gate, retaining restoration ownership on any error.
	///
	/// If a Windows call fails after writing the byte, the gate still owns
	/// the patch and any pending page-protection or cache cleanup; call
	/// [`Self::restore`] before releasing it. Calling this again after
	/// successful enabling is idempotent while the installed instruction
	/// remains unchanged.
	///
	/// # Safety
	///
	/// No thread may execute or modify `Host_AccumulateTime` during the call.
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
	/// failed Windows call can be retried while the gate remains alive.
	///
	/// # Safety
	///
	/// As for [`Self::enable`].
	pub unsafe fn restore(&mut self) -> Result<(), PatchError> {
		// SAFETY: As the caller promises.
		unsafe { self.patch.restore() }
	}
}

/// Why the gate could not be located.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
	/// More than one candidate passed every check, so none is patched.
	#[error("engine.dll contains more than one verified TF2 time-scale gate")]
	AmbiguousGate,

	/// `engine.dll`'s headers or mapped pages are not the readable 64-bit PE
	/// image the scan expects.
	#[error("engine.dll has an unsupported or inaccessible PE image")]
	InvalidImage,

	/// Windows could not find a loaded `engine.dll`, and reported this error.
	#[error("engine.dll is not loaded: {0}")]
	ModuleNotLoaded(#[source] std::io::Error),

	/// `host_timescale` or `sv_cheats` is not in `engine.dll`'s data sections,
	/// or no candidate passed every signature and reference check, as with
	/// another engine build or an already patched gate.
	#[error("engine.dll does not contain the verified TF2 time-scale gate")]
	UnsupportedEngine,
}

/// Finds the address of the single gate opcode to patch in `image`.
///
/// A signature match must read from `timescale_parent` and `cheats_parent`,
/// the addresses of the variables' `m_pParent` fields, which must lie in
/// `image`'s readable non-executable sections, and load a pointer from within
/// those. Both rejection branches must target [`REJECT_OFFSET`] within the
/// match's section. Fails unless exactly one match passes every check.
#[cfg(any(target_os = "windows", test))]
fn find_gate(
	image: &Image,
	timescale_parent: usize,
	cheats_parent: usize,
) -> Result<usize, GateError> {
	if !in_data(image, timescale_parent) || !in_data(image, cheats_parent) {
		return Err(GateError::UnsupportedEngine);
	}

	let mut found = None;

	for section in image.sections.iter().filter(|section| section.executable) {
		let Some(section_end) = section.address.checked_add(section.bytes.len()) else {
			continue;
		};

		for index in util::find_all(&section.bytes, &PATTERN) {
			let Some(bytes) = index
				.checked_add(PATTERN_LEN)
				.and_then(|end| section.bytes.get(index..end))
			else {
				continue;
			};
			let Some(address) = section.address.checked_add(index) else {
				continue;
			};
			let Some(reject) = address.checked_add(REJECT_OFFSET) else {
				continue;
			};

			if !(section.address..section_end).contains(&reject)
				|| util::relative(address, bytes, 3) != Some(timescale_parent)
				|| util::relative(address, bytes, 20) != Some(cheats_parent)
				|| util::relative(address, bytes, 54) != Some(timescale_parent)
				|| util::relative(address, bytes, 13) != Some(reject)
				|| util::relative(address, bytes, 47) != Some(reject)
				|| !util::relative(address, bytes, 33).is_some_and(|demo| in_data(image, demo))
			{
				continue;
			}

			let Some(gate) = address.checked_add(PATCH_OFFSET) else {
				continue;
			};

			if found.replace(gate).is_some() {
				return Err(GateError::AmbiguousGate);
			}
		}
	}

	found.ok_or(GateError::UnsupportedEngine)
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
