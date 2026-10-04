//! Hand-written ABI of `CBaseEntity`'s think contexts in
//! `game/server/baseentity.h`: the `thinkfunc_t` layout of each context in
//! `m_aThinkFunctions`, the sentinel and name length of
//! `game/shared/shareddefs.h`, and ports of the inline lookups the game runs
//! on them.
//!
//! An entity schedules functions besides its main think in named contexts
//! (`SetContextThink`), such as the `DieContext` in which TF2's dropped ammo
//! packs remove themselves. Each context is a `thinkfunc_t` in the
//! `CUtlVector` `m_aThinkFunctions`, holding its function, its pooled name and
//! the tick it next runs at. `CBaseEntity`'s datamap declares the vector as a
//! `FIELD_CUSTOM` field, which gives no size or type, so
//! [`find_think_functions`] only trusts the offset it gives where the
//! generated layout has the vector too.
//!
//! Writing a tick here does what `SetNextThink(context, tick)` does, except
//! for `CheckHasThinkFunction`, which keeps `EFL_NO_THINK_FUNCTION` and the
//! entity list's set of thinking entities in step: the game runs that the
//! next time it schedules any think of the entity, and a context whose tick is
//! [`TICK_NEVER_THINK`] does not run in the meantime.

#[cfg(test)]
#[path = "../tests/entities/think.rs"]
mod tests;

use crate::entities::datamap::DataMaps;
use std::ffi::{CStr, c_char, c_int};
use std::mem::offset_of;

/// The `CUtlVector<thinkfunc_t>` that holds an entity's think contexts,
/// `CBaseEntity::m_aThinkFunctions`.
#[doc(alias("m_aThinkFunctions"))]
pub type ThinkFunctions = sys::CUtlVector<sys::thinkfunc_t, sys::CUtlMemory<sys::thinkfunc_t>>;

// The layout the game reads and writes contexts with. TF2's 64-bit Windows
// `server.dll` indexes the vector's `m_Memory.m_pMemory` with a stride of 24
// bytes, reads names at 8 and writes ticks at 16 (`CBaseEntity::ThinkSet`),
// and bounds the search by `m_Size` at 16 bytes into the vector
// (`CBaseEntity::GetIndexForThinkContext`), as the generated layout does. On
// Linux, which was not checked against a binary, the member function pointer
// `m_pfnThink` takes 16 bytes under the Itanium ABI, which moves the rest by
// 8.
const _: () = {
	use sys::thinkfunc_t as Context;

	assert!(
		size_of::<Context>()
			== cfg_select! {
				target_os = "windows" => 24,
				target_os = "linux" => 32,
			}
	);
	assert!(
		offset_of!(Context, m_iszContext)
			== cfg_select! {
				target_os = "windows" => 8,
				target_os = "linux" => 16,
			}
	);
	assert!(
		offset_of!(Context, m_nNextThinkTick)
			== cfg_select! {
				target_os = "windows" => 16,
				target_os = "linux" => 24,
			}
	);
	assert!(offset_of!(ThinkFunctions, m_Memory) == 0);
	assert!(offset_of!(sys::CUtlMemory<Context>, m_pMemory) == 0);
	assert!(offset_of!(ThinkFunctions, m_Size) == 16);
	assert!(
		THINK_FUNCTIONS_OFFSET
			== cfg_select! {
				target_os = "windows" => 240,
				target_os = "linux" => 256,
			}
	);
};

/// `MAX_CONTEXT_LENGTH` from `game/shared/shareddefs.h`: how many bytes of a
/// context's name `GetIndexForThinkContext` compares, with `strncmp`.
pub const MAX_CONTEXT_LENGTH: usize = 32;

/// The offset of `m_aThinkFunctions` in the generated `CBaseEntity`, TF2's,
/// where TF2's 64-bit Windows `server.dll` has it.
#[doc(alias("m_aThinkFunctions"))]
pub const THINK_FUNCTIONS_OFFSET: usize = offset_of!(sys::CBaseEntity, m_aThinkFunctions);

/// `TICK_NEVER_THINK` from `game/shared/shareddefs.h`: the tick of a context
/// that is not scheduled. `PhysicsRunSpecificThink` skips every tick of 0 or
/// less.
pub const TICK_NEVER_THINK: c_int = -1;

/// The context at `index` of the entity's think contexts, or `None` past
/// their count, `m_Size`.
///
/// # Safety
///
/// As for [`think_context_index`].
unsafe fn context_at(
	entity: *mut sys::CBaseEntity,
	offset: usize,
	index: usize,
) -> Option<*mut sys::thinkfunc_t> {
	// SAFETY: The caller guarantees the vector lies at `offset` of the live
	// entity. Its members are read without forming references, as the game
	// writes them through its own.
	let (elements, size) = unsafe {
		let vector = entity.byte_add(offset).cast::<ThinkFunctions>();

		(
			(&raw const (*vector).m_Memory.m_pMemory).read(),
			(&raw const (*vector).m_Size).read(),
		)
	};

	let size = usize::try_from(size).unwrap_or(0);

	// SAFETY: The vector's memory holds `m_Size` contexts. They are indexed as
	// the game indexes them, through `m_Memory.m_pMemory`, rather than through
	// `m_pElements`, a copy tier1 keeps for debuggers.
	(index < size && !elements.is_null()).then(|| unsafe { elements.add(index) })
}

