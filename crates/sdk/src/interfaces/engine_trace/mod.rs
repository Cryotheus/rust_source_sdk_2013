//! `IEngineTrace`, which traces rays and queries what the world contains.
//!
//! [`EngineTrace::trace`] sweeps a [`Ray`], a line or a box, through what a
//! [`TraceFilter`] lets it hit, until it meets contents in a mask such as
//! [`MASK_PLAYERSOLID`]. [`EngineTrace::trace_line`] and
//! [`EngineTrace::trace_world_line`] are its common lines.

#[cfg(test)]
#[path = "../../tests/interfaces/engine_trace.rs"]
mod tests;

use crate::entities::Entity;
use crate::math::Vector;
use sdk_raw::vcall;
use std::ffi::{c_int, c_uint};
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};
use std::slice;

pub use sdk_raw::interfaces::engine_trace::{
	ALL_VISIBLE_CONTENTS, CONTENTS_AREAPORTAL, CONTENTS_AUX, CONTENTS_BLOCKLOS, CONTENTS_CURRENT_0,
	CONTENTS_CURRENT_90, CONTENTS_CURRENT_180, CONTENTS_CURRENT_270, CONTENTS_CURRENT_DOWN,
	CONTENTS_CURRENT_UP, CONTENTS_DEBRIS, CONTENTS_DETAIL, CONTENTS_EMPTY, CONTENTS_GRATE,
	CONTENTS_HITBOX, CONTENTS_IGNORE_NODRAW_OPAQUE, CONTENTS_LADDER, CONTENTS_MONSTER,
	CONTENTS_MONSTERCLIP, CONTENTS_MOVEABLE, CONTENTS_OPAQUE, CONTENTS_ORIGIN, CONTENTS_PLAYERCLIP,
	CONTENTS_SLIME, CONTENTS_SOLID, CONTENTS_TEAM1, CONTENTS_TEAM2, CONTENTS_TESTFOGVOLUME,
	CONTENTS_TRANSLUCENT, CONTENTS_UNUSED, CONTENTS_UNUSED6, CONTENTS_WATER, CONTENTS_WINDOW,
	LAST_VISIBLE_CONTENTS, MASK_ALL, MASK_BLOCKLOS, MASK_BLOCKLOS_AND_NPCS, MASK_CURRENT,
	MASK_DEADSOLID, MASK_NPCSOLID, MASK_NPCSOLID_BRUSHONLY, MASK_NPCWORLDSTATIC, MASK_OPAQUE,
	MASK_OPAQUE_AND_NPCS, MASK_PLAYERSOLID, MASK_PLAYERSOLID_BRUSHONLY, MASK_SHOT, MASK_SHOT_HULL,
	MASK_SHOT_PORTAL, MASK_SOLID, MASK_SOLID_BRUSHONLY, MASK_SPLITAREAPORTAL, MASK_VISIBLE,
	MASK_VISIBLE_AND_NPCS, MASK_WATER,
};

// `ITraceFilter` declares no destructor, so its two methods fill the vtable
// from its start under both ABIs, and `FILTER_VTABLE` is built as the
// generated vtable itself. A `FilterInterface` starts with the interface the
// engine calls.
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
	assert!(std::mem::offset_of!(FilterInterface<'_, '_>, interface) == 0);
};

/// The vtable of every [`FilterInterface`], whose methods ask its
/// [`TraceFilter`].
static FILTER_VTABLE: sys::ITraceFilter__bindgen_vtable = sys::ITraceFilter__bindgen_vtable {
	ITraceFilter_ShouldHitEntity: should_hit_entity,
	ITraceFilter_GetTraceType: trace_type,
};

/// The filter the engine calls: the interface, followed by the
/// [`TraceFilter`] its methods ask.
#[repr(C)]
struct FilterInterface<'a, 'e> {
	interface: sys::ITraceFilter,
	filter: TraceFilter<'a, 'e>,
}

