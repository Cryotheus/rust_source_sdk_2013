//! Tests of `EngineTrace`: the boxes and points it clips to one entity, with
//! the ray the engine receives, as `Ray_t::Init` makes it, and the answer it
//! gives; the rays it clips to one entity, with the entity they name, and the
//! entities it keeps from the engine; and the lines and hulls it traces, with
//! the ray, mask and kind of trace the engine receives, the entities the
//! filters let a ray hit, and what the traces report.

use super::*;
use crate::test_support::entities::{MockEntity, remove_collideable, set_datamap, state_maps};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};

thread_local! {
	/// The entities along every line the mock engine traces, nearest first.
	static ALONG: RefCell<Vec<*mut sys::IHandleEntity>> = const { RefCell::new(Vec::new()) };

	/// Every call to `ClipRayToEntity`, in order.
	static CLIPPED: RefCell<Vec<Clipped>> = const { RefCell::new(Vec::new()) };

	/// Whether the rays `ClipRayToEntity` clips stop halfway, as [`fill`] has a
	/// hit, or reach their ends.
	static CLIP_STOPS: Cell<bool> = const { Cell::new(false) };

	/// Whether the traces `ClipRayToEntity` fills start solid.
	static START_SOLID: Cell<bool> = const { Cell::new(false) };

	/// Every call to `TraceRay`, in order.
	static TRACED: RefCell<Vec<Traced>> = const { RefCell::new(Vec::new()) };
}

/// A line from the origin along the X axis, 100 units long.
const LINE: (Vector, Vector) = (Vector::new(0.0, 0.0, 0.0), Vector::new(100.0, 0.0, 0.0));

/// The name of the surface the mock engine's hits are on.
const SURFACE: &CStr = c"**studio**";

/// The arguments `ClipRayToEntity` received.
#[derive(Debug, Clone, PartialEq)]
struct Clipped {
	start: Vector,
	delta: Vector,
	start_offset: Vector,
	extents: Vector,
	is_ray: bool,
	is_swept: bool,
	mask: c_uint,
	entity: *mut sys::IHandleEntity,
}

/// A mock of the engine's traces, which keeps its vtable alive.
struct MockEngineTrace {
	_vtable: Box<sys::IEngineTrace__bindgen_vtable>,
	interface: Box<sys::IEngineTrace>,
}

impl MockEngineTrace {
	/// Records `ClipRayToEntity`, whose rays stop halfway as [`CLIP_STOPS`] says
	/// and start solid as [`START_SOLID`] says, and `TraceRay`, which stops
	/// halfway along a line at the nearest of the entities `along` it that the
	/// filter lets it hit, and fills every other slot with a stub that fails the
	/// test if called.
	fn new(along: &[*mut sys::IHandleEntity]) -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::IEngineTrace__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IEngineTrace_ClipRayToEntity).write(clip_ray_to_entity);
					(&raw mut (*vtable).IEngineTrace_TraceRay).write(trace_ray);
				},
			)
		};
		let interface = Box::new(sys::IEngineTrace {
			vtable_: &raw const *vtable,
		});

		ALONG.set(along.to_vec());
		CLIPPED.take();
		CLIP_STOPS.set(false);
		START_SOLID.set(false);
		TRACED.take();

		Self {
			_vtable: vtable,
			interface,
		}
	}

	/// The mock, as the wrapper sees it.
	fn engine_trace(&mut self) -> EngineTrace<'_> {
		// SAFETY: The mock outlives the borrow.
		unsafe { EngineTrace::from_raw(NonNull::from(&mut *self.interface)) }
	}
}

/// The arguments `TraceRay` received, with the kind of trace its filter asked
/// for.
#[derive(Debug, Clone, PartialEq)]
struct Traced {
	start: Vector,
	delta: Vector,
	start_offset: Vector,
	extents: Vector,
	is_ray: bool,
	swept: bool,
	mask: c_uint,
	trace_type: sys::TraceType_t,
}

