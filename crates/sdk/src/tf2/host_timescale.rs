//! A narrowly checked workaround for TF2's Windows `host_timescale` cheat gate.
//!
//! Linux has no corresponding gate. This does not change `sv_cheats`, console
//! flags, or clients' replicated variables. It only bypasses the Windows
//! engine's check immediately before applying a positive `host_timescale`.

use crate::interfaces::Cvar;
use std::marker::PhantomData;
use std::rc::Rc;

/// Owns the single-byte Windows workaround until it is explicitly restored.
///
/// The Windows engine module remains loaded while this guard exists. Unknown
/// instruction sequences are refused. `Drop` attempts restoration, but callers
/// should call [`Self::restore`] to observe and handle errors before unloading.
/// On Linux this is a no-op guard.
#[doc(alias = "Host_AccumulateTime")]
#[must_use = "keep the patch guard until the plugin restores normal time scaling"]
pub struct HostTimescalePatch {
	#[cfg(target_os = "windows")]
	patch: windows::Patch,
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
		let patch = {
			// SAFETY: The caller upholds the engine-thread and timing contract.
			unsafe { windows::Patch::locate(cvar) }?
		};
		#[cfg(not(target_os = "windows"))]
		let _ = cvar;

		Ok(Self {
			#[cfg(target_os = "windows")]
			patch,
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
			unsafe { self.patch.enable() }
		}
		#[cfg(not(target_os = "windows"))]
		Ok(())
	}

