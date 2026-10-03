//! Primary C++ vtable discovery from compiler RTTI in owned image snapshots.

#[cfg(any(target_os = "windows", test))]
use super::u32_at;

use super::{Image, is_executable, word_at};

impl Image {
	#[cfg(any(target_os = "linux", test))]
	fn itanium(&self, class: &str, slot: usize) -> Vec<usize> {
		let mut tables = Vec::new();

		for name in self.matches(format!("{}{class}\0", class.len()).as_bytes(), 1) {
			for name_pointer in self.matches(&name.to_le_bytes(), 8) {
				let Some(type_info) = name_pointer.checked_sub(8) else {
					continue;
				};

				for reference in self.matches(&type_info.to_le_bytes(), 8) {
					// The primary address point follows offset-to-top=0 and the
					// pointer to the class's type_info object.
					let Some(prefix) = reference.checked_sub(8).and_then(|p| self.rtti_read(p, 8))
					else {
						continue;
					};
					let Some(table) = reference.checked_add(8) else {
						continue;
					};

					if word_at(prefix, 0) == Some(0) && self.valid_table(table, slot) {
						tables.push(table);
					}
				}
			}
		}

		tables.sort_unstable();
		tables.dedup();
		tables
	}

	#[cfg(any(target_os = "windows", test))]
	fn msvc(&self, class: &str, slot: usize) -> Vec<usize> {
		let mut tables = Vec::new();

		for name in self.matches(format!(".?AV{class}@@\0").as_bytes(), 1) {
			let Some(descriptor) = name.checked_sub(16) else {
				continue;
			};
			let Some(relative) = descriptor
				.checked_sub(self.base)
				.and_then(|n| u32::try_from(n).ok())
			else {
				continue;
			};

			for reference in self.matches(&relative.to_le_bytes(), 4) {
				let Some(locator) = reference.checked_sub(12) else {
					continue;
				};
				let Some(bytes) = self.rtti_read(locator, 24) else {
					continue;
				};

				// MSVC x64 complete-object locator: signature=1, primary
				// subobject offset=0, no construction displacement, self RVA.
				if u32_at(bytes, 0) != Some(1)
					|| u32_at(bytes, 4) != Some(0)
					|| u32_at(bytes, 8) != Some(0)
					|| u32_at(bytes, 20).and_then(|n| self.base.checked_add(n as usize))
						!= Some(locator)
				{
					continue;
				}

				for pointer in self.matches(&locator.to_le_bytes(), 8) {
					let Some(table) = pointer.checked_add(8) else {
						continue;
					};
					if self.valid_table(table, slot) {
						tables.push(table);
					}
				}
			}
		}

		tables.sort_unstable();
		tables.dedup();
		tables
	}

	/// Finds one primary vtable for a global, unqualified C++ class name.
	/// Uses MSVC x64 RTTI on Windows and Itanium RTTI on Linux. Secondary and
	/// ambiguous tables are rejected, as are tables lacking an executable entry
	/// at `slot`. An entry may point to a hook trampoline outside this image.
	///
	/// The returned address is metadata from the snapshot. It does not keep the
	/// module loaded, establish a function signature, or authorize dereferencing
	/// the address. Callers must establish those guarantees before using it.
	pub fn primary_vtable(&self, class: &str, slot: usize) -> Option<usize> {
		if self.base == 0 || class.is_empty() || class.as_bytes().contains(&0) {
			return None;
		}

		#[cfg(target_os = "windows")]
		let candidates = self.msvc(class, slot);

		#[cfg(target_os = "linux")]
		let candidates = self.itanium(class, slot);

		let mut candidates = candidates.into_iter();
		let one = candidates.next()?;
		candidates.next().is_none().then_some(one)
	}

	// RTTI and vtable records must stay wholly inside a data snapshot, even
	// when a matched reference is adjacent to an executable section.
	fn rtti_read(&self, address: usize, len: usize) -> Option<&[u8]> {
		address.checked_add(len)?;
		self.sections
			.iter()
			.filter(|section| !section.executable)
			.find_map(|section| {
				section.address.checked_add(section.bytes.len())?;
				let offset = address.checked_sub(section.address)?;
				section.bytes.get(offset..offset.checked_add(len)?)
			})
	}

