use super::{Error, SignaturePattern, pattern, platform, relative};

/// An owned snapshot of a module's readable image regions.
#[derive(Debug, Clone)]
pub struct Image {
	pub base: usize,
	pub sections: Vec<Section>,
}

impl Image {
	/// Snapshot the module containing an executable address.
	///
	/// # Safety
	/// The caller must keep the module containing `address` loaded throughout
	/// this call, including any loader-owned filename read on Linux. Addresses
	/// in the result are only usable while the original module remains loaded.
	pub unsafe fn load(address: usize) -> Result<Self, Error> {
		// SAFETY: The caller guarantees the loader metadata remains live.
		unsafe { platform::load(address) }
	}

	/// Resolve an x86 CALL rel32, requiring its destination inside image code.
	pub fn call(&self, address: usize) -> Option<usize> {
		let bytes = self.read(address, 5)?;
		if bytes[0] != 0xe8 {
			return None;
		}
		let target = relative(address, bytes, 1)?;
		self.executable(target).then_some(target)
	}

	/// Check that a range belongs to one region with the requested permissions.
	/// `false` means the corresponding permission is not required.
	pub fn contains(&self, address: usize, len: usize, executable: bool, writable: bool) -> bool {
		self.sections.iter().any(|section| {
			(!executable || section.executable)
				&& (!writable || section.writable)
				&& section
					.address
					.checked_add(section.bytes.len())
					.is_some_and(|end| {
						address >= section.address
							&& address.checked_add(len).is_some_and(|last| last <= end)
					})
		})
	}

	/// Check whether an address lies in executable image bytes.
	pub fn executable(&self, address: usize) -> bool {
		self.contains(address, 1, true, false)
	}

	/// Find aligned exact byte strings in non-executable data regions.
	/// Empty strings or zero alignment have no matches.
	pub fn matches(&self, bytes: &[u8], alignment: usize) -> Vec<usize> {
		if bytes.is_empty() || alignment == 0 {
			return Vec::new();
		}
		self.sections
			.iter()
			.filter(|section| !section.executable)
			.filter(|section| section.address.checked_add(section.bytes.len()).is_some())
			.flat_map(|section| {
				section.bytes.windows(bytes.len()).enumerate().filter_map(
					move |(offset, candidate)| {
						let address = section.address.checked_add(offset)?;
						(address % alignment == 0 && candidate == bytes).then_some(address)
					},
				)
			})
			.collect()
	}

	/// Read bytes wholly contained in one snapshotted region.
	pub fn read(&self, address: usize, len: usize) -> Option<&[u8]> {
		address.checked_add(len)?;
		self.sections.iter().find_map(|section| {
			section.address.checked_add(section.bytes.len())?;
			let offset = address.checked_sub(section.address)?;
			section.bytes.get(offset..offset.checked_add(len)?)
		})
	}

	/// Find exactly one aligned signature in executable regions.
	/// Empty signatures, zero alignment, and ambiguous matches return `None`.
	pub fn unique(&self, signature: &[SignaturePattern], alignment: usize) -> Option<usize> {
		if signature.is_empty() || alignment == 0 {
			return None;
		}
		let mut found = None;
		for section in self.sections.iter().filter(|section| section.executable) {
			section.address.checked_add(section.bytes.len())?;
			let start = (alignment - section.address % alignment) % alignment;
			let Some(bytes) = section.bytes.get(start..) else {
				continue;
			};
			for (index, bytes) in bytes
				.windows(signature.len())
				.step_by(alignment)
				.enumerate()
			{
				if pattern(bytes, signature) {
					if found.is_some() {
						return None;
					}
					found = Some(
						section
							.address
							.checked_add(start)?
							.checked_add(index.checked_mul(alignment)?)?,
					);
				}
			}
		}
		found
	}
}

/// Owned bytes and permissions for one readable image region.
#[derive(Debug, Clone)]
pub struct Section {
	pub address: usize,
	pub bytes: Vec<u8>,
	pub executable: bool,
	pub writable: bool,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::sig;

	#[test]
	fn ranges_alignment_and_permissions_are_checked() {
		let mut image = Image {
			base: 1,
			sections: vec![Section {
				address: 1,
				bytes: vec![0; 33],
				executable: true,
				writable: false,
			}],
		};
		image.sections[0].bytes[15] = 0xff;
		assert_eq!(image.unique(&sig![0xff], 16), Some(16));
		assert!(!image.contains(1, 1, false, true));
		assert!(!image.contains(33, 2, false, false));
		assert!(!image.contains(usize::MAX, 2, false, false));
		assert!(image.matches(&[0xff], 1).is_empty());
		image.sections[0].executable = false;
		assert_eq!(image.matches(&[0xff], 16), [16]);
		assert!(image.matches(&[], 1).is_empty());
		assert!(image.matches(&[0xff], 0).is_empty());
		image.sections[0].address = usize::MAX;
		assert!(image.read(usize::MAX, 1).is_none());
		assert!(image.matches(&[0xff], 1).is_empty());
	}

	#[test]
	fn signatures_reject_ambiguity_and_calls_outside_executable_sections() {
		let mut image = Image {
			base: 0x1000,
			sections: vec![Section {
				address: 0x1000,
				bytes: vec![0x90; 256],
				executable: true,
				writable: false,
			}],
		};
		image.sections[0].bytes[0x10..0x13].copy_from_slice(&[0x48, 0x89, 0xff]);
		assert!(image.unique(&sig![], 16).is_none());
		assert!(image.unique(&sig![0x48 ? 0xff], 0).is_none());
		assert_eq!(image.unique(&sig![0x48 ? 0xff], 16), Some(0x1010));
		image.sections[0].bytes[0x20..0x23].copy_from_slice(&[0x48, 0x89, 0xff]);
		assert!(image.unique(&sig![0x48 ? 0xff], 16).is_none());
		image.sections[0].bytes[0x30..0x35].copy_from_slice(&[0xe8, 0, 0, 0, 0]);
		assert_eq!(image.call(0x1030), Some(0x1035));
		image.sections[0].bytes[0x31..0x35].copy_from_slice(&0x1000_i32.to_le_bytes());
		assert!(image.call(0x1030).is_none());
	}
}
