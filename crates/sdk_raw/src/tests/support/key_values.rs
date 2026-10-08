//! Key values laid out as TF2's tier1 lays them out, which keep their names in
//! one table of their own, so that tests read them without a key values
//! system.

use std::ffi::{CStr, c_char, c_int};
use std::ptr::{self, NonNull};

// The layout TF2's tier1 has, which `key_values` reads.
const _: () = assert!(size_of::<MockKeyValues>() == 72 && size_of::<NameTable>() == 120);

/// Key values a test describes: a name, and either a string value or key
/// values of their own.
#[derive(Debug, Clone)]
pub enum MockKey<'a> {
	/// Key values whose value is this string.
	String(&'a CStr, &'a CStr),

	/// Key values that hold these key values, in order.
	Section(&'a CStr, Vec<MockKey<'a>>),
}

/// Key values as TF2's tier1 lays them out, with their names in a table of
/// their own.
#[repr(C, align(8))]
struct MockKeyValues {
	/// `m_iKeyName`: the offset of the name in the table's strings.
	symbol: c_int,

	/// `m_sValue`.
	string: *mut c_char,

	/// `m_wsValue`.
	wide_string: *mut u8,

	/// The union of `m_iValue`, `m_flValue`, `m_pValue` and `m_Color`.
	value: u64,

	/// `m_iDataType`.
	data_type: u8,

	/// `m_bHasEscapeSequences`.
	escapes: u8,

	/// `m_bEvaluateConditionals`.
	conditionals: u8,

	/// The flag TF2's tier1 keeps in the byte the SDK's header leaves `unused`.
	own_names: u8,

	/// `m_pPeer`.
	peer: *mut MockKeyValues,

	/// `m_pSub`.
	sub: *mut MockKeyValues,

	/// `m_pChain`.
	chain: *mut MockKeyValues,

	/// The table of names, after the SDK's 64 bytes.
	table: *const NameTable,
}

/// A tree of key values, which frees itself when dropped.
///
/// Every pointer into it stays valid until then: the key values, their
/// strings and their table are each boxed.
#[derive(Debug)]
pub struct MockTree {
	/// The key values, the root first.
	keys: Vec<NonNull<MockKeyValues>>,

	/// The table's strings, each followed by its NUL.
	_names: Box<[u8]>,

	/// The key values' string values, each followed by its NUL.
	strings: Vec<NonNull<[u8]>>,

	table: Box<NameTable>,
}

impl MockTree {
	/// Builds the key values `root` describes.
	///
	/// # Panics
	///
	/// If the names do not fit a `c_int` of offsets.
	pub fn new(root: &MockKey<'_>) -> Self {
		let mut names = vec![0];

		collect_names(root, &mut names);

		let names = names.into_boxed_slice();
		let table = Box::new(NameTable::of(&names));

		let mut tree = Self {
			keys: Vec::new(),
			_names: names,
			strings: Vec::new(),
			table,
		};

		tree.build(root);
		tree
	}

	/// The root key values.
	pub fn root(&self) -> NonNull<sys::KeyValues> {
		self.keys[0].cast()
	}

	/// The current string value of the key values named `name` in the
	/// section of the root's key values named `section`, as the tree holds
	/// it, or `None` if there is no such value.
	pub fn string(&self, section: &CStr, name: &CStr) -> Option<&CStr> {
		let section = self.find(self.keys[0], section)?;
		let key = self.find(section, name)?;

		// SAFETY: The tree owns its key values and their strings, which live
		// until it drops.
		let string = unsafe { key.as_ref() }.string;

		// SAFETY: As above; each string is NUL-terminated.
		(!string.is_null()).then(|| unsafe { CStr::from_ptr(string) })
	}

