//! Tests of the lines `EngineTrace` traces: the ray, mask and kind of trace
//! the engine receives, the entities the filters let a line hit, and what the
//! traces report.

use super::*;
use crate::test_support::entities::MockEntity;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;

thread_local! {
	/// The entities along every line the mock engine traces, nearest first.
	static ALONG: RefCell<Vec<*mut sys::IHandleEntity>> = const { RefCell::new(Vec::new()) };

	/// Every call to `TraceRay`, in order.
	static TRACED: RefCell<Vec<Traced>> = const { RefCell::new(Vec::new()) };
}

/// The arguments `TraceRay` received, with the kind of trace its filter asked
/// for.
#[derive(Debug, Clone, PartialEq)]
struct Traced {
	start: Vector,
	delta: Vector,
	is_ray: bool,
	swept: bool,
	mask: c_uint,
	trace_type: sys::TraceType_t,
}

/// A mock of the engine's tracer, which keeps its vtable alive.
struct MockEngineTrace {
	_vtable: Box<sys::IEngineTrace__bindgen_vtable>,
	interface: Box<sys::IEngineTrace>,
}

impl MockEngineTrace {
	/// Records `TraceRay`, which stops halfway along a line at the nearest of
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
					(&raw mut (*vtable).IEngineTrace_TraceRay).write(trace_ray);
				},
			)
		};
		let interface = Box::new(sys::IEngineTrace {
			vtable_: &raw const *vtable,
		});

		ALONG.set(along.to_vec());
		TRACED.take();

		Self {
			_vtable: vtable,
			interface,
		}
	}

	/// The mock, as the wrappers see it.
	fn engine_trace(&mut self) -> EngineTrace<'_> {
		// SAFETY: The mock outlives the borrow.
		unsafe { EngineTrace::from_raw(NonNull::from(&mut *self.interface)) }
	}
}

/// `IEngineTrace::TraceRay`: offers the filter the entities along the line,
/// unless it asks for the world alone, and stops halfway at the first it lets
/// the line hit, or reaches the line's end.
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
	let (start, delta) = (
		Vector::from(ray.m_Start._base),
		Vector::from(ray.m_Delta._base),
	);

	TRACED.with_borrow_mut(|traced| {
		traced.push(Traced {
			start,
			delta,
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
	trace._base.endpos = Vector(*start + *delta * fraction).into();
	trace.m_pEnt = hit.map_or(ptr::null_mut(), <*mut _>::cast);
}

/// A line from the origin along the X axis, 100 units long.
const LINE: (Vector, Vector) = (Vector::new(0.0, 0.0, 0.0), Vector::new(100.0, 0.0, 0.0));

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
