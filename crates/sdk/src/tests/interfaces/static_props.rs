//! Tests of `StaticProps`: the list the engine fills is emptied before each
//! listing and checked after it, a second listing is refused while one lives,
//! an index past the last prop is refused without asking the engine, and a
//! ray clipped to a prop names the world and the prop's index, whatever the
//! engine filled in.

use super::*;
use crate::interfaces::engine_trace::{EngineTrace, MASK_SOLID, Ray};
use crate::test_support::entities::MockEntity;
use sdk_raw::interfaces::static_prop_manager::IStaticPropMgrServerVtable;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::c_uint;

/// Held by each test, as the list the engine fills is the process's.
static SERIAL: Mutex<()> = Mutex::new(());

thread_local! {
	/// The collideables the mock engine lists, in order.
	static PROPS: RefCell<Vec<*mut sys::ICollideable>> = const { RefCell::new(Vec::new()) };

	/// Whether the mock engine leaves its list longer than its memory.
	static MALFORM: Cell<bool> = const { Cell::new(false) };

	/// The indices `GetStaticPropByIndex` was asked for, in order.
	static ASKED: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };

	/// The collideables `ClipRayToCollideable` was given, in order.
	static CLIPPED: RefCell<Vec<*mut sys::ICollideable>> = const { RefCell::new(Vec::new()) };

	/// Whether the rays `ClipRayToCollideable` clips stop halfway.
	static CLIP_STOPS: Cell<bool> = const { Cell::new(false) };
}

/// A mock of the engine's traces that only clips to collideables, and keeps
/// its vtable alive.
struct MockEngineTrace {
	_vtable: Box<sys::IEngineTrace__bindgen_vtable>,
	interface: Box<sys::IEngineTrace>,
}

impl MockEngineTrace {
	/// Records `ClipRayToCollideable`, whose rays stop halfway as
	/// [`CLIP_STOPS`] says, with a hitbox of 0 and no entity, and fills every
	/// other slot with a stub that fails the test if called.
	fn new() -> Self {
		// SAFETY: As for `MockManager::new`.
		let vtable = unsafe {
			mock_vtable::<sys::IEngineTrace__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IEngineTrace_ClipRayToCollideable)
						.write(clip_ray_to_collideable);
				},
			)
		};
		let interface = Box::new(sys::IEngineTrace {
			vtable_: &raw const *vtable,
		});

		CLIPPED.take();
		CLIP_STOPS.set(false);

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

/// A mock of the engine's manager of static props, which keeps its vtable
/// alive.
struct MockManager {
	_vtable: Box<IStaticPropMgrServerVtable>,
	interface: Box<IStaticPropMgrServer>,
}

impl MockManager {
	/// Lists `props`, and finds the prop at an index as `GetStaticPropByIndex`,
	/// except that it finds nothing at index 1.
	fn new(props: &[*mut sys::ICollideable]) -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<IStaticPropMgrServerVtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).get_all_static_props).write(get_all_static_props);
				(&raw mut (*vtable).get_static_prop_by_index).write(get_static_prop_by_index);
			})
		};
		let interface = Box::new(IStaticPropMgrServer {
			vtable_: &raw const *vtable,
		});

		PROPS.set(props.to_vec());
		MALFORM.set(false);
		ASKED.take();

		Self {
			_vtable: vtable,
			interface,
		}
	}

	/// The mock, as the wrapper sees it.
	fn manager(&mut self) -> StaticPropManager<'_> {
		// SAFETY: The mock outlives the borrow.
		unsafe { StaticPropManager::from_raw(NonNull::from(&mut *self.interface)) }
	}
}

#[test]
fn a_clip_that_meets_a_prop_names_the_world_and_its_index() {
	let _serial = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut world = MockEntity::new(0);
	let mut mock = MockManager::new(&[collideable(1), collideable(2), collideable(3)]);
	let mut engine = MockEngineTrace::new();
	let world_entity = world.entity();
	let props = StaticProps::from_manager(mock.manager(), world_entity).unwrap();
	let prop = props.get(2).unwrap();

	CLIP_STOPS.set(true);

	let met = engine
		.engine_trace()
		.clip_to_static_prop(line(), MASK_SOLID, prop);

	assert_eq!(met.fraction, 0.5);
	assert_eq!(met.entity, Some(world_entity));
	assert_eq!(met.hitbox, 3);
	assert_eq!(met.static_prop(), Some(2));

	CLIP_STOPS.set(false);

	let missed = engine
		.engine_trace()
		.clip_to_static_prop(line(), MASK_SOLID, prop);

	assert_eq!(missed.fraction, 1.0);
	assert_eq!(missed.entity, None);
	assert_eq!(missed.static_prop(), None);
	assert_eq!(CLIPPED.take(), [collideable(3), collideable(3)]);
}

#[test]
fn a_list_longer_than_its_memory_is_refused_and_emptied() {
	let _serial = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut world = MockEntity::new(0);
	let mut mock = MockManager::new(&[collideable(1), collideable(2)]);

	MALFORM.set(true);

	assert!(matches!(
		StaticProps::from_manager(mock.manager(), world.entity()),
		Err(StaticPropsError::Malformed { .. })
	));

	MALFORM.set(false);

	let props = StaticProps::from_manager(mock.manager(), world.entity()).unwrap();

	assert_eq!(props.len(), 2);
}

