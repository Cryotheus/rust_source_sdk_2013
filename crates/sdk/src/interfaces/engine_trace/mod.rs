//! `IEngineTrace`, which traces rays and queries what the world contains.

#[cfg(test)]
#[path = "../../tests/interfaces/engine_trace.rs"]
mod tests;

use crate::entities::Entity;
use crate::math::Vector;
use sdk_raw::vcall;
use std::ffi::{c_int, c_uint};
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};

// `ITraceFilter` declares no destructor, so its two methods fill the vtable
// from its start under both ABIs, and `SKIP_ONE_VTABLE` and
// `WORLD_ONLY_VTABLE` are built as the generated vtable itself. A
// `SkipOneFilter` starts with the interface the engine calls.
const _: () = {
	assert!(
		std::mem::offset_of!(
			sys::ITraceFilter__bindgen_vtable,
			ITraceFilter_ShouldHitEntity
		) == 0
	);
	assert!(
		size_of::<sys::ITraceFilter__bindgen_vtable>() == 2 * size_of::<unsafe extern "C" fn()>()
	);
	assert!(std::mem::offset_of!(SkipOneFilter, interface) == 0);
};

/// `CONTENTS_GRATE`: alpha-tested grates, which bullets and sight pass
/// through, but solids do not.
pub const CONTENTS_GRATE: c_uint = 0x8;

/// `CONTENTS_MONSTER`: never on a brush. The game's trace filters only let a
/// trace hit an entity other than a brush, such as a player or a building,
/// when its mask has it.
pub const CONTENTS_MONSTER: c_uint = 0x200_0000;

/// `CONTENTS_MOVEABLE`: brushes that move, such as doors.
pub const CONTENTS_MOVEABLE: c_uint = 0x4000;

/// `CONTENTS_SOLID`: solid brushes.
pub const CONTENTS_SOLID: c_uint = 0x1;

/// `CONTENTS_WINDOW`: translucent brushes, such as glass.
pub const CONTENTS_WINDOW: c_uint = 0x2;

/// `MASK_SOLID`: everything that is normally solid, entities other than
/// brushes included.
pub const MASK_SOLID: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_MONSTER | CONTENTS_GRATE;

/// `MASK_SOLID_BRUSHONLY`: every brush a solid collides with.
pub const MASK_SOLID_BRUSHONLY: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_GRATE;

/// The vtable of the filter of [`EngineTrace::trace_line`], [`SkipOneFilter`],
/// which hits every entity but the one it skips, as the game's
/// `CTraceFilterSimple` does before it applies the game's own rules.
static SKIP_ONE_VTABLE: sys::ITraceFilter__bindgen_vtable = sys::ITraceFilter__bindgen_vtable {
	ITraceFilter_ShouldHitEntity: hits_all_but_skipped,
	ITraceFilter_GetTraceType: traces_everything,
};

/// The vtable of the filter of [`EngineTrace::trace_world_line`], which hits
/// no entity and asks the engine to trace the world alone, as the game's
/// `CTraceFilterWorldOnly` does.
static WORLD_ONLY_VTABLE: sys::ITraceFilter__bindgen_vtable = sys::ITraceFilter__bindgen_vtable {
	ITraceFilter_ShouldHitEntity: hits_no_entity,
	ITraceFilter_GetTraceType: traces_world_only,
};

/// Where a line traced through the world and its entities stopped, and what
/// it hit, from [`EngineTrace::trace_line`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineTrace<'s> {
	/// How far along the line the trace got, from 0 at its start to 1 at its
	/// end, which it reached without hitting anything.
	pub fraction: f32,

	/// Where the trace stopped.
	pub end: Vector,

	/// Whether the line starts inside something solid.
	pub start_solid: bool,

	/// Whether the whole line lies inside something solid.
	pub all_solid: bool,

	/// What the line hit, or `None` if it hit nothing: an entity, or the
	/// world's own, at index 0, for the world's brushes and static props.
	pub entity: Option<Entity<'s>>,
}