	fn valid_table(&self, address: usize, slot: usize) -> bool {
		let Some(bytes) = slot
			.checked_add(1)
			.and_then(|n| n.checked_mul(8))
			.and_then(|size| self.rtti_read(address, size))
		else {
			return false;
		};
		let Some(function) = slot
			.checked_mul(8)
			.and_then(|offset| word_at(bytes, offset))
		else {
			return false;
		};

		// Hook managers can retain trampolines after removing a handler, or
		// another plugin may have hooked the slot first. RTTI still identifies
		// the original table. Do not require its function to be inside the image.
		self.executable(function) || is_executable(function)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::util::Section;

	const BASE: usize = 0x10000;

	fn fixture() -> Image {
		Image {
			base: BASE,
			sections: vec![
				Section {
					address: BASE,
					bytes: vec![0; 1024],
					executable: false,
					writable: false,
				},
				Section {
					address: 0x20000,
					bytes: vec![0; 0x1000],
					executable: true,
					writable: false,
				},
			],
		}
	}

	#[test]
	fn itanium_excludes_secondary_and_truncated_tables() {
		let mut image = fixture();
		image.sections[0].bytes[0x100..0x10d].copy_from_slice(b"10CKickIssue\0");
		word(&mut image, 0x188, BASE + 0x100);
		word(&mut image, 0x208, BASE + 0x180);
		word(&mut image, 0x210 + 9 * 8, 0x20010);
		assert_eq!(image.itanium("CKickIssue", 9), [BASE + 0x210]);
		word(&mut image, 0x200, usize::MAX - 7);
		assert!(image.itanium("CKickIssue", 9).is_empty());
		word(&mut image, 0x200, 0);
		image.sections[0].bytes.truncate(0x210 + 9 * 8);
		assert!(image.itanium("CKickIssue", 9).is_empty());
	}

	#[test]
	fn malformed_locator_and_slot_arithmetic_cannot_wrap() {
		let mut image = fixture();
		assert!(!image.valid_table(BASE, usize::MAX));
		image.base = usize::MAX - 2047;
		image.sections[0].address = image.base;
		image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
		for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, u32::MAX)] {
			image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		}
		assert!(image.msvc("CKickIssue", 8).is_empty());
	}

	#[test]
	fn msvc_requires_unique_primary_locator_and_executable_slot() {
		let mut image = fixture();
		image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
		for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, 0x180)] {
			image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		}
		word(&mut image, 0x200, BASE + 0x180);
		word(&mut image, 0x208 + 8 * 8, 0x20010);
		assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x208]);
		word(&mut image, 0x280, BASE + 0x180);
		word(&mut image, 0x288 + 8 * 8, 0x20020);
		assert_eq!(image.msvc("CKickIssue", 8).len(), 2);
		#[cfg(target_os = "windows")]
		assert!(image.primary_vtable("CKickIssue", 8).is_none());
		word(&mut image, 0x208 + 8 * 8, 0x30000);
		assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x288]);
	}

	#[test]
	fn retained_hook_trampolines_may_be_outside_the_game_image() {
		unsafe extern "C" fn trampoline() {}
		let mut image = fixture();
		word(&mut image, 0x100 + 8 * 8, trampoline as *const () as usize);
		assert!(image.valid_table(BASE + 0x100, 8));
		// An ordinary data allocation is never a valid replacement for code.
		let data = [0_u8; 32];
		word(&mut image, 0x100 + 8 * 8, data.as_ptr() as usize);
		assert!(!image.valid_table(BASE + 0x100, 8));
	}

	#[test]
	fn vtable_records_cannot_use_executable_snapshot_bytes() {
		let mut image = fixture();
		image.sections[1].bytes[8 * 8..9 * 8].copy_from_slice(&0x20010_usize.to_le_bytes());
		assert!(!image.valid_table(0x20000, 8));
		word(&mut image, 0x100 + 8 * 8, 0x20010);
		assert!(image.valid_table(BASE + 0x100, 8));
	}

	fn word(image: &mut Image, offset: usize, value: usize) {
		image.sections[0].bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
	}
}