#[test]
fn boxes_start_at_their_center_and_lead_back_to_their_position() {
	let mut mock = MockEngineTrace::new(&[]);
	let mut room = MockEntity::new(7);
	let pointer = room.as_ptr().cast::<sys::IHandleEntity>();

	let overlaps = mock.engine_trace().box_overlaps_entity(
		room.entity(),
		Vector::new(10.0, 20.0, 30.0),
		Vector::new(-24.0, -24.0, 0.0),
		Vector::new(24.0, 24.0, 82.0),
		MASK_ALL,
	);

	assert!(!overlaps);

	CLIPPED.with_borrow(|clipped| {
		assert_eq!(
			*clipped,
			[Clipped {
				start: Vector::new(10.0, 20.0, 71.0),
				delta: Vector::new(0.0, 0.0, 0.0),
				start_offset: Vector::new(0.0, 0.0, -41.0),
				extents: Vector::new(24.0, 24.0, 41.0),
				is_ray: false,
				is_swept: false,
				mask: MASK_ALL,
				entity: pointer,
			}]
		);
	});
}

/// `IEngineTrace::ClipRayToEntity`, which records its arguments, and fills
/// the trace as [`fill`] does: stopped halfway if [`CLIP_STOPS`] is set, and
/// starting solid if [`START_SOLID`] is set. It names no entity.
unsafe extern "C" fn clip_ray_to_entity(
	_this: *mut sys::IEngineTrace,
	ray: *const sys::Ray_t,
	mask: c_uint,
	entity: *mut sys::IHandleEntity,
	trace: *mut sys::trace_t,
) {
	// SAFETY: The wrapper passes a live ray, and a live trace to fill, which
	// nothing else uses during the call.
	let (ray, trace) = unsafe { (&*ray, &mut *trace) };

	fill(trace, ray, CLIP_STOPS.get());
	trace._base.startsolid = START_SOLID.get();

	CLIPPED.with_borrow_mut(|calls| {
		calls.push(Clipped {
			start: ray.m_Start._base.into(),
			delta: ray.m_Delta._base.into(),
			start_offset: ray.m_StartOffset._base.into(),
			extents: ray.m_Extents._base.into(),
			is_ray: ray.m_IsRay,
			is_swept: ray.m_IsSwept,
			mask,
			entity,
		});
	});
}

#[test]
fn clips_name_the_entity_they_are_given() {
	let mut door = MockEntity::new(3);
	let entity = door.entity();
	let mut mock = MockEngineTrace::new(&[entity.as_ptr().cast()]);
	let engine = mock.engine_trace();
	let hull = Ray::hull(
		LINE.0,
		LINE.1,
		Vector::new(-24.0, -24.0, 0.0),
		Vector::new(24.0, 24.0, 82.0),
	);

	// A ray that misses the entity reaches its end.
	let missed = engine.clip_to_entity(hull, MASK_PLAYERSOLID, entity);

	assert_eq!(missed.fraction, 1.0);
	assert_eq!(missed.end, LINE.1);
	assert_eq!(missed.entity, None);

	// One that starts inside the entity and leaves it met it, though it
	// reaches its end too.
	START_SOLID.set(true);

	let inside = engine.clip_to_entity(hull, MASK_PLAYERSOLID, entity);

	assert!(inside.start_solid && !inside.hit());
	assert_eq!(inside.entity, Some(entity));

	// A hit reports what a trace's does, and is on the entity given, which
	// the mock engine's clips, unlike its traces, do not name.
	START_SOLID.set(false);
	CLIP_STOPS.set(true);

	let clipped = engine.clip_to_entity(hull, MASK_PLAYERSOLID, entity);

	assert_eq!(clipped.fraction, 0.5);
	assert_eq!(
		clipped,
		engine.trace(hull, MASK_PLAYERSOLID, TraceFilter::Everything)
	);
}

#[test]
fn entities_without_a_collideable_are_never_clipped() {
	let mut bare = MockEntity::new(5);
	let entity = bare.entity();
	let mut mock = MockEngineTrace::new(&[]);
	let engine = mock.engine_trace();
	let origin = Vector::new(0.0, 0.0, 0.0);

	// The mock engine would have every ray start inside the entity.
	START_SOLID.set(true);
	remove_collideable();

	let trace = engine.clip_to_entity(Ray::line(LINE.0, LINE.1), MASK_SOLID, entity);

	assert_eq!(trace.fraction, 1.0);
	assert_eq!(trace.end, LINE.1);
	assert!(!trace.start_solid);
	assert_eq!(trace.entity, None);
	assert!(!engine.box_overlaps_entity(entity, LINE.0, origin, origin, MASK_ALL));

	CLIPPED.with_borrow(|clipped| assert!(clipped.is_empty()));
}

