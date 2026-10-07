//! Tests of the boxes and points `EngineTrace` clips to one entity: the ray
//! the engine receives, as `Ray_t::Init` makes it, and the answer it gives.

use super::*;
use crate::test_support::entities::MockEntity;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

thread_local! {
	/// Every call to `ClipRayToEntity`, in order.
	static CLIPPED: RefCell<Vec<Clipped>> = const { RefCell::new(Vec::new()) };

	/// Whether the traces `ClipRayToEntity` fills start solid.
	static START_SOLID: Cell<bool> = const { Cell::new(false) };
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

/// A mock of the engine's traces, which keeps its vtable alive.
struct MockEngineTrace {
	_vtable: Box<sys::IEngineTrace__bindgen_vtable>,
	interface: Box<sys::IEngineTrace>,
}

impl MockEngineTrace {
	/// Records `ClipRayToEntity`, whose traces start solid as [`START_SOLID`]
	/// says, and fills every other slot with a stub that fails the test if
	/// called.
	fn new() -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes a slot of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::IEngineTrace__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IEngineTrace_ClipRayToEntity).write(clip_ray_to_entity);
				},
			)
		};
		let interface = Box::new(sys::IEngineTrace {
			vtable_: &raw const *vtable,
		});

		CLIPPED.take();
		START_SOLID.set(false);

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

#[test]
fn boxes_start_at_their_center_and_lead_back_to_their_position() {
	let mut mock = MockEngineTrace::new();
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
	let mut mock = MockEngineTrace::new();
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