#[test]
fn a_null_entry_is_no_prop() {
	let _serial = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut world = MockEntity::new(0);
	let mut mock = MockManager::new(&[collideable(1), ptr::null_mut()]);
	let props = StaticProps::from_manager(mock.manager(), world.entity()).unwrap();

	assert_eq!(props.len(), 2);
	assert_eq!(props.get(1), None);
}

#[test]
fn a_second_listing_is_refused_while_one_lives() {
	let _serial = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut world = MockEntity::new(0);
	let mut mock = MockManager::new(&[collideable(1)]);
	let manager = mock.manager();
	let first = StaticProps::from_manager(manager, world.entity()).unwrap();

	assert!(matches!(
		StaticProps::from_manager(manager, first.world),
		Err(StaticPropsError::Listed)
	));
	assert_eq!(first.len(), 1);
}

/// Records the collideable, and stops the ray halfway as [`CLIP_STOPS`] says.
unsafe extern "C" fn clip_ray_to_collideable(
	_this: *mut sys::IEngineTrace,
	ray: *const sys::Ray_t,
	_mask: c_uint,
	collideable: *mut sys::ICollideable,
	trace: *mut sys::trace_t,
) {
	CLIPPED.with_borrow_mut(|clipped| clipped.push(collideable));

	// SAFETY: The wrapper passes its ray and its zeroed trace.
	let (ray, trace) = unsafe { (&*ray, &mut *trace) };
	let fraction = if CLIP_STOPS.get() { 0.5 } else { 1.0 };
	let start = ray.m_Start._base;
	let delta = ray.m_Delta._base;

	trace._base.fraction = fraction;
	trace._base.endpos = sys::Vector {
		x: start.x + delta.x * fraction,
		y: start.y + delta.y * fraction,
		z: start.z + delta.z * fraction,
	};
}

/// A collideable the tests only compare, never reach.
fn collideable(n: usize) -> *mut sys::ICollideable {
	ptr::without_provenance_mut(0x1000 * n)
}

#[test]
fn each_listing_holds_its_own_props_alone() {
	let _serial = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut world = MockEntity::new(0);
	let mut mock = MockManager::new(&[collideable(1), collideable(2), collideable(3)]);

	{
		let props = StaticProps::from_manager(mock.manager(), world.entity()).unwrap();

		assert_eq!(props.len(), 3);
		assert_eq!(
			props
				.get(2)
				.map(|prop| (prop.collideable().as_ptr(), prop.index())),
			Some((collideable(3), 2))
		);
	}

	PROPS.set(vec![collideable(4)]);

	let props = StaticProps::from_manager(mock.manager(), world.entity()).unwrap();

	assert_eq!(props.len(), 1);
	assert_eq!(
		props.get(0).map(|prop| prop.collideable().as_ptr()),
		Some(collideable(4))
	);
	assert_eq!(props.get(1), None);
	assert_eq!(props.get(u32::MAX), None);
}

/// Adds [`PROPS`] to the end of `list`, growing its memory, which is leaked,
/// when it is too small, as the engine grows a list with its allocator.
unsafe extern "C" fn get_all_static_props(
	_this: *mut IStaticPropMgrServer,
	list: *mut CollideableList,
) {
	// SAFETY: The wrapper passes its list, which it holds locked.
	let list = unsafe { &mut *list };

	PROPS.with_borrow(|props| {
		let length = list.m_Size as usize + props.len();

		if length > list.m_Memory.m_nAllocationCount as usize {
			let mut grown = vec![ptr::null_mut(); length];

			for (slot, kept) in grown.iter_mut().enumerate().take(list.m_Size as usize) {
				// SAFETY: The slot is within the list's length.
				*kept = unsafe { list.m_Memory.m_pMemory.add(slot).read() };
			}

			list.m_Memory.m_pMemory = Box::leak(grown.into_boxed_slice()).as_mut_ptr();
			list.m_Memory.m_nAllocationCount = length as c_int;
		}

		for (slot, &prop) in (list.m_Size as usize..).zip(props) {
			// SAFETY: The slot is within the memory, grown above.
			unsafe { list.m_Memory.m_pMemory.add(slot).write(prop) };
		}

		list.m_Size = length as c_int;
		list.m_pElements = list.m_Memory.m_pMemory;

		if MALFORM.get() {
			list.m_Size = list.m_Memory.m_nAllocationCount + 1;
		}
	});
}

/// Records the index, and finds the listed prop there, but nothing at index
/// 1.
unsafe extern "C" fn get_static_prop_by_index(
	_this: *mut IStaticPropMgrServer,
	index: c_int,
) -> *mut sys::ICollideable {
	ASKED.with_borrow_mut(|asked| asked.push(index));

	if index == 1 {
		return ptr::null_mut();
	}

	PROPS.with_borrow(|props| props.get(index as usize).copied().unwrap_or_default())
}

/// A line from the origin along the X axis, 100 units long.
fn line() -> Ray {
	Ray::line(Vector::new(0.0, 0.0, 0.0), Vector::new(100.0, 0.0, 0.0))
}

#[test]
fn the_lookup_by_index_is_compared_and_not_asked_past_the_last() {
	let _serial = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut world = MockEntity::new(0);
	let mut mock = MockManager::new(&[collideable(1), collideable(2), collideable(3)]);
	let props = StaticProps::from_manager(mock.manager(), world.entity()).unwrap();

	assert_eq!(props.agrees(0), Some(true));
	assert_eq!(props.agrees(1), Some(false));
	assert_eq!(props.agrees(3), None);
	assert_eq!(ASKED.take(), [0, 1]);
}