/// Fills `trace` for `ray` as the mock engine does: stopped halfway, if
/// `hit`, on a surface facing back along the X axis, or at the ray's end.
/// As the engine does, it moves the end by the ray's start offset, back to
/// where a box's position is. A hit is on [`SURFACE`] of a displacement,
/// with hitbox 4 in the head's hit group, and the ray leaves what it starts
/// in a quarter of the way along. The entity is left to the caller.
fn fill(trace: &mut sys::trace_t, ray: &sys::Ray_t, hit: bool) {
	let fraction = if hit { 0.5 } else { 1.0 };
	let (start, delta, start_offset) = (
		Vector::from(ray.m_Start._base),
		Vector::from(ray.m_Delta._base),
		Vector::from(ray.m_StartOffset._base),
	);

	trace._base.fraction = fraction;
	trace._base.endpos = Vector(*start + *delta * fraction + *start_offset).into();

	if hit {
		trace._base.plane.normal = Vector::new(-1.0, 0.0, 0.0).into();
		trace._base.contents = CONTENTS_MONSTER.cast_signed();
		trace._base.dispFlags = 0b111;
		trace.fractionleftsolid = 0.25;
		trace.surface.name = SURFACE.as_ptr();
		trace.surface.flags = 0x4;
		trace.hitbox = 4;
		trace.hitgroup = HITGROUP_HEAD;
	}
}

#[test]
fn filters_choose_the_entities_a_ray_stops_at() {
	let mut mocks = [1, 2, 3].map(MockEntity::new);
	let [first, second, third] = mocks.each_mut().map(|entity| entity.entity());
	let along = [first, second, third].map(|entity| entity.as_ptr().cast());
	let mut mock = MockEngineTrace::new(&along);
	let engine = mock.engine_trace();
	let everything = sys::TraceType_t_TRACE_EVERYTHING;

	assert_eq!(
		stopped_at(engine, TraceFilter::Everything),
		(Some(first.as_ptr()), everything)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::EntitiesOnly),
		(Some(first.as_ptr()), sys::TraceType_t_TRACE_ENTITIES_ONLY)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::Skip(&[first, second])),
		(Some(third.as_ptr()), everything)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::Only(&[second])),
		(Some(second.as_ptr()), everything)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::Only(&[])),
		(None, everything)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::OnlyEntities(&[third])),
		(Some(third.as_ptr()), sys::TraceType_t_TRACE_ENTITIES_ONLY)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::WorldAndProps),
		(None, everything)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::WorldOnly),
		(None, sys::TraceType_t_TRACE_WORLD_ONLY)
	);
}

#[test]
fn hulls_start_at_their_center_and_end_at_their_position() {
	let mut entity = MockEntity::new(1);
	let mut mock = MockEngineTrace::new(&[entity.as_ptr().cast()]);
	let (mins, maxs) = (
		Vector::new(-24.0, -24.0, 0.0),
		Vector::new(24.0, 24.0, 82.0),
	);

	let trace = mock.engine_trace().trace(
		Ray::hull(LINE.0, LINE.1, mins, maxs),
		MASK_PLAYERSOLID,
		TraceFilter::Everything,
	);

	assert!(trace.hit());
	assert_eq!(trace.end, Vector::new(50.0, 0.0, 0.0));
	assert_eq!(trace.fraction_left_solid, 0.25);
	assert_eq!(trace.normal, Vector::new(-1.0, 0.0, 0.0));
	assert_eq!(trace.contents, CONTENTS_MONSTER.cast_signed());
	assert_eq!(trace.surface_name, Some(SURFACE));
	assert_eq!(trace.surface_flags, 0x4);
	assert_eq!(trace.displacement_flags, 0b111);
	assert_eq!(trace.hitbox, 4);
	assert_eq!(trace.hitgroup, HITGROUP_HEAD);
	assert_eq!(trace.entity.map(Entity::as_ptr), Some(entity.as_ptr()));

	TRACED.with_borrow(|traced| {
		assert_eq!(
			*traced,
			[Traced {
				start: Vector::new(0.0, 0.0, 41.0),
				delta: LINE.1,
				start_offset: Vector::new(0.0, 0.0, -41.0),
				extents: Vector::new(24.0, 24.0, 41.0),
				is_ray: false,
				swept: true,
				mask: MASK_PLAYERSOLID,
				trace_type: sys::TraceType_t_TRACE_EVERYTHING,
			}]
		);
	});

	// A hull that does not move is tested where it is.
	let still = mock.engine_trace().trace(
		Ray::hull(LINE.1, LINE.1, mins, maxs),
		MASK_PLAYERSOLID,
		TraceFilter::WorldAndProps,
	);

	assert!(!still.hit());
	assert_eq!(still.end, LINE.1);
	assert_eq!(still.normal, Vector::new(0.0, 0.0, 0.0));
	assert_eq!(still.surface_name, None);
	assert_eq!(still.displacement_flags, 0);
	assert_eq!(still.entity, None);
	TRACED.with_borrow(|traced| assert!(!traced[1].swept));
}