/// Whether the context name `name` points to matches `wanted` in its first
/// [`MAX_CONTEXT_LENGTH`] bytes, as `strncmp` compares them. A null name is
/// empty, as `STRING` reads `NULL_STRING`.
///
/// # Safety
///
/// `name` must be null or point to a NUL-terminated string, or to at least
/// [`MAX_CONTEXT_LENGTH`] readable bytes.
unsafe fn context_matches(name: *const c_char, wanted: &CStr) -> bool {
	let wanted = wanted.to_bytes();
	let wanted = &wanted[..wanted.len().min(MAX_CONTEXT_LENGTH)];

	if name.is_null() {
		return wanted.is_empty();
	}

	for index in 0..MAX_CONTEXT_LENGTH {
		// SAFETY: Bytes are only read up to the name's NUL, at which `wanted`,
		// which holds no NUL, stops matching, or up to the length compared.
		let byte = unsafe { name.add(index).read() } as u8;

		match wanted.get(index) {
			Some(&expected) if expected == byte => {}
			Some(_) => return false,
			None => return byte == 0,
		}
	}

	true
}

/// Finds the offset of `m_aThinkFunctions` in the entities whose data
/// description maps are `maps`.
///
/// The offset is that of the `FIELD_CUSTOM` field `CBaseEntity`'s own map
/// declares under that name, which a custom field declares without a size.
/// Returns `None` unless it is [`THINK_FUNCTIONS_OFFSET`], since the offset
/// alone does not show that the vector holds `thinkfunc_t`s as the generated
/// layout does: other layouts are refused rather than trusted.
#[doc(alias("m_aThinkFunctions"))]
pub fn find_think_functions(mut maps: DataMaps<'_>) -> Option<usize> {
	let map = maps.find(|map| map.class_name() == Some(c"CBaseEntity"))?;

	let field = map.fields().iter().find(|field| {
		field.fieldType == sys::_fieldtypes_FIELD_CUSTOM
			&& field.name() == Some(c"m_aThinkFunctions")
	})?;

	(field.offset()? == THINK_FUNCTIONS_OFFSET).then_some(THINK_FUNCTIONS_OFFSET)
}

/// Sets the tick at which the entity's think context at `index` next runs,
/// `m_nNextThinkTick`, and returns whether it has such a context, below its
/// count, `m_Size`.
///
/// [`TICK_NEVER_THINK`] unschedules the context, as
/// `SetNextThink(context, TICK_NEVER_THINK)` does. `EFL_NO_THINK_FUNCTION` is
/// left as it is, as the [module documentation](self) describes.
///
/// # Safety
///
/// As for [`think_context_index`]. The game writes the tick on the main
/// thread too, through its own pointers.
#[doc(alias("SetNextThink", "m_nNextThinkTick"))]
pub unsafe fn set_think_context_tick(
	entity: *mut sys::CBaseEntity,
	offset: usize,
	index: usize,
	tick: c_int,
) -> bool {
	// SAFETY: The caller upholds `context_at`'s contract.
	let Some(context) = (unsafe { context_at(entity, offset, index) }) else {
		return false;
	};

	// SAFETY: The context lies in the vector's memory, and its tick is written
	// without forming a reference.
	unsafe { (&raw mut (*context).m_nNextThinkTick).write(tick) };

	true
}

/// The index of the entity's think context named `context`, as
/// `CBaseEntity::GetIndexForThinkContext` finds it, or `None` if it has none.
///
/// Names are compared in their first [`MAX_CONTEXT_LENGTH`] bytes, as
/// `strncmp` compares them, and a context without a name has an empty one.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL, whose
///   `m_aThinkFunctions` lies at `offset`, as [`find_think_functions`] finds it
///   in the DLL's datamaps, and the call must be made on the server's main
///   thread, the only one that changes the vector.
/// - Each context's name must be null or a NUL-terminated string, as the
///   pooled strings the game names contexts with are.
#[doc(alias("GetIndexForThinkContext"))]
pub unsafe fn think_context_index(
	entity: *mut sys::CBaseEntity,
	offset: usize,
	context: &CStr,
) -> Option<usize> {
	(0..)
		.map_while(|index| {
			// SAFETY: The caller upholds `context_at`'s contract.
			let element = unsafe { context_at(entity, offset, index) }?;

			// SAFETY: The context lies in the vector's memory, and its name is read
			// without forming a reference, then compared as the caller allows.
			Some(unsafe {
				context_matches(
					(&raw const (*element).m_iszContext.pszValue).read(),
					context,
				)
			})
		})
		.position(|matches| matches)
}

/// The tick at which the entity's think context at `index` next runs,
/// `m_nNextThinkTick`, or `None` past its count of contexts, `m_Size`.
///
/// A tick of 0 or less, such as [`TICK_NEVER_THINK`], means the context is not
/// scheduled.
///
/// # Safety
///
/// As for [`think_context_index`].
#[doc(alias("GetNextThinkTick", "m_nNextThinkTick"))]
pub unsafe fn think_context_tick(
	entity: *mut sys::CBaseEntity,
	offset: usize,
	index: usize,
) -> Option<c_int> {
	// SAFETY: The caller upholds `context_at`'s contract.
	let context = unsafe { context_at(entity, offset, index) }?;

	// SAFETY: The context lies in the vector's memory, and its tick is read
	// without forming a reference.
	Some(unsafe { (&raw const (*context).m_nNextThinkTick).read() })
}

/// The tick at which a think scheduled at `time` runs, in `float` arithmetic,
/// as `TIME_TO_TICKS` in `game/shared/shareddefs.h` computes it from
/// `TICK_INTERVAL`, the seconds per tick, `interval`: `0.5f + time /
/// interval`, truncated towards zero.
///
/// Unlike C++, whose conversion is undefined for them, a quotient beyond
/// `int`'s range saturates to its bounds, and NaN gives 0.
#[doc(alias("TIME_TO_TICKS"))]
pub fn time_to_ticks(time: f32, interval: f32) -> c_int {
	(0.5 + time / interval) as c_int
}