impl LineTrace<'_> {
	/// Whether the line hit something before its end.
	pub fn hit(&self) -> bool {
		self.fraction < 1.0
	}
}

/// The filter of [`EngineTrace::trace_line`]: the interface the engine calls,
/// followed by the entity it skips.
#[repr(C)]
struct SkipOneFilter {
	interface: sys::ITraceFilter,

	/// The skipped entity's `IHandleEntity`, or null to skip none.
	skip: *const sys::IHandleEntity,
}

/// Where a line traced through the world stopped, from
/// [`EngineTrace::trace_world_line`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldTrace {
	/// How far along the line the trace got, from 0 at its start to 1 at its
	/// end, which it reached without hitting anything.
	pub fraction: f32,

	/// Where the trace stopped.
	pub end: Vector,

	/// Whether the line starts inside the world's solid contents.
	pub start_solid: bool,

	/// Whether the whole line lies inside the world's solid contents.
	pub all_solid: bool,
}

impl WorldTrace {
	/// Whether the line hit the world before its end.
	pub fn hit(&self) -> bool {
		self.fraction < 1.0
	}
}

/// `CTraceFilterSimple::ShouldHitEntity` without the game's own rules: hits
/// every entity but the one a [`SkipOneFilter`] skips.
unsafe extern "C" fn hits_all_but_skipped(
	filter: *mut sys::ITraceFilter,
	entity: *mut sys::IHandleEntity,
	_contents_mask: c_int,
) -> bool {
	// SAFETY: The engine passes back the filter `trace_line` gave it, a live
	// `SkipOneFilter`, whose interface is its first field.
	let skip = unsafe { (*filter.cast::<SkipOneFilter>()).skip };

	!ptr::eq(entity.cast_const(), skip)
}

/// `CTraceFilterWorldOnly::ShouldHitEntity`.
unsafe extern "C" fn hits_no_entity(
	_filter: *mut sys::ITraceFilter,
	_entity: *mut sys::IHandleEntity,
	_contents_mask: c_int,
) -> bool {
	false
}

/// `CTraceFilter::GetTraceType`: the world and its entities alike.
unsafe extern "C" fn traces_everything(_filter: *const sys::ITraceFilter) -> sys::TraceType_t {
	sys::TraceType_t_TRACE_EVERYTHING
}

/// `CTraceFilterWorldOnly::GetTraceType`.
unsafe extern "C" fn traces_world_only(_filter: *const sys::ITraceFilter) -> sys::TraceType_t {
	sys::TraceType_t_TRACE_WORLD_ONLY
}

interface! {
	/// Traces rays and queries what the world contains (`IEngineTrace`).
	#[doc(alias("IEngineTrace"))]
	pub struct EngineTrace(sys::IEngineTrace) = Engine sdk_raw::interfaces::engine_trace::VERSION;
}

