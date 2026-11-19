//! `IEngineTrace`, which traces rays and queries what the world contains.
//!
//! [`EngineTrace::trace`] sweeps a [`Ray`], a line or a box, through what a
//! [`TraceFilter`] lets it hit, until it meets contents in a mask such as
//! [`MASK_PLAYERSOLID`]. [`EngineTrace::trace_line`] and
//! [`EngineTrace::trace_world_line`] are its common lines.
//! [`EngineTrace::clip_to_entity`] clips a ray to one entity and nothing else,
//! and [`EngineTrace::clip_to_static_prop`] to one static prop.

#[cfg(test)]
#[path = "../../tests/interfaces/engine_trace.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::static_props::StaticProp;
use crate::math::Vector;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int, c_short, c_uint, c_ushort};
use std::fmt::{self, Debug, Formatter};
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
	HITGROUP_CHEST, HITGROUP_GEAR, HITGROUP_GENERIC, HITGROUP_HEAD, HITGROUP_LEFTARM,
	HITGROUP_LEFTLEG, HITGROUP_RIGHTARM, HITGROUP_RIGHTLEG, HITGROUP_STOMACH,
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

/// A test of the entities a ray traced with [`TraceFilter::Matching`] may stop
/// at, which the engine runs only on those the ray reaches.
#[derive(Clone, Copy)]
pub struct EntityTest<'a, 'e>(&'a dyn Fn(Entity<'e>) -> bool);

impl<'a, 'e> EntityTest<'a, 'e> {
	/// A filter's test: whether a ray may stop at the entity it is given.
	pub const fn new(test: &'a dyn Fn(Entity<'e>) -> bool) -> Self {
		Self(test)
	}

	/// Whether a ray may stop at `entity`. A panic in the test stops the ray,
	/// as a filter that stops at every entity would, rather than unwind into
	/// the engine.
	fn passes(self, entity: Entity<'e>) -> bool {
		std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)(entity))).unwrap_or(true)
	}
}

impl Debug for EntityTest<'_, '_> {
	fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
		formatter.write_str("EntityTest(..)")
	}
}

impl PartialEq for EntityTest<'_, '_> {
	/// Whether both are the same test, by its address.
	fn eq(&self, other: &Self) -> bool {
		ptr::addr_eq(self.0, other.0)
	}
}

/// The filter the engine calls: the interface, followed by the
/// [`TraceFilter`] its methods ask.
#[repr(C)]
struct FilterInterface<'a, 'e> {
	interface: sys::ITraceFilter,
	filter: TraceFilter<'a, 'e>,
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

/// A line, or a box swept along one, for [`EngineTrace::trace`] to trace or
/// [`EngineTrace::clip_to_entity`] to clip to one entity, as `Ray_t::Init`
/// makes them.
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

/// Where a ray traced by [`EngineTrace::trace`], or clipped to one entity by
/// [`EngineTrace::clip_to_entity`], stopped, and what it hit.
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

	/// For a ray that [starts solid](Self::start_solid), how far along it,
	/// as [`Self::fraction`] measures, it leaves what it starts in
	/// (`fractionleftsolid`). The engine leaves 0 otherwise.
	#[doc(alias("fractionleftsolid"))]
	pub fraction_left_solid: f32,

	/// The normal of the surface the ray hit, pointing out of it, or the
	/// origin if it hit nothing.
	pub normal: Vector,

	/// The `CONTENTS_*` flags of what the ray hit.
	pub contents: c_int,

	/// The name of the surface the ray hit, as the engine fills it
	/// (`surface.name`): a brush's material, such as `TOOLS/TOOLSCLIP`, or a
	/// name of the engine's own for what has no material, such as a model's.
	/// `None` if the engine left none.
	#[doc(alias("csurface_t", "surface"))]
	pub surface_name: Option<&'s CStr>,

	/// The `SURF_*` flags of the brush surface the ray hit (`surface.flags`),
	/// such as `SURF_SKY`.
	pub surface_flags: c_ushort,

