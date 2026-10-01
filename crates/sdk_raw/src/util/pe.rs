//! Bounded PE32+ x86-64 image parsing shared by live and file snapshots.

use super::{Error, Image, MAX_IMAGE_BYTES, Section, u16_at, u32_at, word_at};

pub(super) struct Headers {
	base: usize,
	pub size: usize,
	pub sections: Vec<SectionHeader>,
}

pub(super) struct SectionHeader {
	pub offset: usize,
	pub len: usize,
	raw_offset: usize,
	raw_len: usize,
	pub executable: bool,
	pub writable: bool,
}

impl Headers {
	pub fn read(
		mut read: impl FnMut(usize, usize) -> Result<Vec<u8>, Error>,
	) -> Result<Self, Error> {
		let invalid = || Error::InvalidImage;
		let dos = read(0, 64)?;
		if dos.get(..2) != Some(b"MZ") {
			return Err(invalid());
		}
		let pe = u32_at(&dos, 60).ok_or_else(invalid)? as usize;
		if !(64..=0x100000).contains(&pe) {
			return Err(invalid());
		}
		let coff = read(pe, 24)?;
		if coff.get(..4) != Some(b"PE\0\0") || u16_at(&coff, 4) != Some(0x8664) {
			return Err(invalid());
		}
		let count = u16_at(&coff, 6).ok_or_else(invalid)? as usize;
		let optional_size = u16_at(&coff, 20).ok_or_else(invalid)? as usize;
		if !(1..=96).contains(&count) || !(112..=4096).contains(&optional_size) {
			return Err(invalid());
		}
		let optional = read(pe + 24, optional_size)?;
		if u16_at(&optional, 0) != Some(0x20b) {
			return Err(invalid());
		}
		let base = word_at(&optional, 24).ok_or_else(invalid)?;
		let size = u32_at(&optional, 56).ok_or_else(invalid)? as usize;
		if !(pe + 24 + optional_size + count * 40..=MAX_IMAGE_BYTES).contains(&size) {
			return Err(invalid());
		}
		let headers = read(pe + 24 + optional_size, count * 40)?;
		if headers.len() != count * 40 {
			return Err(invalid());
		}
		let mut sections = Vec::new();
		let mut total = 0_usize;
		for header in headers.chunks_exact(40) {
			let len = u32_at(header, 8).ok_or_else(invalid)? as usize;
			let offset = u32_at(header, 12).ok_or_else(invalid)? as usize;
			let flags = u32_at(header, 36).ok_or_else(invalid)?;
			if offset.checked_add(len).is_none_or(|end| end > size) {
				return Err(invalid());
			}
			if len == 0 || flags & 0x40000000 == 0 {
				continue;
			}
			total = total
				.checked_add(len)
				.filter(|n| *n <= MAX_IMAGE_BYTES)
				.ok_or_else(invalid)?;
			sections.push(SectionHeader {
				offset,
				len,
				raw_offset: u32_at(header, 20).ok_or_else(invalid)? as usize,
				raw_len: u32_at(header, 16).ok_or_else(invalid)? as usize,
				executable: flags & 0x20000000 != 0,
				writable: flags & 0x80000000 != 0,
			});
		}
		Ok(Self {
			base,
			size,
			sections,
		})
	}
}

/// Snapshot a PE file using its preferred load address and virtual section sizes.
/// Relocations are not applied; use `Image::load` for live relocated RTTI.
pub fn from_file(bytes: &[u8]) -> Result<Image, Error> {
	let invalid = || Error::InvalidImage;
	let headers = Headers::read(|offset, len| {
		Ok(bytes
			.get(offset..offset.checked_add(len).ok_or_else(invalid)?)
			.ok_or_else(invalid)?
			.to_vec())
	})?;
	headers.base.checked_add(headers.size).ok_or_else(invalid)?;
	let mut sections = Vec::new();
	for header in headers.sections {
		let copy = header.raw_len.min(header.len);
		let data = bytes
			.get(header.raw_offset..header.raw_offset.checked_add(copy).ok_or_else(invalid)?)
			.ok_or_else(invalid)?;
		let mut snapshot = vec![0; header.len];
		snapshot[..copy].copy_from_slice(data);
		sections.push(Section {
			address: headers
				.base
				.checked_add(header.offset)
				.ok_or_else(invalid)?,
			bytes: snapshot,
			executable: header.executable,
			writable: header.writable,
		});
	}
	Ok(Image {
		base: headers.base,
		sections,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture() -> Vec<u8> {
		let mut bytes = vec![0; 0x204];
		bytes[..2].copy_from_slice(b"MZ");
		bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
		bytes[64..68].copy_from_slice(b"PE\0\0");
		bytes[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
		bytes[70..72].copy_from_slice(&1_u16.to_le_bytes());
		bytes[84..86].copy_from_slice(&112_u16.to_le_bytes());
		bytes[88..90].copy_from_slice(&0x20b_u16.to_le_bytes());
		bytes[112..120].copy_from_slice(&0x10000_usize.to_le_bytes());
		bytes[144..148].copy_from_slice(&0x2000_u32.to_le_bytes());
		for (offset, value) in [
			(208, 8_u32),
			(212, 0x1000),
			(216, 4),
			(220, 0x200),
			(236, 0x60000000),
		] {
			bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		}
		bytes[0x200..].copy_from_slice(&[1, 2, 3, 4]);
		bytes
	}

	#[test]
	fn pe_file_checks_headers_ranges_and_zero_fills_virtual_tail() {
		let bytes = fixture();
		let image = from_file(&bytes).unwrap();
		assert_eq!(
			image.read(0x11000, 8),
			Some([1, 2, 3, 4, 0, 0, 0, 0].as_slice())
		);
		assert!(image.executable(0x11000));
		for length in [0, 63, 88, 199, 239, 0x203] {
			assert!(from_file(&bytes[..length]).is_err());
		}
		let mut invalid = bytes.clone();
		invalid[68] = 0;
		assert!(from_file(&invalid).is_err());
		invalid = bytes.clone();
		invalid[212..216].copy_from_slice(&0x2000_u32.to_le_bytes());
		assert!(from_file(&invalid).is_err());
		invalid = bytes;
		invalid[112..120].copy_from_slice(&usize::MAX.to_le_bytes());
		assert!(from_file(&invalid).is_err());
	}
}