#[test]
fn lines_hit_the_nearest_entity_but_the_skipped_one() {
	let (mut skipped, mut other) = (MockEntity::new(1), MockEntity::new(2));
	let along = [skipped.as_ptr().cast(), other.as_ptr().cast()];
	let mut mock = MockEngineTrace::new(&along);

	let trace = mock
		.engine_trace()
		.trace_line(LINE.0, LINE.1, MASK_SOLID, Some(skipped.entity()));

	assert!(trace.hit());
	assert_eq!(trace.fraction, 0.5);
	assert_eq!(trace.end, Vector::new(50.0, 0.0, 0.0));
	assert_eq!(trace.entity.map(Entity::as_ptr), Some(other.as_ptr()));

	TRACED.with_borrow(|traced| {
		assert_eq!(
			*traced,
			[Traced {
				start: LINE.0,
				delta: LINE.1,
				start_offset: Vector::new(0.0, 0.0, 0.0),
				extents: Vector::new(0.0, 0.0, 0.0),
				is_ray: true,
				swept: true,
				mask: MASK_SOLID,
				trace_type: sys::TraceType_t_TRACE_EVERYTHING,
			}]
		);
	});
}

#[test]
fn lines_past_only_the_skipped_entity_hit_nothing() {
	let mut skipped = MockEntity::new(1);
	let mut mock = MockEngineTrace::new(&[skipped.as_ptr().cast()]);

	let trace = mock
		.engine_trace()
		.trace_line(LINE.0, LINE.1, MASK_SOLID, Some(skipped.entity()));

	assert!(!trace.hit());
	assert_eq!(trace.end, LINE.1);
	assert_eq!(trace.entity, None);
}

#[test]
fn lines_skipping_nothing_hit_the_nearest_entity() {
	let (mut first, mut second) = (MockEntity::new(1), MockEntity::new(2));
	let mut mock = MockEngineTrace::new(&[first.as_ptr().cast(), second.as_ptr().cast()]);

	let trace = mock
		.engine_trace()
		.trace_line(LINE.0, LINE.1, MASK_SOLID, None);

	assert_eq!(trace.entity.map(Entity::as_ptr), Some(first.as_ptr()));
}

#[test]
fn points_are_rays_that_do_not_move() {
	let mut mock = MockEngineTrace::new(&[]);
	let mut room = MockEntity::new(7);
	let pointer = room.as_ptr().cast::<sys::IHandleEntity>();
	let origin = Vector::new(0.0, 0.0, 0.0);

	START_SOLID.set(true);

	let within = mock.engine_trace().box_overlaps_entity(
		room.entity(),
		Vector::new(-1.5, 2.0, 64.0),
		origin,
		origin,
		CONTENTS_SOLID,
	);

	assert!(within);

	CLIPPED.with_borrow(|clipped| {
		assert_eq!(
			*clipped,
			[Clipped {
				start: Vector::new(-1.5, 2.0, 64.0),
				delta: origin,
				start_offset: origin,
				extents: origin,
				is_ray: true,
				is_swept: false,
				mask: CONTENTS_SOLID,
				entity: pointer,
			}]
		);
	});
}

#[test]
fn static_props_are_hits_on_the_world_with_a_hitbox() {
	let mut world = MockEntity::new(0);
	let mut prop = MockEntity::new(1);

	// The mock's hits have hitbox 4, on a displacement.
	let mut mock = MockEngineTrace::new(&[world.as_ptr().cast()]);
	let trace = mock.engine_trace().trace(
		Ray::line(LINE.0, LINE.1),
		MASK_SOLID,
		TraceFilter::Everything,
	);

	assert_eq!(trace.static_prop(), Some(3));
	assert!(trace.hit_displacement());

	// An entity's hitboxes are no static prop.
	let mut mock = MockEngineTrace::new(&[prop.as_ptr().cast()]);
	let trace = mock.engine_trace().trace(
		Ray::line(LINE.0, LINE.1),
		MASK_SOLID,
		TraceFilter::Everything,
	);

	assert_eq!(trace.static_prop(), None);

	// Nor is a miss.
	let mut mock = MockEngineTrace::new(&[]);
	let trace = mock.engine_trace().trace(
		Ray::line(LINE.0, LINE.1),
		MASK_SOLID,
		TraceFilter::Everything,
	);

	assert_eq!(trace.static_prop(), None);
	assert!(!trace.hit_displacement());
}