/// A line, or a box swept along one, for [`EngineTrace::trace`] to trace, as
/// `Ray_t::Init` makes them.
#[doc(alias("Ray_t"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
	/// Where the ray starts: the line's start, or the point the box's
	/// `mins` and `maxs` are offsets from.
	pub start: Vector,

	/// Where the ray ends, as `start` does.
	pub end: Vector,

	/// The box's lowest corner, from its position, or the origin for a line.
	pub mins: Vector,

	/// The box's highest corner, from its position, or the origin for a line.
	pub maxs: Vector,
}

impl Ray {
	/// A box from `mins` to `maxs` around a position, swept from `start` to
	/// `end`, such as a player's hull sweeping from one of their origins to
	/// another. A box whose `start` and `end` are equal is tested where it is.
	pub const fn hull(start: Vector, end: Vector, mins: Vector, maxs: Vector) -> Self {
		Self {
			start,
			end,
			mins,
			maxs,
		}
	}

	/// A line from `start` to `end`.
	pub const fn line(start: Vector, end: Vector) -> Self {
		let origin = Vector::new(0.0, 0.0, 0.0);

		Self::hull(start, end, origin, origin)
	}

	/// The ray as `Ray_t::Init` makes it: starting at the box's center, from
	/// which the start offset leads back to the ray's start, and a ray if the
	/// box has no size.
	fn to_raw(self) -> sys::Ray_t {
		let aligned = |vector: Vector| sys::VectorAligned {
			_base: sys::Vector::from(vector),
		};
		let delta = *self.end - *self.start;
		let extents = (*self.maxs - *self.mins) * 0.5;
		let center = (*self.mins + *self.maxs) * 0.5;

		sys::Ray_t {
			m_Start: aligned(Vector(*self.start + center)),
			m_Delta: aligned(Vector(delta)),
			m_StartOffset: aligned(Vector(-center)),
			m_Extents: aligned(Vector(extents)),
			m_IsRay: extents.length_squared() < 1e-6,
			m_IsSwept: delta.length_squared() != 0.0,
		}
	}
}

/// Where a ray traced by [`EngineTrace::trace`] stopped, and what it hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trace<'s> {
	/// How far along the ray the trace got, from 0 at its start to 1 at its
	/// end, which it reached without hitting anything.
	pub fraction: f32,

	/// Where the trace stopped: for a box, its position there, the point
	/// [`Ray::mins`] and [`Ray::maxs`] are offsets from.
	pub end: Vector,

	/// Whether the ray starts inside something solid.
	pub start_solid: bool,

	/// Whether the whole ray lies inside something solid.
	pub all_solid: bool,

	/// The normal of the surface the ray hit, pointing out of it, or the
	/// origin if it hit nothing.
	pub normal: Vector,

	/// The `CONTENTS_*` flags of what the ray hit.
	pub contents: c_int,

	/// What the ray hit, or `None` if it hit nothing: an entity, or the
	/// world's own, at index 0, for the world's brushes and static props.
	pub entity: Option<Entity<'s>>,
}

impl Trace<'_> {
	/// Whether the ray hit something before its end.
	pub fn hit(&self) -> bool {
		self.fraction < 1.0
	}
}

