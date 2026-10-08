//! Tests of finding an entity's think contexts through `CBaseEntity`'s
//! datamap, and of which of them count as scheduled.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap};
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::utl_vector;
use std::alloc::Layout;
use std::mem::zeroed;
use std::ptr::null_mut;

/// A context named `name`, scheduled at `tick`.
fn context(name: &'static CStr, tick: c_int) -> sys::thinkfunc_t {
	// SAFETY: Zero is valid for every field of `thinkfunc_t`.
	let mut context: sys::thinkfunc_t = unsafe { zeroed() };

	context.m_iszContext.pszValue = name.as_ptr();
	context.m_nNextThinkTick = tick;
	context
}

#[test]
fn only_contexts_at_positive_ticks_are_scheduled() {
	let mut contexts = [context(c"DieContext", 2000), context(c"FlyContext", 0)];
	let mut mock = MockEntity::with_layout(5, Layout::new::<sys::CBaseEntity>());
	let fields = vec![field(
		c"m_aThinkFunctions",
		sys::_fieldtypes_FIELD_CUSTOM,
		raw::THINK_FUNCTIONS_OFFSET,
	)];

	set_datamap(data_map(c"CBaseEntity", fields, null_mut()));

	// SAFETY: The mock's storage has the layout of a `CBaseEntity`.
	unsafe {
		mock.as_ptr()
			.byte_add(raw::THINK_FUNCTIONS_OFFSET)
			.cast::<raw::ThinkFunctions>()
			.write(utl_vector(&mut contexts));
	}

	let entity = mock.entity();

	assert_eq!(entity.next_think_tick(c"DieContext"), Ok(Some(2000)));
	assert_eq!(entity.next_think_tick(c"FlyContext"), Ok(None));
	assert_eq!(entity.next_think_tick(c"Missing"), Ok(None));

	// A context at tick 0 or less does not run, so it is not cancelled.
	assert_eq!(entity.cancel_think_context(c"FlyContext"), Ok(false));
	assert_eq!(entity.cancel_think_context(c"Missing"), Ok(false));
	assert_eq!(entity.cancel_think_context(c"DieContext"), Ok(true));
	assert_eq!(entity.next_think_tick(c"DieContext"), Ok(None));
	assert_eq!(entity.cancel_think_context(c"DieContext"), Ok(false));

	assert_eq!(contexts[0].m_nNextThinkTick, raw::TICK_NEVER_THINK);
	assert_eq!(contexts[1].m_nNextThinkTick, 0);
}
