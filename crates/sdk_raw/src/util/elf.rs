//! Bounded ELF64 symbol inspection, with optional validation against a loaded
//! Linux module. These utilities identify addresses; they do not call them or
//! establish the ABI of any function.

#[cfg(test)]
#[path = "../tests/util/elf.rs"]
mod tests;

#[cfg(target_os = "linux")]
use super::{Error, MemoryReader, Module, platform};

use super::{u16_at, u32_at, word_at};

/// A checked view of a little-endian x86-64 ET_DYN file's section table.
pub struct Elf<'a> {
	bytes: &'a [u8],
	headers: Vec<&'a [u8]>,
}

impl<'a> Elf<'a> {
	/// Checks that `bytes` is a little-endian x86-64 ET_DYN file whose section
	/// headers lie within it, or returns `None`.
	pub fn new(bytes: &'a [u8]) -> Option<Self> {
		if bytes.get(..7)? != b"\x7fELF\x02\x01\x01"
			|| u16_at(bytes, 18)? != 62
			|| u16_at(bytes, 16)? != 3
		{
			return None;
		}
		let offset = word_at(bytes, 40)?;
		let size = u16_at(bytes, 58)? as usize;
		let count = u16_at(bytes, 60)? as usize;
		if size != 64 || !(1..=4096).contains(&count) {
			return None;
		}
		let headers = bytes
			.get(offset..offset.checked_add(size.checked_mul(count)?)?)?
			.chunks_exact(size)
			.collect();
		Some(Self { bytes, headers })
	}

	fn section(&self, header: &[u8]) -> Option<&'a [u8]> {
		let offset = word_at(header, 24)?;
		let len = word_at(header, 32)?;
		self.bytes.get(offset..offset.checked_add(len)?)
	}

	/// Looks up one exact STT_FUNC name in SHT_SYMTAB. The function must have
	/// a unique definition, executable section and body of 1..=65536 bytes.
	/// Returns its image-relative virtual address and borrowed file bytes.
	/// Stripped files, duplicate names and malformed ranges return `None`.
	pub fn symbol(&self, name: &[u8]) -> Option<(usize, &'a [u8])> {
		let mut found = None;
		for header in &self.headers {
			if u32_at(header, 4)? != 2 || word_at(header, 56)? != 24 {
				continue;
			}
			let names_header = self.headers.get(u32_at(header, 40)? as usize)?;
			if u32_at(names_header, 4)? != 3 {
				return None;
			}
			let names = self.section(names_header)?;
			let symbols = self.section(header)?;
			if symbols.len() % 24 != 0 {
				return None;
			}
			for symbol in symbols.as_chunks::<24>().0 {
				let offset = u32_at(symbol, 0)? as usize;
				let tail = names.get(offset..)?;
				let end = tail.iter().position(|b| *b == 0)?;
				if &tail[..end] != name || symbol[4] & 15 != 2 {
					continue;
				}
				let section = self.headers.get(u16_at(symbol, 6)? as usize)?;
				if word_at(section, 8)? & 4 == 0 {
					return None;
				}
				let address = word_at(symbol, 8)?;
				let size = word_at(symbol, 16)?;
				if !(1..=65536).contains(&size) {
					return None;
				}
				let offset = address.checked_sub(word_at(section, 16)?)?;
				let body = self
					.section(section)?
					.get(offset..offset.checked_add(size)?)?;
				if found.is_some() {
					return None;
				}
				found = Some((address, body));
			}
		}
		found
	}
}

/// Owns an ELF file and checked load ranges for a Linux module. It does not
/// pin the library or promise that returned addresses remain loaded.
#[cfg(target_os = "linux")]
pub struct LoadedElf {
	bytes: Vec<u8>,
	module: Module,
	memory: MemoryReader,
	segments: Vec<platform::LoadSegment>,
}

#[cfg(target_os = "linux")]
impl LoadedElf {
	/// Opens the containing module and verifies its ELF/program headers match
	/// the loaded image. `address` must be in a readable executable segment.
	/// Files larger than 256 MiB are refused before allocating their contents.
	///
	/// # Safety
	/// The caller must keep the module loaded with unchanged image mappings
	/// throughout this call. Returned addresses require the same lifetime
	/// guarantee whenever they are subsequently used as native pointers.
	pub unsafe fn at(address: usize) -> Result<Self, Error> {
		use std::fs::File;
		use std::io::Read;
		const MAX_FILE_BYTES: usize = 0x10000000;
		// SAFETY: The caller keeps the module and loader filename alive.
		let module = unsafe { Module::at(address) }?;
		let file = File::open(module.path())?;
		if file.metadata()?.len() > MAX_FILE_BYTES as u64 {
			return Err(Error::InvalidImage);
		}
		let mut bytes = Vec::new();
		file.take(MAX_FILE_BYTES as u64 + 1)
			.read_to_end(&mut bytes)?;
		if bytes.len() > MAX_FILE_BYTES || Elf::new(&bytes).is_none() {
			return Err(Error::InvalidImage);
		}
		let memory = MemoryReader::open()?;
		let header = memory.copy(module.base(), 64)?;
		if bytes.get(..64) != Some(header.as_slice()) {
			return Err(Error::InvalidImage);
		}
		let programs = platform::program_headers(&memory, module.base(), &header)?;
		let offset = word_at(&header, 32).ok_or(Error::InvalidImage)?;
		let end = offset
			.checked_add(programs.len())
			.ok_or(Error::InvalidImage)?;
		if bytes.get(offset..end) != Some(programs.as_slice()) {
			return Err(Error::InvalidImage);
		}
		let segments = platform::load_segments(module.base(), &programs)?;
		if !segments
			.iter()
			.any(|segment| segment.contains(address, 1, true, false))
		{
			return Err(Error::InvalidImage);
		}
		Ok(Self {
			bytes,
			module,
			memory,
			segments,
		})
	}

	/// Checks readable PT_LOAD bounds and any additional requested flags.
	pub fn contains(&self, address: usize, len: usize, executable: bool, writable: bool) -> bool {
		self.segments
			.iter()
			.any(|segment| segment.contains(address, len, executable, writable))
	}

	/// The loaded module this file was checked against.
	pub fn module(&self) -> &Module {
		&self.module
	}

	/// Copies current memory only inside this image's readable load ranges.
	pub fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
		self.contains(address, len, false, false).then_some(())?;
		self.memory.copy(address, len).ok()
	}

	/// Resolves a unique function and verifies its complete live body matches
	/// the file. Replaced files, detours and text relocations are refused.
	/// Returned byte slices belong to the owned file, never native memory.
	pub fn resolve(&self, name: &[u8]) -> Option<(usize, &[u8])> {
		let (offset, body) = Elf::new(&self.bytes)?.symbol(name)?;
		let address = self.module.base().checked_add(offset)?;
		if !self.contains(address, body.len(), true, false) {
			return None;
		}
		let live = self.memory.copy(address, body.len()).ok()?;
		(live == body).then_some((address, body))
	}
}
