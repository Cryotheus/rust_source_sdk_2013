//! Complete cluster bounds and conservative expanded-origin visibility.

use super::*;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};

thread_local! {
	static COUNT: Cell<c_int> = const { Cell::new(4) };
	static ACTUAL: Cell<c_int> = const { Cell::new(4) };
	static BAD_BOUNDS: Cell<bool> = const { Cell::new(false) };
	static REQUESTED: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };
}

struct MockEngine {
	_vtable: Box<sys::IVEngineServer__bindgen_vtable>,
	interface: Box<sys::IVEngineServer>,
}

impl MockEngine {
	fn new() -> Self {
		COUNT.set(4);
		ACTUAL.set(4);
		BAD_BOUNDS.set(false);
		REQUESTED.take();
		// SAFETY: The mock consists only of function pointers. These patches
		// fill the generated signatures, and every unexpected call aborts.
		let vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_GetClusterCount).write(count);
					(&raw mut (*vtable).IVEngineServer_GetAllClusterBounds).write(bounds);
					(&raw mut (*vtable).IVEngineServer_GetPVSForCluster).write(pvs);
				},
			)
		};
		let interface = Box::new(sys::IVEngineServer {
			vtable_: &raw const *vtable,
		});
		Self {
			_vtable: vtable,
			interface,
		}
	}

	fn engine(&mut self) -> ValveEngine<'_> {
		// SAFETY: The mock and its vtable outlive the scoped wrapper.
		unsafe { ValveEngine::from_raw(NonNull::from(&mut *self.interface)) }
	}
}

unsafe extern "C" fn bounds(
	_engine: *mut sys::IVEngineServer,
	output: *mut sys::bbox_t,
	capacity: c_int,
) -> c_int {
	let boxes = [
		([-100.0, -100.0, -100.0], [0.0, 100.0, 100.0]),
		// A thin interior leaf crossed by eye +/- 8, containing neither the
		// eye nor any corner of that box.
		([2.0, -1.0, -1.0], [3.0, 1.0, 1.0]),
		([20.0, 20.0, 20.0], [30.0, 30.0, 30.0]),
		([40.0, 40.0, 40.0], [50.0, 50.0, 50.0]),
	];
	for (index, (mins, maxs)) in boxes.into_iter().take(capacity.max(0) as usize).enumerate() {
		let mins = if BAD_BOUNDS.get() && index == 1 {
			[f32::NAN; 3]
		} else {
			mins
		};
		let vector = |v: [f32; 3]| sys::Vector {
			x: v[0],
			y: v[1],
			z: v[2],
		};
		// SAFETY: The wrapper supplies capacity writable bbox_t entries.
		unsafe {
			output.add(index).write(sys::bbox_t {
				mins: vector(mins),
				maxs: vector(maxs),
			})
		};
	}
	ACTUAL.get()
}

unsafe extern "C" fn count(_engine: *mut sys::IVEngineServer) -> c_int {
	COUNT.get()
}

#[test]
fn expanded_origin_covers_an_interior_leaf_missed_by_center_and_corner_samples() {
	let mut mock = MockEngine::new();
	let engine = mock.engine();
	let bounds = engine.cluster_bounds().unwrap();
	let mins = Vector::new(-9.0, -8.0, -8.0);
	let maxs = Vector::new(7.0, 8.0, 8.0);
	let selected: Vec<_> = bounds
		.iter()
		.filter(|bounds| bounds.intersects(mins, maxs))
		.map(|bounds| bounds.cluster())
		.collect();
	assert_eq!(selected, [Cluster(0), Cluster(1)]);
	assert!(!bounds[1].intersects(Vector::new(-1.0, 0.0, 0.0), Vector::new(-1.0, 0.0, 0.0)));
	for x in [-9.0, 7.0] {
		for y in [-8.0, 8.0] {
			for z in [-8.0, 8.0] {
				let point = Vector::new(x, y, z);
				assert!(!bounds[1].intersects(point, point));
			}
		}
	}
	let combined = selected
		.into_iter()
		.map(|cluster| engine.pvs_for_cluster(cluster))
		.reduce(|mut pvs, next| {
			pvs.union_with(&next);
			pvs
		})
		.unwrap();
	assert!(combined.contains(Cluster(3)));
	assert_eq!(REQUESTED.take(), [0, 1]);
	assert_eq!(combined.as_bytes(), &[0b1011]);
}

#[test]
fn incomplete_or_invalid_native_bounds_are_rejected() {
	let mut mock = MockEngine::new();
	let engine = mock.engine();
	for count in [-1, PVS_CAPACITY as c_int * 8 + 1] {
		COUNT.set(count);
		assert_eq!(
			engine.cluster_bounds(),
			Err(ClusterBoundsError::InvalidCount(count))
		);
	}
	COUNT.set(4);
	ACTUAL.set(3);
	assert_eq!(
		engine.cluster_bounds(),
		Err(ClusterBoundsError::ChangedCount {
			expected: 4,
			actual: 3
		})
	);
	ACTUAL.set(4);
	BAD_BOUNDS.set(true);
	assert_eq!(
		engine.cluster_bounds(),
		Err(ClusterBoundsError::InvalidBounds(1))
	);
	BAD_BOUNDS.set(false);
	COUNT.set(0);
	assert_eq!(engine.cluster_bounds(), Ok(Vec::new()));
}

unsafe extern "C" fn pvs(
	_engine: *mut sys::IVEngineServer,
	cluster: c_int,
	capacity: c_int,
	output: *mut u8,
) -> c_int {
	assert!(capacity > 0);
	REQUESTED.with_borrow_mut(|requests| requests.push(cluster));
	let bits = match cluster {
		0 => 1,
		1 => 0b1010,
		2 => 4,
		3 => 8,
		_ => panic!("invalid cluster"),
	};
	// SAFETY: The wrapper provides at least one writable byte.
	unsafe { output.write(bits) };
	1
}
