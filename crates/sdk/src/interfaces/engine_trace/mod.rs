//! `IEngineTrace`, which traces rays and queries what the world contains.

#[cfg(test)]
#[path = "../../tests/interfaces/engine_trace.rs"]
mod tests;

use crate::entities::Entity;
use crate::math::Vector;
use sdk_raw::vcall;
use std::ffi::{c_int, c_uint};
use std::mem::MaybeUninit;
use std::ptr;

// `ITraceFilter` declares no destructor, so its two methods fill the vtable
// from its start under both ABIs, and `WORLD_ONLY_VTABLE` is built as the
// generated vtable itself.
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
};

/// `CONTENTS_GRATE`: alpha-tested grates, which bullets and sight pass
/// through, but solids do not.
pub const CONTENTS_GRATE: c_uint = 0x8;

/// `CONTENTS_MOVEABLE`: brushes that move, such as doors.
pub const CONTENTS_MOVEABLE: c_uint = 0x4000;

/// `CONTENTS_SOLID`: solid brushes.
pub const CONTENTS_SOLID: c_uint = 0x1;

/// `CONTENTS_WINDOW`: translucent brushes, such as glass.
pub const CONTENTS_WINDOW: c_uint = 0x2;

/// `MASK_ALL`: every contents flag, so that a trace stops at whatever it
/// meets, the brushes of triggers included.
pub const MASK_ALL: c_uint = 0xFFFF_FFFF;

/// `MASK_SOLID_BRUSHONLY`: every brush a solid collides with.
pub const MASK_SOLID_BRUSHONLY: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_GRATE;

/// The vtable of the filter of [`EngineTrace::trace_world_line`], which hits
/// no entity and asks the engine to trace the world alone, as the game's
/// `CTraceFilterWorldOnly` does.
static WORLD_ONLY_VTABLE: sys::ITraceFilter__bindgen_vtable = sys::ITraceFilter__bindgen_vtable {
	ITraceFilter_ShouldHitEntity: hits_no_entity,
	ITraceFilter_GetTraceType: traces_world_only,
};

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

/// `CTraceFilterWorldOnly::ShouldHitEntity`.
unsafe extern "C" fn hits_no_entity(
	_filter: *mut sys::ITraceFilter,
	_entity: *mut sys::IHandleEntity,
	_contents_mask: c_int,
) -> bool {
	false
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
	/// Whether the box from `mins` to `maxs` around `position` overlaps
	/// `entity`'s collision model where its contents are in `mask`: whether
	/// the engine's `ClipRayToEntity`, with the box held still at `position`,
	/// starts solid. Only the entity is tested, not the world nor any other
	/// entity, and a trigger is tested like any other entity, though other
	/// traces pass through it.
	///
	/// With `mins` and `maxs` at the origin, the box is a point, and this is
	/// whether `position` lies within the entity, as the game's
	/// `CBaseTrigger::PointIsWithin` asks of a trigger with [`MASK_ALL`]. The
	/// engine asks the entity's own `TestCollision` instead if the entity is
	/// flagged to test points
	/// ([`SolidFlags::CUSTOM_RAY_TEST`](crate::entities::solid::SolidFlags::CUSTOM_RAY_TEST))
	/// or boxes (`CUSTOM_BOX_TEST`) itself.
	#[doc(alias("ClipRayToEntity", "PointIsWithin"))]
	pub fn box_overlaps_entity(
		self,
		entity: Entity<'_>,
		position: Vector,
		mins: Vector,
		maxs: Vector,
		mask: c_uint,
	) -> bool {
		let aligned = |vector: Vector| sys::VectorAligned {
			_base: sys::Vector::from(vector),
		};
		let extents = Vector((*maxs - *mins) * 0.5);
		let center = Vector((*mins + *maxs) * 0.5);

		// As `Ray_t::Init` makes a box that does not move: it starts at the box's
		// center, from which the start offset leads back to `position`, and is a
		// ray if the box has no size.
		let ray = sys::Ray_t {
			m_Start: aligned(Vector(*position + *center)),
			m_Delta: aligned(Vector::new(0.0, 0.0, 0.0)),
			m_StartOffset: aligned(Vector(-*center)),
			m_Extents: aligned(extents),
			m_IsRay: extents.length_squared() < 1e-6,
			m_IsSwept: false,
		};

		// `CBaseEntity`'s primary base derives from `IHandleEntity`, so the
		// pointers coincide.
		let handle = entity.as_ptr().cast::<sys::IHandleEntity>();
		let mut trace = MaybeUninit::<sys::trace_t>::zeroed();

		// SAFETY: `Server::new` guarantees the interface is live, and the entity
		// is live. The ray and the trace are locals that outlive the call, which
		// is the only time the engine uses them, and the engine fills the trace,
		// which starts zeroed, as `CGameTrace`'s constructor leaves it.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_ClipRayToEntity(&ray, mask, handle, trace.as_mut_ptr()))
		};

		// SAFETY: As for `trace_world_line`.
		unsafe { trace.assume_init() }._base.startsolid
	}

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

	/// Traces a line from `start` to `end` through the world alone, as the game's
	/// `UTIL_TraceLine` does with `CTraceFilterWorldOnly`: the line stops at the
	/// world's brushes whose contents are in `mask`, such as
	/// [`MASK_SOLID_BRUSHONLY`], and passes through every entity. Static props,
	/// which the engine traces with the entities, do not stop it either.
	#[doc(alias("TraceRay", "UTIL_TraceLine", "CTraceFilterWorldOnly"))]
	pub fn trace_world_line(self, start: Vector, end: Vector, mask: c_uint) -> WorldTrace {
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

		let mut filter = sys::ITraceFilter {
			vtable_: &raw const WORLD_ONLY_VTABLE,
		};

		let mut trace = MaybeUninit::<sys::trace_t>::zeroed();

		// SAFETY: `Server::new` guarantees the interface is live. The ray, the
		// filter and the trace are locals that outlive the call, which is the only
		// time the engine uses them. The filter's methods touch nothing, and the
		// engine fills the trace, which starts zeroed, as `CGameTrace`'s
		// constructor leaves it.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_TraceRay(&ray, mask, &mut filter, trace.as_mut_ptr()))
		};

		// SAFETY: Every field of the trace is plain data, for which zero bits are
		// valid, and the engine wrote whole values over them.
		let trace = unsafe { trace.assume_init() };

		WorldTrace {
			fraction: trace._base.fraction,
			end: trace._base.endpos.into(),
			start_solid: trace._base.startsolid,
			all_solid: trace._base.allsolid,
		}
	}
}