/// Which entities a ray traced by [`EngineTrace::trace`] may stop at, as the
/// game's trace filters choose, without the game's own rules.
///
/// The engine stops a ray at an entity by the contents of its model, and
/// lets it pass through triggers and entities that are not solid. The game's
/// filters, such as `CTraceFilterSimple`, also pass through what the
/// entity they skip owns, such as its projectiles, and by collision groups,
/// such as debris. They also stop at an entity other than a brush only when
/// the mask has [`CONTENTS_MONSTER`], which the engine does not require.
/// None of that is done here.
///
/// The engine asks the filter about entities alone, never about static props,
/// which every filter but [`Self::WorldOnly`] and [`Self::EntitiesOnly`]
/// stops at, as the game's filters do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceFilter<'a, 'e> {
	/// The world, static props and every entity (`CTraceFilterHitAll`).
	#[doc(alias("CTraceFilterHitAll"))]
	Everything,

	/// The world's brushes alone (`CTraceFilterWorldOnly`).
	#[doc(alias("CTraceFilterWorldOnly", "TRACE_WORLD_ONLY"))]
	WorldOnly,

	/// The world's brushes and static props, but no entity
	/// (`CTraceFilterWorldAndPropsOnly`).
	#[doc(alias("CTraceFilterWorldAndPropsOnly"))]
	WorldAndProps,

	/// Entities alone, passing through the world and static props
	/// (`CTraceFilterEntitiesOnly`).
	#[doc(alias("CTraceFilterEntitiesOnly", "TRACE_ENTITIES_ONLY"))]
	EntitiesOnly,

	/// The world, static props and every entity but these, as the game's
	/// `CTraceFilterSimple` passing over an entity does, and
	/// `CTraceFilterSkipTwoEntities` and `CTraceFilterSimpleList` over more.
	#[doc(alias(
		"CTraceFilterSimple",
		"CTraceFilterSkipTwoEntities",
		"CTraceFilterSimpleList"
	))]
	Skip(&'a [Entity<'e>]),

	/// The world, static props and these entities alone.
	Only(&'a [Entity<'e>]),

	/// The world, static props and every entity but those on this team, by
	/// their `m_iTeamNum`, such as TF2's `TF_TEAM_RED`. Unlike TF2's own
	/// `CTraceFilterIgnoreTeammates`, which passes through the team's players
	/// and combat items alone, this passes through every entity of the team,
	/// such as its buildings, projectiles and team-coloured brushes.
	#[doc(alias("CTraceFilterIgnoreTeammates"))]
	SkipTeam(c_int),
}

impl TraceFilter<'_, '_> {
	/// Whether a ray may stop at `entity`.
	fn hits(&self, entity: Entity<'_>) -> bool {
		let listed = |entities: &[Entity<'_>]| {
			entities
				.iter()
				.any(|listed| ptr::eq(listed.as_ptr(), entity.as_ptr()))
		};

		match *self {
			Self::Everything | Self::EntitiesOnly => true,
			Self::WorldOnly | Self::WorldAndProps => false,
			Self::Skip(entities) => !listed(entities),
			Self::Only(entities) => listed(entities),
			Self::SkipTeam(team) => entity.team().ok() != Some(team),
		}
	}

	/// The kind of trace the engine runs for the filter.
	fn trace_type(&self) -> sys::TraceType_t {
		match self {
			Self::WorldOnly => sys::TraceType_t_TRACE_WORLD_ONLY,
			Self::EntitiesOnly => sys::TraceType_t_TRACE_ENTITIES_ONLY,
			_ => sys::TraceType_t_TRACE_EVERYTHING,
		}
	}
}

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

/// `ITraceFilter::ShouldHitEntity`, which asks the [`TraceFilter`] of a
/// [`FilterInterface`].
unsafe extern "C" fn should_hit_entity(
	filter: *mut sys::ITraceFilter,
	entity: *mut sys::IHandleEntity,
	_contents_mask: c_int,
) -> bool {
	// SAFETY: The engine passes back the filter `trace` gave it, a live
	// `FilterInterface`, whose interface is its first field. Unless the trace
	// type is `TRACE_EVERYTHING_FILTER_PROPS`, which no filter asks for, the
	// engine passes only entities, never static props, as
	// `public/engine/IEngineTrace.h` notes. An entity's `IHandleEntity` is the
	// first base of `IServerEntity`, its own first base, so it shares the
	// entity's address, and the entity is live during the trace.
	unsafe {
		let filter = &(*filter.cast::<FilterInterface<'_, '_>>()).filter;

		NonNull::new(entity.cast::<sys::CBaseEntity>())
			.is_some_and(|entity| filter.hits(Entity::from_raw(entity)))
	}
}