	/// The index of the properties of the surface the ray hit
	/// (`surface.surfaceProps`), such as what a footstep on it sounds like,
	/// which [`PhysicsSurfaceProps::surface`] gives: a brush's material's, or
	/// a model's own.
	///
	/// [`PhysicsSurfaceProps::surface`]: crate::interfaces::PhysicsSurfaceProps::surface
	#[doc(alias("surfaceProps"))]
	pub surface_props: c_short,

	/// The `DISPSURF_FLAG_*` flags of the displacement the ray hit
	/// (`dispFlags`), which the engine leaves 0 for anything else.
	#[doc(alias("dispFlags", "DISPSURF_FLAG"))]
	pub displacement_flags: c_ushort,

	/// The hitbox of a model the ray hit by its hitboxes, or, where
	/// [`Self::entity`] is the world's, 1 more than the index of the static
	/// prop the ray hit, and 0 for the world's brushes.
	pub hitbox: c_int,

	/// The hit group of the hitbox the ray hit, such as [`HITGROUP_HEAD`], as
	/// the model's hitbox set gives it, or [`HITGROUP_GENERIC`] for what it hit
	/// otherwise. The engine tests hitboxes only when the mask has
	/// [`CONTENTS_HITBOX`], as [`MASK_SHOT`] does, and the ray is a line.
	pub hitgroup: c_int,

	/// What the ray hit, or `None` if it hit nothing: an entity, or the
	/// world's own, at index 0, for the world's brushes and static props.
	pub entity: Option<Entity<'s>>,
}

impl<'s> Trace<'s> {
	/// The bit of [`Trace::displacement_flags`] that marks a displacement
	/// (`DISPSURF_FLAG_SURFACE`).
	const DISPLACEMENT_SURFACE: c_ushort = 1 << 0;

	/// What a trace the engine filled reports, for [`EngineTrace::trace`] and
	/// [`EngineTrace::clip_to_entity`] alike.
	///
	/// The ray met something if it stopped before its end, or starts or lies
	/// in something solid, and only then is an entity named: `clipped_to`,
	/// for a ray clipped to that entity alone, or else the one the engine's
	/// trace points at.
	///
	/// # Safety
	///
	/// The engine must have filled `raw` during `'s`, over zeroed memory.
	/// Without `clipped_to`, the entity a trace that met something points at
	/// must be null or stay allocated for `'s`.
	unsafe fn from_raw(raw: sys::trace_t, clipped_to: Option<Entity<'s>>) -> Self {
		let met = raw._base.fraction < 1.0 || raw._base.startsolid || raw._base.allsolid;

		// SAFETY: The caller vouches for the entity a trace that met something
		// points at, and this is asked of no other trace.
		let pointed_at =
			|| NonNull::new(raw.m_pEnt).map(|entity| unsafe { Entity::from_raw(entity) });

		Self {
			fraction: raw._base.fraction,
			end: raw._base.endpos.into(),
			start_solid: raw._base.startsolid,
			all_solid: raw._base.allsolid,
			fraction_left_solid: raw.fractionleftsolid,
			normal: raw._base.plane.normal.into(),
			contents: raw._base.contents,
			// SAFETY: A surface's name points at a string of the engine's or of
			// the level's, or of the game's where an entity that tests collisions
			// itself filled the trace. A module's string lasts while the module
			// is loaded, and the level's while the level does: by the first and
			// third conditions of `Server::new`, each for all of `'s`. A name
			// that was never set is null, as the trace started zeroed.
			surface_name: NonNull::new(raw.surface.name.cast_mut())
				.map(|name| unsafe { CStr::from_ptr(name.as_ptr()) }),
			surface_flags: raw.surface.flags,
			surface_props: raw.surface.surfaceProps,
			displacement_flags: raw._base.dispFlags,
			hitbox: raw.hitbox,
			hitgroup: raw.hitgroup,
			entity: if met {
				clipped_to.or_else(pointed_at)
			} else {
				None
			},
		}
	}