impl<'s> EngineTrace<'s> {
	/// The `CONTENTS_*` flags of the world and entities at a point, as defined
	/// in `public/bspflags.h`.
	#[doc(alias("GetPointContents"))]
	pub fn point_contents(self, position: Vector) -> c_int {
		let position = sys::Vector::from(position);

		// SAFETY: `Server::new` guarantees the interface is live, the position
		// is a local, and no entity is requested.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_GetPointContents(&position, ptr::null_mut()))
		}
	}

	/// Traces a line from `start` to `end` through the world and its solid
	/// entities but `skip`, as the game's `UTIL_TraceLine` does with
	/// `CTraceFilterSimple` passing over an entity: the line stops at the first
	/// whose contents are in `mask`, such as [`MASK_SOLID`].
	///
	/// The game's own rules are left out, so the line also stops at what the
	/// skipped entity owns, such as its projectiles, and at debris. The engine
	/// stops it at an entity other than a brush by the contents of its model
	/// alone, while the game's filters also require [`CONTENTS_MONSTER`] in
	/// `mask` for those.
	#[doc(alias("TraceRay", "UTIL_TraceLine", "CTraceFilterSimple"))]
	pub fn trace_line(
		self,
		start: Vector,
		end: Vector,
		mask: c_uint,
		skip: Option<Entity<'_>>,
	) -> LineTrace<'s> {
		let mut filter = SkipOneFilter {
			interface: sys::ITraceFilter {
				vtable_: &raw const SKIP_ONE_VTABLE,
			},
			// An entity's `IHandleEntity` is the first base of `IServerEntity`,
			// its own first base, so it shares the entity's address.
			skip: skip.map_or(ptr::null(), |skip| skip.as_ptr().cast_const().cast()),
		};

		// SAFETY: The filter is a local that outlives the call, and its methods
		// only read the filter.
		let trace = unsafe { self.trace_ray(start, end, mask, (&raw mut filter).cast()) };
		let hit = trace._base.fraction < 1.0 || trace._base.startsolid || trace._base.allsolid;

		LineTrace {
			fraction: trace._base.fraction,
			end: trace._base.endpos.into(),
			start_solid: trace._base.startsolid,
			all_solid: trace._base.allsolid,
			// SAFETY: The engine points a trace that hit something at the live
			// entity it hit, which stays allocated during `'s`, as entities are
			// freed at the end of a frame.
			entity: NonNull::new(trace.m_pEnt)
				.filter(|_| hit)
				.map(|entity| unsafe { Entity::from_raw(entity) }),
		}
	}

	/// Traces a line from `start` to `end`, as `filter` decides, with the ray
	/// `Ray_t::Init` makes for a line.
	///
	/// # Safety
	///
	/// `filter` must point to an `ITraceFilter` that stays live for the call,
	/// whose methods are sound to call with the entities the engine passes.
	unsafe fn trace_ray(
		self,
		start: Vector,
		end: Vector,
		mask: c_uint,
		filter: *mut sys::ITraceFilter,
	) -> sys::trace_t {
		let delta = *end - *start;
		let aligned = |vector: Vector| sys::VectorAligned {
			_base: sys::Vector::from(vector),
		};

		// As `Ray_t::Init` makes a line: no extents, and swept unless it has no
		// length.
		let ray = sys::Ray_t {
			m_Start: aligned(start),
			m_Delta: aligned(Vector(delta)),
			m_StartOffset: aligned(Vector::new(0.0, 0.0, 0.0)),
			m_Extents: aligned(Vector::new(0.0, 0.0, 0.0)),
			m_IsRay: true,
			m_IsSwept: delta.length_squared() != 0.0,
		};

		let mut trace = MaybeUninit::<sys::trace_t>::zeroed();

		// SAFETY: `Server::new` guarantees the interface is live. The ray and the
		// trace are locals that outlive the call, which is the only time the
		// engine uses them, and the caller vouches for the filter. The engine
		// fills the trace, which starts zeroed, as `CGameTrace`'s constructor
		// leaves it.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_TraceRay(&ray, mask, filter, trace.as_mut_ptr()))
		};

		// SAFETY: Every field of the trace is plain data, for which zero bits are
		// valid, and the engine wrote whole values over them.
		unsafe { trace.assume_init() }
	}

	/// Traces a line from `start` to `end` through the world alone, as the game's
	/// `UTIL_TraceLine` does with `CTraceFilterWorldOnly`: the line stops at the
	/// world's brushes whose contents are in `mask`, such as
	/// [`MASK_SOLID_BRUSHONLY`], and passes through every entity. Static props,
	/// which the engine traces with the entities, do not stop it either.
	#[doc(alias("TraceRay", "UTIL_TraceLine", "CTraceFilterWorldOnly"))]
	pub fn trace_world_line(self, start: Vector, end: Vector, mask: c_uint) -> WorldTrace {
		let mut filter = sys::ITraceFilter {
			vtable_: &raw const WORLD_ONLY_VTABLE,
		};

		// SAFETY: The filter is a local that outlives the call, and its methods
		// touch nothing.
		let trace = unsafe { self.trace_ray(start, end, mask, &raw mut filter) };

		WorldTrace {
			fraction: trace._base.fraction,
			end: trace._base.endpos.into(),
			start_solid: trace._base.startsolid,
			all_solid: trace._base.allsolid,
		}
	}
}
