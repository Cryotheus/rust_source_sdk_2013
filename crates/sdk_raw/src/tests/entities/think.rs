//! Tests of the lookup of `m_aThinkFunctions` in `CBaseEntity`'s datamap, and
//! of reading and writing think contexts as the game lays them out.

use super::*;
use crate::test_support::entities::{data_map, field};
use crate::test_support::utl_vector;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

/// A context named `name`, scheduled at `tick`, which last ran at tick 1.
fn context(name: *const std::ffi::c_char, tick: c_int) -> sys::thinkfunc_t {
	// SAFETY: Zero is valid for every field of `thinkfunc_t`.
	let mut context: sys::thinkfunc_t = unsafe { zeroed() };

	context.m_iszContext.pszValue = name;
	context.m_nNextThinkTick = tick;
	context.m_nLastThinkTick = 1;
	context
}

#[test]
fn contexts_are_found_and_written_as_the_game_lays_them_out() {
	let mut contexts = [
		context(null(), 7),
		context(c"FlyContext".as_ptr(), 0),
		context(c"DieContextWithANameLongerThan32Bytes".as_ptr(), 2000),
		context(c"Hidden".as_ptr(), 50),
	];
	let mut entity = entity_with(&mut contexts[..3]);
	let entity = &raw mut *entity;
	let offset = THINK_FUNCTIONS_OFFSET;

	// SAFETY: The entity holds a vector of the contexts at the offset, named
	// by NUL-terminated strings or null.
	unsafe {
		// A null name reads as empty, and names match in their first 32 bytes,
		// as `strncmp` compares them.
		assert_eq!(think_context_index(entity, offset, c""), Some(0));
		assert_eq!(think_context_index(entity, offset, c"FlyContext"), Some(1));
		assert_eq!(think_context_index(entity, offset, c"FlyContex"), None);
		assert_eq!(
			think_context_index(
				entity,
				offset,
				c"DieContextWithANameLongerThan32B, then differs"
			),
			Some(2)
		);

		// The search stops at `m_Size`, though the memory holds more.
		assert_eq!(think_context_index(entity, offset, c"Hidden"), None);
		assert_eq!(think_context_tick(entity, offset, 3), None);
		assert!(!set_think_context_tick(entity, offset, 3, TICK_NEVER_THINK));

		assert_eq!(think_context_tick(entity, offset, 2), Some(2000));
		assert!(set_think_context_tick(entity, offset, 2, TICK_NEVER_THINK));
		assert_eq!(
			think_context_tick(entity, offset, 2),
			Some(TICK_NEVER_THINK)
		);
	}

	// Only the tick changed, in the context written.
	assert_eq!(contexts[2].m_nNextThinkTick, TICK_NEVER_THINK);
	assert_eq!(contexts[2].m_nLastThinkTick, 1);
	assert_eq!(contexts[3].m_nNextThinkTick, 50);
}

/// A zeroed entity whose `m_aThinkFunctions` views `contexts`, with room for
/// one more, and whose `m_pElements` is null, as only `m_Memory.m_pMemory` is
/// read.
fn entity_with(contexts: &mut [sys::thinkfunc_t]) -> Box<sys::CBaseEntity> {
	// SAFETY: Zero is valid for every field of `CBaseEntity`.
	let mut entity: Box<sys::CBaseEntity> = Box::new(unsafe { zeroed() });
	let mut vector = utl_vector(contexts);

	vector.m_Memory.m_nAllocationCount += 1;
	vector.m_pElements = null_mut();
	entity.m_aThinkFunctions = vector;
	entity
}

#[test]
fn only_the_custom_field_at_the_generated_offset_is_trusted() {
	let find = |offset, field_type| {
		let fields = vec![field(c"m_aThinkFunctions", field_type, offset)];
		let base = data_map(c"CBaseEntity", fields, null_mut());
		let map = data_map(c"CTFAmmoPack", Vec::new(), base);

		// SAFETY: Test maps are leaked, so they stay allocated and unmodified.
		find_think_functions(unsafe { DataMaps::new(map) })
	};

	// The game declares the vector as a custom field, without a size.
	assert_eq!(
		find(THINK_FUNCTIONS_OFFSET, sys::_fieldtypes_FIELD_CUSTOM),
		Some(THINK_FUNCTIONS_OFFSET)
	);
	assert_eq!(
		find(THINK_FUNCTIONS_OFFSET + 8, sys::_fieldtypes_FIELD_CUSTOM),
		None
	);
	assert_eq!(
		find(THINK_FUNCTIONS_OFFSET, sys::_fieldtypes_FIELD_EMBEDDED),
		None
	);
}

#[test]
fn ticks_are_computed_in_float_and_truncated() {
	assert_eq!(time_to_ticks(30.0, 0.015), 2000);
	assert_eq!(time_to_ticks(0.0074, 0.015), 0);
	assert_eq!(time_to_ticks(0.0075, 0.015), 1);

	// `(int)` truncates towards zero rather than flooring.
	assert_eq!(time_to_ticks(-1.0, 0.015), -66);
}