	/// A ray that reached its end and met nothing, where `end` is the ray's
	/// own: nothing solid, no surface and no entity, and zero for the rest.
	const fn miss(end: Vector) -> Self {
		Self {
			fraction: 1.0,
			end,
			start_solid: false,
			all_solid: false,
			fraction_left_solid: 0.0,
			normal: Vector::new(0.0, 0.0, 0.0),
			contents: 0,
			surface_name: None,
			surface_flags: 0,
			surface_props: 0,
			displacement_flags: 0,
			hitbox: 0,
			hitgroup: HITGROUP_GENERIC,
			entity: None,
		}
	}

	/// Whether the ray hit something before its end.
	pub fn hit(&self) -> bool {
		self.fraction < 1.0
	}

	/// Whether what the ray hit is a displacement, as its
	/// [flags](Self::displacement_flags) mark it (`DISPSURF_FLAG_SURFACE`).
	pub fn hit_displacement(&self) -> bool {
		self.displacement_flags & Self::DISPLACEMENT_SURFACE != 0
	}

	/// The index of the static prop the ray hit, among the level's static
	/// props, or `None` if it hit none: the engine reports a static prop as
	/// the world's [entity](Self::entity), with a [hitbox](Self::hitbox) 1
	/// more than its index.
	pub fn static_prop(&self) -> Option<u32> {
		let world = self.entity.and_then(Entity::index) == Some(0);

		u32::try_from(self.hitbox)
			.ok()
			.filter(|&hitbox| world && hitbox > 0)
			.map(|hitbox| hitbox - 1)
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

	/// These entities alone, passing through the world and static props, for
	/// when the world is traced apart.
	#[doc(alias("TRACE_ENTITIES_ONLY"))]
	OnlyEntities(&'a [Entity<'e>]),

	/// The world, static props and every entity but those on this team, by
	/// their `m_iTeamNum`, such as TF2's `TF_TEAM_RED`. Unlike TF2's own
	/// `CTraceFilterIgnoreTeammates`, which passes through the team's players
	/// and combat items alone, this passes through every entity of the team,
	/// such as its buildings, projectiles and team-coloured brushes.
	#[doc(alias("CTraceFilterIgnoreTeammates"))]
	SkipTeam(c_int),

	/// The world, static props and the entities a test passes, which the
	/// engine asks about only as the ray reaches them: for a choice too costly
	/// to make of every entity before the trace, such as by its model or
	/// collision rules.
	Matching(EntityTest<'a, 'e>),
}

