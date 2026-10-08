//! Tests of `EngineTrace`: the boxes and points it clips to one entity, with
//! the ray the engine receives, as `Ray_t::Init` makes it, and the answer it
//! gives; and the lines and hulls it traces, with the ray, mask and kind of
//! trace the engine receives, the entities the filters let a ray hit, and
//! what the traces report.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};

thread_local! {
	/// The entities along every line the mock engine traces, nearest first.
	static ALONG: RefCell<Vec<*mut sys::IHandleEntity>> = const { RefCell::new(Vec::new()) };

	/// Every call to `ClipRayToEntity`, in order.
	static CLIPPED: RefCell<Vec<Clipped>> = const { RefCell::new(Vec::new()) };

	/// Whether the traces `ClipRayToEntity` fills start solid.
	static START_SOLID: Cell<bool> = const { Cell::new(false) };

	/// Every call to `TraceRay`, in order.
	static TRACED: RefCell<Vec<Traced>> = const { RefCell::new(Vec::new()) };
}

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

/// A mock of the engine's traces, which keeps its vtable alive.
struct MockEngineTrace {
	_vtable: Box<sys::IEngineTrace__bindgen_vtable>,
	interface: Box<sys::IEngineTrace>,
}

impl MockEngineTrace {
	/// Records `ClipRayToEntity`, whose traces start solid as [`START_SOLID`]
	/// says, and `TraceRay`, which stops halfway along a line at the nearest of
	/// the entities `along` it that the filter lets it hit, and fills every
	/// other slot with a stub that fails the test if called.
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

/// `IEngineTrace::ClipRayToEntity`, which records its arguments, and fills
/// the trace as starting solid if [`START_SOLID`] is set.
unsafe extern "C" fn clip_ray_to_entity(
	_this: *mut sys::IEngineTrace,
	ray: *const sys::Ray_t,
	mask: c_uint,
	entity: *mut sys::IHandleEntity,
	trace: *mut sys::trace_t,
) {
	// SAFETY: The wrapper passes a live ray, and a live trace to fill, which
	// are read and written as the engine does.
	let clipped = unsafe {
		let ray = &*ray;

		(&raw mut (*trace)._base.startsolid).write(START_SOLID.get());

		Clipped {
			start: ray.m_Start._base.into(),
			delta: ray.m_Delta._base.into(),
			start_offset: ray.m_StartOffset._base.into(),
			extents: ray.m_Extents._base.into(),
			is_ray: ray.m_IsRay,
			is_swept: ray.m_IsSwept,
			mask,
			entity,
		}
	};

	CLIPPED.with_borrow_mut(|calls| calls.push(clipped));
}

/// `IEngineTrace::TraceRay`: offers the filter the entities along the ray,
/// unless it asks for the world alone, and stops halfway at the first it lets
/// the ray hit, on a surface facing back along the X axis, or reaches the
/// ray's end. As the engine does, it moves the end by the ray's start offset,
/// back to where a box's position is.
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

	let fraction = if hit.is_some() { 0.5 } else { 1.0 };

	trace._base.fraction = fraction;
	trace._base.endpos = Vector(*start + *delta * fraction + *start_offset).into();

	if hit.is_some() {
		trace._base.plane.normal = Vector::new(-1.0, 0.0, 0.0).into();
		trace._base.contents = CONTENTS_MONSTER.cast_signed();
	}

	trace.m_pEnt = hit.map_or(ptr::null_mut(), <*mut _>::cast);
}

/// A line from the origin along the X axis, 100 units long.
const LINE: (Vector, Vector) = (Vector::new(0.0, 0.0, 0.0), Vector::new(100.0, 0.0, 0.0));

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
fn lines_skipping_nothing_hit_the_nearest_entity() {
	let (mut first, mut second) = (MockEntity::new(1), MockEntity::new(2));
	let mut mock = MockEngineTrace::new(&[first.as_ptr().cast(), second.as_ptr().cast()]);

	let trace = mock
		.engine_trace()
		.trace_line(LINE.0, LINE.1, MASK_SOLID, None);

	assert_eq!(trace.entity.map(Entity::as_ptr), Some(first.as_ptr()));
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
	assert_eq!(trace.normal, Vector::new(-1.0, 0.0, 0.0));
	assert_eq!(trace.contents, CONTENTS_MONSTER.cast_signed());
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
	assert_eq!(still.entity, None);
	TRACED.with_borrow(|traced| assert!(!traced[1].swept));
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
		stopped_at(engine, TraceFilter::WorldAndProps),
		(None, everything)
	);
	assert_eq!(
		stopped_at(engine, TraceFilter::WorldOnly),
		(None, sys::TraceType_t_TRACE_WORLD_ONLY)
	);
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