	/// Boxes the key values `key` describes, and those it holds, and returns
	/// them.
	fn build(&mut self, key: &MockKey<'_>) -> NonNull<MockKeyValues> {
		let (name, string, data_type, keys) = match key {
			MockKey::String(name, value) => {
				let bytes = Box::<[u8]>::from(value.to_bytes_with_nul());
				let string = NonNull::from(Box::leak(bytes));

				self.strings.push(string);
				(*name, string.cast::<c_char>().as_ptr(), 1, &[][..])
			}

			MockKey::Section(name, keys) => (*name, ptr::null_mut(), 0, &keys[..]),
		};

		let mock = Box::new(MockKeyValues {
			symbol: self.table.symbol(name),
			string,
			wide_string: ptr::null_mut(),
			value: 0,
			data_type,
			escapes: 0,
			conditionals: 1,
			own_names: 1,
			peer: ptr::null_mut(),
			sub: ptr::null_mut(),
			chain: ptr::null_mut(),
			table: &raw const *self.table,
		});

		let built = NonNull::from(Box::leak(mock));

		self.keys.push(built);

		let mut previous: Option<NonNull<MockKeyValues>> = None;

		for key in keys {
			let child = self.build(key);

			// SAFETY: The tree owns its key values, and no reference to them
			// is held.
			unsafe {
				match previous {
					Some(previous) => (*previous.as_ptr()).peer = child.as_ptr(),
					None => (*built.as_ptr()).sub = child.as_ptr(),
				}
			}

			previous = Some(child);
		}

		built
	}

	/// The first of `parent`'s key values named `name`.
	fn find(&self, parent: NonNull<MockKeyValues>, name: &CStr) -> Option<NonNull<MockKeyValues>> {
		let symbol = self.table.symbol(name);

		// SAFETY: The tree owns its key values, which live until it drops.
		let mut key = unsafe { parent.as_ref() }.sub;

		while let Some(found) = NonNull::new(key) {
			// SAFETY: As above.
			let found_ref = unsafe { found.as_ref() };

			if found_ref.symbol == symbol {
				return Some(found);
			}

			key = found_ref.peer;
		}

		None
	}
}

impl Drop for MockTree {
	fn drop(&mut self) {
		for key in self.keys.drain(..) {
			// SAFETY: Each was leaked from a box by `build`, and is freed once.
			drop(unsafe { Box::from_raw(key.as_ptr()) });
		}

		for string in self.strings.drain(..) {
			// SAFETY: As above.
			drop(unsafe { Box::from_raw(string.as_ptr()) });
		}
	}
}

/// A table of names that key values keep for themselves, as TF2's tier1 lays
/// it out, as far as its strings: the `CUtlVector<char>` after its mutex and
/// its hash of symbols.
#[repr(C, align(8))]
#[derive(Debug)]
struct NameTable {
	mutex_and_hash: [u8; 88],
	memory: *const c_char,
	allocation_count: c_int,
	grow_size: c_int,
	size: c_int,
	elements: *const c_char,
}

impl NameTable {
	/// A table whose strings are `strings`, each followed by its NUL.
	fn of(strings: &[u8]) -> Self {
		let memory = strings.as_ptr().cast::<c_char>();
		let size = c_int::try_from(strings.len()).unwrap();

		Self {
			mutex_and_hash: [0xCD; 88],
			memory,
			allocation_count: size,
			grow_size: 0,
			size,
			elements: memory,
		}
	}

	/// The offset of `name` in the table's strings.
	///
	/// # Panics
	///
	/// If the table lacks the name.
	fn symbol(&self, name: &CStr) -> c_int {
		let size = usize::try_from(self.size).unwrap();

		// SAFETY: The table's strings are `size` bytes, which the tree keeps.
		let strings = unsafe { std::slice::from_raw_parts(self.memory.cast::<u8>(), size) };
		let name = name.to_bytes_with_nul();

		let offset = (0..strings.len())
			.find(|&offset| {
				(offset == 0 || strings[offset - 1] == 0) && strings[offset..].starts_with(name)
			})
			.expect("the table has every name");

		c_int::try_from(offset).unwrap()
	}
}

/// Adds the names of `key` and those it holds to `names`, each followed by its
/// NUL, unless already there.
fn collect_names(key: &MockKey<'_>, names: &mut Vec<u8>) {
	let (name, keys) = match key {
		MockKey::String(name, _) => (*name, &[][..]),
		MockKey::Section(name, keys) => (*name, &keys[..]),
	};

	let bytes = name.to_bytes_with_nul();
	let present = (0..names.len()).any(|offset| {
		(offset == 0 || names[offset - 1] == 0) && names[offset..].starts_with(bytes)
	});

	if !present {
		names.extend_from_slice(bytes);
	}

	for key in keys {
		collect_names(key, names);
	}
}