impl<'e> TraceFilter<'_, 'e> {
	/// Whether a ray may stop at `entity`.
	fn hits(&self, entity: Entity<'e>) -> bool {
		let listed = |entities: &[Entity<'_>]| {
			entities
				.iter()
				.any(|listed| ptr::eq(listed.as_ptr(), entity.as_ptr()))
		};

		match *self {
			Self::Everything | Self::EntitiesOnly => true,
			Self::WorldOnly | Self::WorldAndProps => false,
			Self::Skip(entities) => !listed(entities),
			Self::Only(entities) | Self::OnlyEntities(entities) => listed(entities),
			Self::SkipTeam(team) => entity.team().ok() != Some(team),
			Self::Matching(test) => test.passes(entity),
		}
	}

	/// The kind of trace the engine runs for the filter.
	fn trace_type(&self) -> sys::TraceType_t {
		match self {
			Self::WorldOnly => sys::TraceType_t_TRACE_WORLD_ONLY,
			Self::EntitiesOnly | Self::OnlyEntities(_) => sys::TraceType_t_TRACE_ENTITIES_ONLY,
			_ => sys::TraceType_t_TRACE_EVERYTHING,
		}
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
	/// traces pass through it. An entity with no collideable overlaps nothing,
	/// as [`Self::clip_to_entity`] misses it.
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
		let ray = Ray::hull(position, position, mins, maxs);

		self.clip_ray_to_entity(ray, mask, entity)
			.is_some_and(|trace| trace._base.startsolid)
	}

	/// The trace the engine's `ClipRayToEntity` fills for `ray` and `entity`,
	/// or `None`, without a call to the engine, for an entity with no
	/// collideable: the engine reaches an entity's collision through its
	/// collideable, and is not known to check that it has one.
	fn clip_ray_to_entity(
		self,
		ray: Ray,
		mask: c_uint,
		entity: Entity<'_>,
	) -> Option<sys::trace_t> {
		entity.collideable()?;

		let ray = ray.to_raw();

		// `CBaseEntity`'s primary base derives from `IHandleEntity`, so the
		// pointers coincide.
		let handle = entity.as_ptr().cast::<sys::IHandleEntity>();
		let mut trace = MaybeUninit::<sys::trace_t>::zeroed();

		// SAFETY: `Server::new` guarantees the interface is live. The entity is
		// valid for the whole call: the lifetime of an `Entity` is a scope
		// `Server::new` vouches for, which this call lies within. By that
		// function's third condition the scope lies within one call from the
		// engine into the plugin, during which no frame completes, and by its
		// fourth nothing that runs in it, the engine's and the game's code this
		// call reaches included, frees an entity but by deferred deletion,
		// which frees it at the end of a frame. So the entity stays allocated
		// for the whole scope, even if it was removed earlier in it. Its
		// collideable was found above, and nothing has run since, so the
		// engine, which reaches the entity's collision through it, finds one.
		// The ray and the trace are locals that outlive the call, which is the
		// only time the engine uses them.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_ClipRayToEntity(&ray, mask, handle, trace.as_mut_ptr()))
		};

		// SAFETY: The trace started zeroed, and every field of it is plain data,
		// for which zero bits are valid, so it is initialised whatever the engine
		// wrote of it, which is whole values over those fields.
		Some(unsafe { trace.assume_init() })
	}

	/// Clips `ray` to `entity` alone, by the engine's `ClipRayToEntity`: how
	/// far a line, or a box swept along one, gets from the ray's start to its
	/// end before it meets the entity's collision model where its contents are
	/// in `mask`. Only the entity is tested, not the world, static props nor
	/// any other entity, so a ray that misses it reaches its end whatever else
	/// lies along it.
	///
	/// Nothing here decides whether the entity should stop the ray. The
	/// engine's function takes no [`TraceFilter`], and this tests neither the
	/// entity's [solid type](Entity::solid_type) nor its
	/// [solid flags](Entity::solid_flags): a trigger is clipped like any other
	/// entity, though [`Self::trace`] passes through it, as
	/// [`Self::box_overlaps_entity`] relies on. A caller that wants only what
	/// a trace would stop at tests the entity's solidity itself. As for that
	/// function, the engine asks the entity's own `TestCollision` instead of
	/// its model if the entity is flagged to test rays or boxes itself.
	///
	/// A ray that starts inside the entity
	/// [starts solid](Trace::start_solid), and is [all solid](Trace::all_solid)
	/// if it never leaves it. Either way it met the entity, whatever its
	/// [fraction](Trace::fraction): [`Trace::hit`], which goes by the fraction
	/// alone, can still be false.
	///
	/// The [entity](Trace::entity) of a ray that met `entity`, by stopping
	/// before its end or by starting or lying inside it, is `entity`, and
	/// `None` otherwise. It is taken from the argument, not from the engine's
	/// trace. The engine is given the entity as an `IHandleEntity` and has a
	/// `CBaseEntity` in its trace to point at what was hit (`m_pEnt`), but the
	/// interface does not say when a clip sets it, and the entity clipped is
	/// known: naming it gives the result the argument's lifetime without
	/// trusting a pointer read back from the engine.
	///
	/// An entity with no collideable is not passed to the engine, which is not
	/// known to check for one: the ray misses it, with a fraction of 1 at the
	/// ray's end.
	///
	/// Of a ray that misses an entity the engine was asked about, only the
	/// [fraction](Trace::fraction) of 1 is promised. The engine fills the
	/// rest, and its [end](Trace::end) is not known to be the ray's own to
	/// the last bit. [`Trace::fraction_left_solid`] is whatever the engine's
	/// clip leaves, which is not known to be what [`Self::trace`] reports of
	/// the same entity for a box.
	///
	/// Clipping the entities a ray could meet one by one, and keeping the
	/// nearest answer, is a way to trace through entities chosen beforehand
	/// without the engine's search for what lies along the ray, which
	/// [`Self::trace`] makes even with [`TraceFilter::OnlyEntities`]. The
	/// interface does not promise that a trace clips each entity as this does,
	/// nor say which entity a trace names where a ray starts inside one or
	/// meets several at one fraction, so compare the two before relying on
	/// their agreeing.
	#[doc(alias("ClipRayToEntity"))]
	pub fn clip_to_entity(self, ray: Ray, mask: c_uint, entity: Entity<'s>) -> Trace<'s> {
		let Some(trace) = self.clip_ray_to_entity(ray, mask, entity) else {
			return Trace::miss(ray.end);
		};

		// SAFETY: The engine filled the trace just now, within `'s`, over zeroed
		// memory, and the entity it names is the argument.
		unsafe { Trace::from_raw(trace, Some(entity)) }
	}

	/// Clips `ray` to the static prop `prop` alone, by the engine's
	/// `ClipRayToCollideable` given the prop's collideable: how far a line, or a
	/// box swept along one, gets from the ray's start to its end before it meets
	/// the prop's collision model where its contents are in `mask`. Only the prop
	/// is tested, not the world, any entity nor any other prop, so a ray that
	/// misses it reaches its end whatever else lies along it.
	///
	/// Nothing here decides whether the prop should stop the ray: a prop that is
	/// not solid is clipped like any other, though [`Self::trace`] passes
	/// through it, so a caller that wants only what a trace would stop at tests
	/// the prop's [solid type](StaticProp::solid_type) itself.
	///
	/// A ray that met the prop, by stopping before its end or by starting or
	/// lying inside it, reports it as a trace that stops at it does: its
	/// [entity](Trace::entity) is the world, and [`Trace::static_prop`] is the
	/// prop's index. Both are set here, from the prop, not read back from the
	/// engine, whose interface does not say what a clip fills them with. A ray
	/// that misses has no entity, and of it only the
	/// [fraction](Trace::fraction) of 1 is promised, as for
	/// [`Self::clip_to_entity`].
	///
	/// A trace through the world and the static props is not promised to clip
	/// each prop as this does, nor to hand a prop the whole ray: where the
	/// world stops the ray first, one was seen to hand the props the ray as far
	/// as the world let it go (see the
	/// [module documentation](crate::interfaces::static_props)). So clip a prop
	/// to the ray the world leaves, and compare against [`Self::trace`] before
	/// relying on the two agreeing.
	#[doc(alias("ClipRayToCollideable"))]
	pub fn clip_to_static_prop(self, ray: Ray, mask: c_uint, prop: StaticProp<'s>) -> Trace<'s> {
		let raw = ray.to_raw();
		let mut trace = MaybeUninit::<sys::trace_t>::zeroed();

		// SAFETY: `Server::new` guarantees the interface is live. The collideable
		// is the live prop's for `'s`, for the reason the documentation of
		// `StaticProp` gives: `'s` lies within one call into the plugin, during
		// which the level, and with it its props, last. The ray and the trace are
		// locals that outlive the call, which is the only time the engine uses
		// them.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_ClipRayToCollideable(&raw, mask, prop.collideable().as_ptr(), trace.as_mut_ptr()))
		};

		// SAFETY: As in `clip_ray_to_entity`, the zeroed trace is initialised
		// whatever the engine wrote of it.
		let mut trace = unsafe { trace.assume_init() };
		let met = trace._base.fraction < 1.0 || trace._base.startsolid || trace._base.allsolid;

		if !met {
			return Trace::miss(ray.end);
		}

		// A prop's index is below the length of a list the engine keeps as an
		// `int`, so 1 more than it is an `int` too.
		trace.hitbox =
			c_int::try_from(prop.index()).map_or(c_int::MAX, |index| index.saturating_add(1));

		// SAFETY: The engine filled the trace just now, within `'s`, over zeroed
		// memory, and the entity it names is the world, which lives for `'s`.
		unsafe { Trace::from_raw(trace, Some(prop.world())) }
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

		// SAFETY: The engine filled the trace just now, within `'s`, over zeroed
		// memory. It points a trace that hit something at the live entity it
		// hit, which stays allocated during `'s`, as entities are freed at the
		// end of a frame.
		unsafe { Trace::from_raw(trace, None) }
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
