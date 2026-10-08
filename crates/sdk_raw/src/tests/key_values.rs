//! Tests of reading key values' names through a mock key values system.

use super::*;
use std::ptr::null;

/// The names of the mock table, by symbol.
const NAMES: [&CStr; 3] = [c"AchievementEarned", c"MVM_Upgrade", c"+inspect_server"];

/// The strings of a mock table that key values keep for themselves, which
/// starts with an empty name, as TF2's does.
const OWN_STRINGS: &[u8] = b"\0AchievementEarned\0achievementID\0";

/// Key values as TF2's tier1 lays them out, as far as their name: the symbol
/// in their first 4 bytes, the flag for a table of names of their own in the
/// 36th, and the pointer to that table after the SDK's 64 bytes.
#[repr(C, align(8))]
struct KeyValues {
	symbol: HKeySymbol,
	before_flag: [u8; 31],
	own_names: u8,
	after_flag: [u8; 28],
	table: *const StringTable,
}

impl KeyValues {
	/// Key values named by `symbol` in the key values system's table.
	fn named(symbol: HKeySymbol) -> Self {
		Self {
			symbol,
			before_flag: [0xCD; 31],
			own_names: 0,
			after_flag: [0xCD; 28],
			table: null(),
		}
	}

	/// Key values named by `symbol` in `table`, their own.
	fn named_in(symbol: HKeySymbol, table: *const StringTable) -> Self {
		Self {
			own_names: 1,
			table,
			..Self::named(symbol)
		}
	}

	fn ptr(&mut self) -> NonNull<sys::KeyValues> {
		NonNull::from(self).cast()
	}
}

/// A table of names that key values keep for themselves, as TF2's tier1 lays
/// it out, as far as its strings: the `CUtlVector<char>` after its mutex and
/// its hash of symbols.
#[repr(C, align(8))]
struct StringTable {
	mutex_and_hash: [u8; 88],
	memory: *const c_char,
	allocation_count: c_int,
	grow_size: c_int,
	size: c_int,
	elements: *const c_char,
}