/// `ITraceFilter::GetTraceType`, which the [`TraceFilter`] of a
/// [`FilterInterface`] chooses.
unsafe extern "C" fn trace_type(filter: *const sys::ITraceFilter) -> sys::TraceType_t {
	// SAFETY: As for `should_hit_entity`.
	unsafe {
		(*filter.cast::<FilterInterface<'_, '_>>())
			.filter
			.trace_type()
	}
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
		// A box that does not move.
		let ray = Ray::hull(position, position, mins, maxs).to_raw();

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

	/// Traces `ray` from its start to its end through the world, static props
	/// and the entities `filter` lets it hit, as the game's `UTIL_TraceLine`,
	/// `UTIL_TraceHull` and `UTIL_TraceRay` do: the ray stops at the first
	/// whose contents are in `mask`, such as [`MASK_PLAYERSOLID`] for a
	/// player's hull, and passes through the rest. [`TraceFilter`] says which
	/// of the game's own rules are left out.
	///
	/// A box keeps its orientation, aligned with the world's axes, as players'
	/// and NextBot actors' hulls do.
	#[doc(alias("TraceRay", "UTIL_TraceLine", "UTIL_TraceHull", "UTIL_TraceRay"))]
	pub fn trace(self, ray: Ray, mask: c_uint, filter: TraceFilter<'_, '_>) -> Trace<'s> {
		let mut filter = FilterInterface {
			interface: sys::ITraceFilter {
				vtable_: &raw const FILTER_VTABLE,
			},
			filter,
		};

		let ray = ray.to_raw();
		let mut trace = MaybeUninit::<sys::trace_t>::zeroed();

		// SAFETY: `Server::new` guarantees the interface is live. The ray, the
		// filter and the trace are locals that outlive the call, which is the
		// only time the engine uses them, and the filter's methods only read the
		// filter and the entities the engine passes. The engine fills the trace,
		// which starts zeroed, as `CGameTrace`'s constructor leaves it.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_TraceRay(&ray, mask, (&raw mut filter).cast(), trace.as_mut_ptr()))
		};

		// SAFETY: Every field of the trace is plain data, for which zero bits are
		// valid, and the engine wrote whole values over them.
		let trace = unsafe { trace.assume_init() };
		let hit = trace._base.fraction < 1.0 || trace._base.startsolid || trace._base.allsolid;

		Trace {
			fraction: trace._base.fraction,
			end: trace._base.endpos.into(),
			start_solid: trace._base.startsolid,
			all_solid: trace._base.allsolid,
			normal: trace._base.plane.normal.into(),
			contents: trace._base.contents,
			// SAFETY: The engine points a trace that hit something at the live
			// entity it hit, which stays allocated during `'s`, as entities are
			// freed at the end of a frame.
			entity: NonNull::new(trace.m_pEnt)
				.filter(|_| hit)
				.map(|entity| unsafe { Entity::from_raw(entity) }),
		}
	}

	/// Traces a line from `start` to `end` through the world and its solid
	/// entities but `skip`, as the game's `UTIL_TraceLine` does with
	/// `CTraceFilterSimple` passing over an entity: the line stops at the first
	/// whose contents are in `mask`, such as [`MASK_SOLID`]. This is
	/// [`Self::trace`] with [`TraceFilter::Skip`], or
	/// [`TraceFilter::Everything`] without `skip`.
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
		let filter = skip.as_ref().map_or(TraceFilter::Everything, |skip| {
			TraceFilter::Skip(slice::from_ref(skip))
		});

		let trace = self.trace(Ray::line(start, end), mask, filter);

		LineTrace {
			fraction: trace.fraction,
			end: trace.end,
			start_solid: trace.start_solid,
			all_solid: trace.all_solid,
			entity: trace.entity,
		}
	}

	/// Traces a line from `start` to `end` through the world alone, as the game's
	/// `UTIL_TraceLine` does with `CTraceFilterWorldOnly`: the line stops at the
	/// world's brushes whose contents are in `mask`, such as
	/// [`MASK_SOLID_BRUSHONLY`], and passes through every entity. Static props,
	/// which the engine traces with the entities, do not stop it either. This is
	/// [`Self::trace`] with [`TraceFilter::WorldOnly`].
	#[doc(alias("TraceRay", "UTIL_TraceLine", "CTraceFilterWorldOnly"))]
	pub fn trace_world_line(self, start: Vector, end: Vector, mask: c_uint) -> WorldTrace {
		let trace = self.trace(Ray::line(start, end), mask, TraceFilter::WorldOnly);

		WorldTrace {
			fraction: trace.fraction,
			end: trace.end,
			start_solid: trace.start_solid,
			all_solid: trace.all_solid,
		}
	}
}