	/// Whether this guard has an outstanding installed Windows patch.
	///
	/// This is always `false` on Linux.
	pub fn is_active(&self) -> bool {
		#[cfg(target_os = "windows")]
		return self.patch.active;
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
			unsafe { self.patch.restore() }
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

#[cfg(any(target_os = "windows", test))]
mod scan {
	use super::PatchError;
	use sdk_raw::util::{self, SignaturePattern, sig};
	use std::ops::Range;

	// The patched opcode is the gate's original `jnz rel8`.
	const _: () = assert!(matches!(
		PATTERN[PATCH_OFFSET],
		SignaturePattern::Exact(ORIGINAL)
	));

	/// The gate's original `jnz rel8` opcode.
	pub(super) const ORIGINAL: u8 = 0x75;

	/// Offset of the patched opcode from the start of [`PATTERN`].
	pub(super) const PATCH_OFFSET: usize = 28;

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
	pub(super) const PATTERN_LEN: usize = 63;

	/// Offset of the continuation both rejection branches must target, from
	/// the start of [`PATTERN`].
	const REJECT_OFFSET: usize = 198;

	/// The `jmp rel8` opcode that replaces [`ORIGINAL`].
	pub(super) const REPLACEMENT: u8 = 0xeb;

	/// An owned snapshot, so scanning never borrows mutable engine memory.
	pub(super) struct CodeSection {
		/// The section's offset from the image base.
		pub offset: usize,
		/// A copy of the section's bytes.
		pub bytes: Vec<u8>,
	}

	/// Finds the image offset of the single gate opcode to patch.
	///
	/// `base` is the image's address and `data` the absolute address ranges of
	/// its readable non-executable sections. A signature match must read from
	/// `timescale_parent` and `cheats_parent`, the addresses of the variables'
	/// `m_pParent` fields, and load a pointer from within `data`. Both rejection
	/// branches must target [`REJECT_OFFSET`] within the match's section. Fails
	/// unless exactly one match passes every check.
	pub(super) fn find_gate(
		base: usize,
		code: &[CodeSection],
		data: &[Range<usize>],
		timescale_parent: usize,
		cheats_parent: usize,
	) -> Result<usize, PatchError> {
		let mut found = None;
		for section in code {
			for index in util::find_all(&section.bytes, &PATTERN) {
				let bytes = &section.bytes[index..index + PATTERN.len()];
				let Some(offset) = section.offset.checked_add(index) else {
					continue;
				};
				let Some(address) = base.checked_add(offset) else {
					continue;
				};
				let Some(reject) = address.checked_add(REJECT_OFFSET) else {
					continue;
				};
				let Some(section_start) = base.checked_add(section.offset) else {
					continue;
				};
				let Some(section_end) = section_start.checked_add(section.bytes.len()) else {
					continue;
				};
				if !(section_start..section_end).contains(&reject)
					|| relative_target(address, 7, bytes, 3) != Some(timescale_parent)
					|| relative_target(address, 24, bytes, 20) != Some(cheats_parent)
					|| relative_target(address, 58, bytes, 54) != Some(timescale_parent)
					|| relative_target(address, 17, bytes, 13) != Some(reject)
					|| relative_target(address, 51, bytes, 47) != Some(reject)
				{
					continue;
				}
				let Some(demo) = relative_target(address, 37, bytes, 33) else {
					continue;
				};
				let Some(demo_end) = demo.checked_add(size_of::<usize>()) else {
					continue;
				};
				if !data
					.iter()
					.any(|range| range.start <= demo && demo_end <= range.end)
				{
					continue;
				}
				if found.replace(offset + PATCH_OFFSET).is_some() {
					return Err(PatchError::AmbiguousGate);
				}
			}
		}
		found.ok_or(PatchError::UnsupportedEngine)
	}

	/// Resolves the RIP-relative target of the instruction ending at
	/// `instruction_end`, whose `i32` displacement starts at `displacement`.
	///
	/// Both offsets are relative to `bytes`, which starts at `address`. Returns
	/// `None` if the displacement is out of bounds or the target overflows.
	fn relative_target(
		address: usize,
		instruction_end: usize,
		bytes: &[u8],
		displacement: usize,
	) -> Option<usize> {
		let value = i32::from_le_bytes(bytes.get(displacement..displacement + 4)?.try_into().ok()?);
		address
			.checked_add(instruction_end)?
			.checked_add_signed(value as isize)
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		const BASE: usize = 0x180000000;
		const CHEATS: usize = BASE + 0x2100;

		/// The only data section, holding `CHEATS`, `DEMO`, and `TIMESCALE`.
		const DATA: Range<usize> = BASE + 0x2000..BASE + 0x3000;

		const DEMO: usize = BASE + 0x2200;
		const TIMESCALE: usize = BASE + 0x2000;

		#[test]
		fn candidates_at_nonzero_section_offsets_are_found() {
			let mut code = CodeSection {
				offset: 0x100,
				bytes: vec![0x90; 0x80],
			};
			code.bytes.extend(section(0x180).bytes);
			assert_eq!(find(&[code]).unwrap(), 0x180 + PATCH_OFFSET);
		}

		#[test]
		fn every_candidate_in_a_section_is_validated() {
			let mut code = section(0x100);
			code.bytes.extend(section(0x200).bytes);
			assert!(matches!(
				find(std::slice::from_ref(&code)),
				Err(PatchError::AmbiguousGate)
			));

			// A signature match with the wrong variable must not hide the valid
			// candidate later in the same section or make it ambiguous.
			put_relative(&mut code.bytes, 3, 7, CHEATS, BASE + code.offset);
			assert_eq!(find(&[code]).unwrap(), 0x200 + PATCH_OFFSET);
		}

		fn find(code: &[CodeSection]) -> Result<usize, PatchError> {
			find_gate(BASE, code, &[DATA], TIMESCALE, CHEATS)
		}

		fn put_relative(bytes: &mut [u8], at: usize, end: usize, target: usize, address: usize) {
			let delta = i32::try_from(target as i128 - (address + end) as i128).unwrap();
			bytes[at..at + 4].copy_from_slice(&delta.to_le_bytes());
		}

		#[test]
		fn rejection_branches_and_replacement_destination_are_checked() {
			for (at, end) in [(13, 17), (47, 51)] {
				let mut code = section(0x100);
				put_relative(
					&mut code.bytes,
					at,
					end,
					BASE + code.offset + 199,
					BASE + code.offset,
				);
				assert!(matches!(find(&[code]), Err(PatchError::UnsupportedEngine)));
			}
			let mut code = section(0x100);
			code.bytes[PATCH_OFFSET + 1] = 0x1d;
			assert!(matches!(find(&[code]), Err(PatchError::UnsupportedEngine)));
		}

		#[test]
		fn relocation_targets_must_be_the_registered_variables() {
			for (at, end, wrong) in [
				(3, 7, CHEATS),
				(20, 24, TIMESCALE),
				(54, 58, CHEATS),
				(33, 37, BASE + 0x100),
			] {
				let mut code = section(0x100);
				put_relative(&mut code.bytes, at, end, wrong, BASE + code.offset);
				assert!(matches!(find(&[code]), Err(PatchError::UnsupportedEngine)));
			}
		}

		fn section(offset: usize) -> CodeSection {
			let address = BASE + offset;
			let mut bytes = vec![0x90; 256];
			for (out, input) in bytes.iter_mut().zip(PATTERN) {
				*out = match input {
					SignaturePattern::Exact(byte) => byte,
					SignaturePattern::Any => 0,
				};
			}
			put_relative(&mut bytes, 3, 7, TIMESCALE, address);
			put_relative(&mut bytes, 20, 24, CHEATS, address);
			put_relative(&mut bytes, 54, 58, TIMESCALE, address);
			put_relative(&mut bytes, 33, 37, DEMO, address);
			// Literal, so the fixture pins REJECT_OFFSET independently.
			put_relative(&mut bytes, 13, 17, address + 198, address);
			put_relative(&mut bytes, 47, 51, address + 198, address);
			CodeSection { offset, bytes }
		}

		#[test]
		fn selects_only_the_verified_gate() {
			assert_eq!(find(&[section(0x100)]).unwrap(), 0x100 + PATCH_OFFSET);
			assert!(matches!(
				find(&[section(0x100), section(0x500)]),
				Err(PatchError::AmbiguousGate)
			));
		}

		#[test]
		fn truncated_or_already_patched_code_is_refused() {
			let mut code = section(0x100);
			code.bytes.truncate(PATTERN_LEN);
			assert!(matches!(find(&[code]), Err(PatchError::UnsupportedEngine)));
			let mut code = section(0x100);
			code.bytes[PATCH_OFFSET] = REPLACEMENT;
			assert!(matches!(find(&[code]), Err(PatchError::UnsupportedEngine)));
		}
	}
}

#[cfg(target_os = "windows")]
mod windows {
	use super::{PatchError, scan};
	use crate::interfaces::Cvar;
	use std::ffi::c_void;
	use std::mem::{MaybeUninit, offset_of};
	use std::ops::Range;
	use std::ptr::{self, NonNull};

	const _: () = assert!(offset_of!(MemoryInformation, region_size) == 24);
	const _: () = assert!(offset_of!(sys::ConVar, m_fValue) == 0x54);
	const _: () = assert!(offset_of!(sys::ConVar, m_nValue) == 0x58);
	const _: () = assert!(offset_of!(sys::ConVar, m_pParent) == 56);
	const _: () = assert!(size_of::<MemoryInformation>() == 48);
	const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
	const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x20b;
	const IMAGE_SCN_MEM_EXECUTE: u32 = 0x20000000;
	const IMAGE_SCN_MEM_READ: u32 = 0x40000000;
	const IMAGE_SIZEOF_SECTION_HEADER: usize = 40;
	const MEM_COMMIT: u32 = 0x1000;
	const MEM_IMAGE: u32 = 0x1000000;
	const PAGE_EXECUTE_READWRITE: u32 = 0x40;
	const PAGE_GUARD: u32 = 0x100;
	const PAGE_NOACCESS: u32 = 0x01;

	/// The protections the scan accepts as readable: `PAGE_READONLY`,
	/// `PAGE_READWRITE`, `PAGE_WRITECOPY`, and their `PAGE_EXECUTE_*`
	/// counterparts (plain `PAGE_EXECUTE` is excluded).
	const READABLE_PROTECTIONS: u32 = 0xee;

	/// Windows x64 `MEMORY_BASIC_INFORMATION`, as filled by `VirtualQuery`.
	///
	/// `protection` is its `Protect` member and `kind` its `Type` member.
	#[repr(C)]
	struct MemoryInformation {
		base: *mut c_void,
		allocation_base: *mut c_void,
		allocation_protection: u32,
		partition_id: u16,
		region_size: usize,
		state: u32,
		protection: u32,
		kind: u32,
	}

	/// A loader reference to `engine.dll`, which keeps its image mapped until
	/// this is dropped.
	struct Module(NonNull<c_void>);

	impl Module {
		/// Acquires a reference to the already loaded `engine.dll`, without
		/// loading it.
		fn engine() -> Result<Self, PatchError> {
			// `engine.dll`, as NUL-terminated UTF-16.
			const NAME: [u16; 11] = [101, 110, 103, 105, 110, 101, 46, 100, 108, 108, 0];
			let mut module = ptr::null_mut();
			// SAFETY: Both pointers are valid; flags zero acquires a loader
			// reference to an already loaded named module without loading a DLL.
			if unsafe { GetModuleHandleExW(0, NAME.as_ptr(), &mut module) } == 0 {
				return Err(os_error("GetModuleHandleExW(engine.dll)"));
			}
			NonNull::new(module)
				.map(Self)
				.ok_or(PatchError::InvalidImage)
		}

		fn base(&self) -> usize {
			self.0.as_ptr() as usize
		}

		/// Copies `len` bytes at `offset` from the image base, once [`Self::readable`]
		/// accepts them.
		fn copy(&self, offset: usize, len: usize) -> Result<Vec<u8>, PatchError> {
			self.readable(offset, len)?;
			let mut bytes = vec![0; len];
			// SAFETY: readable verified all source pages belong to the held
			// image and are readable; the owned destination does not overlap.
			// Installation's contract excludes concurrent code modification.
			unsafe {
				ptr::copy_nonoverlapping(
					(self.base() + offset) as *const u8,
					bytes.as_mut_ptr(),
					len,
				)
			};
			Ok(bytes)
		}

		/// Checks every page before copying memory out of the mapped image.
		///
		/// Each page in `len` bytes at `offset` from the image base must be
		/// committed memory of this image whose protection permits reads
		/// without faulting.
		fn readable(&self, offset: usize, len: usize) -> Result<(), PatchError> {
			let mut address = self
				.base()
				.checked_add(offset)
				.ok_or(PatchError::InvalidImage)?;
			let end = address.checked_add(len).ok_or(PatchError::InvalidImage)?;
			while address < end {
				let mut information = MaybeUninit::uninit();
				// SAFETY: VirtualQuery validates the address; the output buffer
				// has the Windows x64 MEMORY_BASIC_INFORMATION layout asserted above.
				if unsafe {
					VirtualQuery(
						address as *const c_void,
						information.as_mut_ptr(),
						size_of::<MemoryInformation>(),
					)
				} != size_of::<MemoryInformation>()
				{
					return Err(PatchError::InvalidImage);
				}
				// SAFETY: VirtualQuery initialized the complete structure.
				let information = unsafe { information.assume_init() };
				if information.allocation_base != self.0.as_ptr()
					|| information.state != MEM_COMMIT
					|| information.kind != MEM_IMAGE
					|| information.protection & (PAGE_GUARD | PAGE_NOACCESS) != 0
					|| information.protection & READABLE_PROTECTIONS == 0
				{
					return Err(PatchError::InvalidImage);
				}
				let next = (information.base as usize)
					.checked_add(information.region_size)
					.ok_or(PatchError::InvalidImage)?;
				if next <= address {
					return Err(PatchError::InvalidImage);
				}
				address = next.min(end);
			}
			Ok(())
		}

		/// Parses the PE headers into copies of the readable executable sections
		/// and the absolute address ranges of the readable non-executable ones.
		fn sections(&self) -> Result<(Vec<scan::CodeSection>, Vec<Range<usize>>), PatchError> {
			let dos = self.copy(0, 64)?;
			if dos[..2] != *b"MZ" {
				return Err(PatchError::InvalidImage);
			}
			let pe = usize::try_from(i32::from_le_bytes(dos[60..64].try_into().unwrap()))
				.map_err(|_| PatchError::InvalidImage)?;
			if !(64..=0x100000).contains(&pe) {
				return Err(PatchError::InvalidImage);
			}
			let coff = self.copy(pe, 24)?;
			if coff[..4] != *b"PE\0\0"
				|| u16::from_le_bytes(coff[4..6].try_into().unwrap()) != IMAGE_FILE_MACHINE_AMD64
			{
				return Err(PatchError::InvalidImage);
			}
			let count = u16::from_le_bytes(coff[6..8].try_into().unwrap()) as usize;
			let optional_size = u16::from_le_bytes(coff[20..22].try_into().unwrap()) as usize;
			if !(1..=96).contains(&count) || !(112..=4096).contains(&optional_size) {
				return Err(PatchError::InvalidImage);
			}
			let optional = self.copy(pe + 24, optional_size)?;
			if u16::from_le_bytes(optional[..2].try_into().unwrap())
				!= IMAGE_NT_OPTIONAL_HDR64_MAGIC
			{
				return Err(PatchError::InvalidImage);
			}
			let image_size = u32::from_le_bytes(optional[56..60].try_into().unwrap()) as usize;
			let table = pe + 24 + optional_size;
			if !(table + count * IMAGE_SIZEOF_SECTION_HEADER..=0x40000000).contains(&image_size) {
				return Err(PatchError::InvalidImage);
			}
			let sections = self.copy(table, count * IMAGE_SIZEOF_SECTION_HEADER)?;
			let mut code = Vec::new();
			let mut data = Vec::new();
			for section in sections.as_chunks::<IMAGE_SIZEOF_SECTION_HEADER>().0 {
				let size = u32::from_le_bytes(section[8..12].try_into().unwrap()) as usize;
				let offset = u32::from_le_bytes(section[12..16].try_into().unwrap()) as usize;
				let flags = u32::from_le_bytes(section[36..40].try_into().unwrap());
				let end = offset.checked_add(size).ok_or(PatchError::InvalidImage)?;
				if offset > image_size || end > image_size {
					return Err(PatchError::InvalidImage);
				}
				if size == 0 || flags & IMAGE_SCN_MEM_READ == 0 {
					continue;
				}
				if flags & IMAGE_SCN_MEM_EXECUTE != 0 {
					code.push(scan::CodeSection {
						offset,
						bytes: self.copy(offset, size)?,
					});
				} else {
					self.readable(offset, size)?;
					data.push(self.base() + offset..self.base() + end);
				}
			}
			Ok((code, data))
		}
	}

	impl Drop for Module {
		fn drop(&mut self) {
			// SAFETY: This releases only the reference acquired by engine().
			unsafe { FreeLibrary(self.0.as_ptr()) };
		}
	}

	/// The located gate and any cleanup its last write left pending.
	pub(super) struct Patch {
		/// Keeps `engine.dll`, and so `location`, mapped.
		_module: Module,
		/// The gate opcode, at [`scan::PATCH_OFFSET`] within the verified block.
		location: NonNull<u8>,
		/// The verified block as located, holding [`scan::ORIGINAL`] at the gate.
		original_block: [u8; scan::PATTERN_LEN],
		/// Whether [`scan::REPLACEMENT`] was written and not yet restored.
		pub active: bool,
		/// Whether the last write still needs an instruction cache flush.
		cache_dirty: bool,
		/// The page protection to put back after a write, while that is pending.
		protection: Option<u32>,
	}

	impl Patch {
		/// Implements [`HostTimescalePatch::locate`](super::HostTimescalePatch::locate)
		/// under the same contract.
		pub(super) unsafe fn locate(cvar: Cvar<'_>) -> Result<Self, PatchError> {
			let timescale = cvar
				.find_var(c"host_timescale")
				.ok_or(PatchError::MissingVariable("host_timescale"))?;
			let cheats = cvar
				.find_var(c"sv_cheats")
				.ok_or(PatchError::MissingVariable("sv_cheats"))?;
			let module = Module::engine()?;
			let (code, data) = module.sections()?;
			let timescale_parent = (timescale.as_ptr() as usize)
				.checked_add(offset_of!(sys::ConVar, m_pParent))
				.ok_or(PatchError::InvalidImage)?;
			let cheats_parent = (cheats.as_ptr() as usize)
				.checked_add(offset_of!(sys::ConVar, m_pParent))
				.ok_or(PatchError::InvalidImage)?;
			if ![timescale_parent, cheats_parent].iter().all(|address| {
				data.iter().any(|range| {
					range.start <= *address
						&& address.checked_add(8).is_some_and(|end| end <= range.end)
				})
			}) {
				return Err(PatchError::UnsupportedEngine);
			}
			let offset =
				scan::find_gate(module.base(), &code, &data, timescale_parent, cheats_parent)?;
			let location = NonNull::new((module.base() + offset) as *mut u8)
				.ok_or(PatchError::InvalidImage)?;
			let original_block = module
				.copy(offset - scan::PATCH_OFFSET, scan::PATTERN_LEN)?
				.try_into()
				.map_err(|_| PatchError::InvalidImage)?;
			Ok(Self {
				_module: module,
				location,
				original_block,
				active: false,
				cache_dirty: false,
				protection: None,
			})
		}

		/// Implements [`HostTimescalePatch::enable`](super::HostTimescalePatch::enable)
		/// under the same contract.
		pub(super) unsafe fn enable(&mut self) -> Result<(), PatchError> {
			// SAFETY: locate validated all 63 bytes and keeps the module loaded;
			// the caller excludes concurrent execution and modification.
			if !unsafe {
				self.matches_block(if self.active {
					scan::REPLACEMENT
				} else {
					scan::ORIGINAL
				})
			} {
				return Err(PatchError::InstructionChanged);
			}
			if self.active {
				self.finish_write()
			} else {
				// SAFETY: The caller excludes execution and concurrent patching.
				unsafe { self.replace(scan::ORIGINAL, scan::REPLACEMENT, true) }
			}
		}

		/// Attempts the pending instruction cache flush and protection restore.
		///
		/// A step that fails stays pending for the next call. The flush's error
		/// takes precedence when both fail.
		fn finish_write(&mut self) -> Result<(), PatchError> {
			let cache_error = if self.cache_dirty {
				// SAFETY: The process pseudo-handle and the held image byte are
				// valid; this publishes the instruction change to execution.
				if unsafe {
					FlushInstructionCache(GetCurrentProcess(), self.location.as_ptr().cast(), 1)
				} == 0
				{
					Some(os_error("FlushInstructionCache(time-scale gate)"))
				} else {
					self.cache_dirty = false;
					None
				}
			} else {
				None
			};
			let protection_error = if let Some(protection) = self.protection {
				let mut old = 0;
				// SAFETY: This restores the protection Windows reported for the
				// live byte's page. Save the pending restoration on failure.
				if unsafe { VirtualProtect(self.location.as_ptr().cast(), 1, protection, &mut old) }
					== 0
				{
					Some(os_error(
						"VirtualProtect(time-scale gate, original protection)",
					))
				} else {
					self.protection = None;
					None
				}
			} else {
				None
			};
			match cache_error.or(protection_error) {
				Some(error) => Err(error),
				None => Ok(()),
			}
		}

		/// Whether the live block equals the verified one with `opcode` at the
		/// gate. The caller must exclude concurrent modification of the block.
		unsafe fn matches_block(&self, opcode: u8) -> bool {
			self.original_block
				.iter()
				.enumerate()
				.all(|(index, expected)| {
					let expected = if index == scan::PATCH_OFFSET {
						opcode
					} else {
						*expected
					};
					// SAFETY: The complete validated block is live in the held image,
					// and the caller excludes concurrent instruction modification.
					(unsafe {
						self.location
							.as_ptr()
							.sub(scan::PATCH_OFFSET)
							.add(index)
							.read_volatile()
					}) == expected
				})
		}

		/// Writes `value` over the gate opcode, records `active`, and finishes
		/// the write.
		///
		/// Writes nothing and returns [`PatchError::InstructionChanged`] unless
		/// the opcode is `expected`. The caller must exclude execution and
		/// concurrent modification of the gate.
		unsafe fn replace(
			&mut self,
			expected: u8,
			value: u8,
			active: bool,
		) -> Result<(), PatchError> {
			// SAFETY: The held module keeps this validated executable byte live;
			// the caller excludes simultaneous execution and modification.
			if unsafe { self.location.as_ptr().read_volatile() } != expected {
				return Err(PatchError::InstructionChanged);
			}
			let mut old = 0;
			// SAFETY: This is one live image byte; Windows changes its page's
			// protection and initializes old before the byte is written.
			if unsafe {
				VirtualProtect(
					self.location.as_ptr().cast(),
					1,
					PAGE_EXECUTE_READWRITE,
					&mut old,
				)
			} == 0
			{
				return Err(os_error("VirtualProtect(time-scale gate, writable)"));
			}
			self.protection.get_or_insert(old);
			// SAFETY: The byte is writable and nobody executes or modifies it.
			unsafe { self.location.as_ptr().write_volatile(value) };
			self.active = active;
			self.cache_dirty = true;
			self.finish_write()
		}

		/// Implements [`HostTimescalePatch::restore`](super::HostTimescalePatch::restore)
		/// under the same contract.
		pub(super) unsafe fn restore(&mut self) -> Result<(), PatchError> {
			if self.active {
				// SAFETY: The held module keeps the checked instruction live.
				// If another component already restored the original byte, there
				// is nothing left to overwrite; pending OS cleanup still runs.
				if unsafe { self.location.as_ptr().read_volatile() } == scan::ORIGINAL {
					self.active = false;
					return self.finish_write();
				}
				// SAFETY: As for the checked instruction; refuse to change its
				// branch if another component modified the surrounding block.
				if !unsafe { self.matches_block(scan::REPLACEMENT) } {
					return Err(PatchError::InstructionChanged);
				}
				// SAFETY: The caller upholds the same contract as installation.
				unsafe { self.replace(scan::REPLACEMENT, scan::ORIGINAL, false) }
			} else {
				self.finish_write()
			}
		}
	}

	impl Drop for Patch {
		fn drop(&mut self) {
			// SAFETY: locate's caller promises the timing/main-thread contract
			// also holds for destruction. Module is released only after this.
			let _ = unsafe { self.restore() };
		}
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
		fn FreeLibrary(module: *mut c_void) -> i32;
		fn GetCurrentProcess() -> *mut c_void;
		fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;

		fn VirtualProtect(
			address: *const c_void,
			size: usize,
			protection: u32,
			old: *mut u32,
		) -> i32;

		fn VirtualQuery(
			address: *const c_void,
			information: *mut MemoryInformation,
			length: usize,
		) -> usize;
	}

	/// Wraps the calling thread's last Windows error for `operation`.
	fn os_error(operation: &'static str) -> PatchError {
		PatchError::Windows {
			operation,
			source: std::io::Error::last_os_error(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::PatchError;

	#[test]
	fn windows_errors_display_their_os_reason() {
		let error = PatchError::Windows {
			operation: "VirtualProtect",
			source: std::io::Error::other("reason"),
		};
		assert_eq!(error.to_string(), "VirtualProtect failed: reason");
	}
}
