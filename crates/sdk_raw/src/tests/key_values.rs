//! Tests of reading key values' names through a mock key values system.

use super::*;
use std::ptr::null;

/// The names of the mock table, by symbol.
const NAMES: [&CStr; 3] = [c"AchievementEarned", c"MVM_Upgrade", c"+inspect_server"];

/// Key values as the engine lays them out, as far as their name: the symbol
/// in the first of their 64 bytes.
#[repr(C, align(8))]
struct KeyValues {
	symbol: HKeySymbol,
	rest: [u8; 60],
}

impl KeyValues {
	fn named(symbol: HKeySymbol) -> Self {
		Self {
			symbol,
			rest: [0xCD; 60],
		}
	}

	fn ptr(&mut self) -> NonNull<sys::KeyValues> {
		NonNull::from(self).cast()
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
		let name = unsafe { CStr::from_ptr(key_name(system, key_values.ptr())) };

		assert_eq!(name, expected);
		assert_eq!(key_values.rest, [0xCD; 60]);
	}
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
	assert_eq!(size_of::<KeyValues>(), size_of::<sys::KeyValues>());
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

/// The mock system's `GetSymbolForString`, which no test calls.
unsafe extern "C" fn symbol_for_string(
	_this: *mut IKeyValuesSystem,
	_name: *const c_char,
	_create: bool,
) -> HKeySymbol {
	unreachable!("no test looks up a symbol");
}