/// The entity a line along [`LINE`] traced through `filter` stopped at, and
/// the kind of trace the filter asked the engine for.
fn stopped_at(
	engine: EngineTrace<'_>,
	filter: TraceFilter<'_, '_>,
) -> (Option<*mut sys::CBaseEntity>, sys::TraceType_t) {
	let trace = engine.trace(Ray::line(LINE.0, LINE.1), MASK_SOLID, filter);
	let trace_type = TRACED.with_borrow(|traced| traced.last().unwrap().trace_type);

	(trace.entity.map(Entity::as_ptr), trace_type)
}

#[test]
fn team_filters_pass_through_the_teams_entities() {
	let mut mocks = [1, 2, 3].map(MockEntity::new);

	// Every mock entity reports this datamap chain, which declares
	// `m_iTeamNum`.
	set_datamap(state_maps(Vec::new()));

	for (mock, team) in mocks.iter_mut().zip([2, 2, 3]) {
		mock.state().team = team;
	}

	let [red, red_too, blue] = mocks.each_mut().map(|entity| entity.entity());
	let along = [red, red_too, blue].map(|entity| entity.as_ptr().cast());
	let mut mock = MockEngineTrace::new(&along);
	let engine = mock.engine_trace();

	assert_eq!(
		stopped_at(engine, TraceFilter::SkipTeam(2)).0,
		Some(blue.as_ptr())
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::SkipTeam(3)).0,
		Some(red.as_ptr())
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::SkipTeam(0)).0,
		Some(red.as_ptr())
	);
}

/// `IEngineTrace::TraceRay`: offers the filter the entities along the ray,
/// unless it asks for the world alone, and fills the trace as [`fill`] does,
/// stopped halfway at the first entity the filter lets the ray hit, which it
/// names, or at the ray's end.
unsafe extern "C" fn trace_ray(
	_this: *mut sys::IEngineTrace,
	ray: *const sys::Ray_t,
	mask: c_uint,
	filter: *mut sys::ITraceFilter,
	trace: *mut sys::trace_t,
) {
	// SAFETY: The wrappers pass a live ray, filter and trace, which nothing
	// else uses during the call.
	let (ray, methods, trace) = unsafe { (&*ray, &*(*filter).vtable_, &mut *trace) };

	// SAFETY: The filter is the wrapper's own, live during the call.
	let trace_type = unsafe { (methods.ITraceFilter_GetTraceType)(filter) };
	let (start, delta, start_offset) = (
		Vector::from(ray.m_Start._base),
		Vector::from(ray.m_Delta._base),
		Vector::from(ray.m_StartOffset._base),
	);

	TRACED.with_borrow_mut(|traced| {
		traced.push(Traced {
			start,
			delta,
			start_offset,
			extents: ray.m_Extents._base.into(),
			is_ray: ray.m_IsRay,
			swept: ray.m_IsSwept,
			mask,
			trace_type,
		});
	});

	let hit = ALONG
		.with_borrow(Vec::clone)
		.into_iter()
		.filter(|_| trace_type != sys::TraceType_t_TRACE_WORLD_ONLY)
		// SAFETY: As for the trace type. The filters only compare the entities.
		.find(|&entity| unsafe {
			(methods.ITraceFilter_ShouldHitEntity)(filter, entity, mask.cast_signed())
		});

	fill(trace, ray, hit.is_some());
	trace.m_pEnt = hit.map_or(ptr::null_mut(), <*mut _>::cast);
}

#[test]
fn world_lines_pass_through_every_entity() {
	let mut entity = MockEntity::new(1);
	let mut mock = MockEngineTrace::new(&[entity.as_ptr().cast()]);

	let trace = mock
		.engine_trace()
		.trace_world_line(LINE.0, LINE.1, MASK_SOLID_BRUSHONLY);

	assert!(!trace.hit());
	assert_eq!(trace.end, LINE.1);

	TRACED.with_borrow(|traced| {
		assert_eq!(traced.len(), 1);
		assert_eq!(traced[0].mask, MASK_SOLID_BRUSHONLY);
		assert_eq!(traced[0].trace_type, sys::TraceType_t_TRACE_WORLD_ONLY);
	});
}