impl StringTable {
	/// A table whose strings are `strings`, each followed by its NUL.
	fn of(strings: &'static [u8]) -> Self {
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
}

#[test]
fn names_are_the_strings_of_the_symbols_in_the_first_field() {
	// A key values system whose table holds `NAMES`, by index.
	let vtable = IKeyValuesSystemVtable {
		register_sizeof_key_values: null(),
		alloc_key_values_memory: null(),
		free_key_values_memory: null(),
		get_symbol_for_string: symbol_for_string,
		get_string_for_symbol: string_for_symbol,
	};
	let mut system = IKeyValuesSystem {
		vtable_: &raw const vtable,
	};
	let system = NonNull::from(&mut system);

	for (symbol, expected) in [
		(0, NAMES[0]),
		(1, NAMES[1]),
		(2, NAMES[2]),
		(-1, c"unknown"),
	] {
		let mut key_values = KeyValues::named(symbol);

		// SAFETY: The mock system lives through the call, and the key values
		// are laid out as the engine's.
		let name = unsafe { CStr::from_ptr(key_name(Some(system), key_values.ptr())) };

		assert_eq!(name, expected);
	}

	// Without the system, its names cannot be found.
	let mut key_values = KeyValues::named(0);

	// SAFETY: The key values are laid out as the engine's.
	assert!(unsafe { key_name(None, key_values.ptr()) }.is_null());
}

#[test]
fn names_kept_by_the_key_values_are_offsets_into_their_own_table() {
	let table = StringTable::of(OWN_STRINGS);

	for (symbol, expected) in [
		(0, c""),
		(1, c"AchievementEarned"),
		(19, c"achievementID"),
		(23, c"evementID"),
		(-1, c""),
	] {
		let mut key_values = KeyValues::named_in(symbol, &raw const table);

		// SAFETY: The key values and their table are laid out as TF2's, and
		// need no key values system.
		let name = unsafe { CStr::from_ptr(key_name(None, key_values.ptr())) };

		assert_eq!(name, expected, "{symbol}");
	}
}

#[test]
fn names_not_in_their_own_table_are_not_found() {
	let table = StringTable::of(OWN_STRINGS);
	let size = table.size;

	let mut moved = StringTable::of(OWN_STRINGS);
	moved.elements = OWN_STRINGS[1..].as_ptr().cast();

	let mut empty = StringTable::of(OWN_STRINGS);
	empty.memory = null();
	empty.elements = null();

	for (symbol, table) in [
		(size, &raw const table),
		(-2, &raw const table),
		(1, &raw const moved),
		(1, &raw const empty),
		(1, null()),
	] {
		let mut key_values = KeyValues::named_in(symbol, table);

		// SAFETY: The key values and their tables, if any, are laid out as
		// TF2's.
		assert!(
			unsafe { key_name(None, key_values.ptr()) }.is_null(),
			"{symbol}"
		);
	}

	// Unnamed key values are named "", as in the game, whatever their table.
	let mut key_values = KeyValues::named_in(-1, null());

	// SAFETY: As above.
	let name = unsafe { CStr::from_ptr(key_name(None, key_values.ptr())) };

	assert_eq!(name, c"");
}

#[test]
fn no_system_is_found_without_vstdlib() {
	// Test processes load no Source library.
	assert!(key_values_system().is_none());
}

#[test]
fn slots_follow_the_header() {
	assert_eq!(GET_SYMBOL_FOR_STRING_SLOT, 3);
	assert_eq!(GET_STRING_FOR_SYMBOL_SLOT, 4);
}

/// The mock system's `GetStringForSymbol`.
unsafe extern "C" fn string_for_symbol(
	_this: *mut IKeyValuesSystem,
	symbol: HKeySymbol,
) -> *const c_char {
	usize::try_from(symbol)
		.ok()
		.and_then(|index| NAMES.get(index))
		.map_or(c"unknown".as_ptr(), |name| name.as_ptr())
}

#[test]
fn sub_keys_are_listed_in_order_with_their_string_values() {
	use crate::test_support::key_values::{MockKey, MockTree};

	let tree = MockTree::new(&MockKey::Section(
		c"GameTags",
		vec![
			MockKey::Section(
				c"tag",
				vec![
					MockKey::String(c"convar", c"mp_friendlyfire"),
					MockKey::String(c"tag", c"friendlyfire"),
				],
			),
			MockKey::Section(c"empty", vec![]),
		],
	));

	let name = |key: NonNull<sys::KeyValues>| {
		// SAFETY: The mock key values keep their names in their own table.
		unsafe { CStr::from_ptr(key_name(None, key)) }
	};

	let string = |key: NonNull<sys::KeyValues>| {
		// SAFETY: The mock key values live as long as the tree, and their
		// strings are NUL-terminated.
		let string = unsafe { string_value(key) };

		// SAFETY: As above.
		(!string.is_null()).then(|| unsafe { CStr::from_ptr(string) })
	};

	// SAFETY: The tree lives through the test, and its key values are laid out
	// as TF2's.
	unsafe {
		let root = tree.root();
		let tag = NonNull::new(first_sub_key(root)).unwrap();
		let empty = NonNull::new(next_key(tag)).unwrap();

		assert_eq!(name(root), c"GameTags");
		assert_eq!(string(root), None);
		assert!(next_key(root).is_null());

		assert_eq!(name(tag), c"tag");
		assert_eq!(string(tag), None);

		assert_eq!(name(empty), c"empty");
		assert!(first_sub_key(empty).is_null());
		assert!(next_key(empty).is_null());

		let convar = NonNull::new(first_sub_key(tag)).unwrap();
		let tag_name = NonNull::new(next_key(convar)).unwrap();

		assert_eq!(name(convar), c"convar");
		assert_eq!(string(convar), Some(c"mp_friendlyfire"));
		assert_eq!(name(tag_name), c"tag");
		assert_eq!(string(tag_name), Some(c"friendlyfire"));
		assert!(next_key(tag_name).is_null());
	}
}

/// The mock system's `GetSymbolForString`, which no test calls.
unsafe extern "C" fn symbol_for_string(
	_this: *mut IKeyValuesSystem,
	_name: *const c_char,
	_create: bool,
) -> HKeySymbol {
	unreachable!("no test looks up a symbol");
}

#[test]
fn the_mocks_are_laid_out_as_tf2s() {
	assert_eq!(offset_of!(KeyValues, own_names), OWN_NAMES_FLAG_OFFSET);
	assert_eq!(offset_of!(KeyValues, table), OWN_NAMES_TABLE_OFFSET);
	assert_eq!(offset_of!(KeyValues, table), size_of::<sys::KeyValues>());
	assert_eq!(size_of::<KeyValues>(), 72);

	assert_eq!(offset_of!(StringTable, memory), OWN_NAMES_MEMORY_OFFSET);
	assert_eq!(offset_of!(StringTable, size), OWN_NAMES_SIZE_OFFSET);
	assert_eq!(offset_of!(StringTable, elements), OWN_NAMES_ELEMENTS_OFFSET);
	assert_eq!(size_of::<StringTable>(), 120);
}
