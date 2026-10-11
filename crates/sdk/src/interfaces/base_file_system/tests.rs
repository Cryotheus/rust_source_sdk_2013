use super::{BaseFileSystem, ReadError, ReadLimits};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_void};
use std::ptr::NonNull;

#[repr(C)]
struct Mock {
	raw: sys::IBaseFileSystem,
	state: RefCell<State>,
}

struct State {
	payload: Vec<u8>,
	position: usize,
	size: u32,
	open: bool,
	replies: VecDeque<i32>,
	requests: Vec<usize>,
	opens: usize,
	sizes: usize,
	path_sizes: usize,
	closes: usize,
	path: Option<CString>,
	options: Option<CString>,
	path_id: Option<CString>,
	bad_handle: bool,
	initialized: bool,
}

impl Mock {
	fn new(payload: &[u8]) -> Box<Self> {
		Box::new(Self {
			raw: sys::IBaseFileSystem { vtable_: &VTABLE },
			state: RefCell::new(State {
				payload: payload.to_vec(),
				position: 0,
				size: payload.len() as u32,
				open: true,
				replies: VecDeque::new(),
				requests: Vec::new(),
				opens: 0,
				sizes: 0,
				path_sizes: 0,
				closes: 0,
				path: None,
				options: None,
				path_id: None,
				bad_handle: false,
				initialized: true,
			}),
		})
	}

	fn fs(&self) -> BaseFileSystem<'_> {
		// SAFETY: The boxed repr(C) fake and every valid vtable entry stay live
		// for this borrow. Tests run the wrapper synchronously on this thread.
		unsafe { BaseFileSystem::from_raw(NonNull::from(&self.raw)) }
	}

	fn closed_once(&self) {
		let state = self.state.borrow();
		assert_eq!(state.opens, 1);
		assert_eq!(state.sizes, 1);
		assert_eq!(state.path_sizes, 0);
		assert_eq!(state.closes, 1);
		assert!(!state.bad_handle);
		assert!(state.initialized);
	}
}

unsafe fn mock<'a>(this: *mut sys::IBaseFileSystem) -> &'a Mock {
	// SAFETY: All test calls originate from the live repr(C) Mock's first field.
	unsafe { &*this.cast::<Mock>() }
}

unsafe extern "C" fn open(
	this: *mut sys::IBaseFileSystem,
	path: *const c_char,
	options: *const c_char,
	path_id: *const c_char,
) -> sys::FileHandle_t {
	let mut state = unsafe { mock(this) }.state.borrow_mut();
	state.opens += 1;
	state.path = Some(unsafe { CStr::from_ptr(path) }.to_owned());
	state.options = Some(unsafe { CStr::from_ptr(options) }.to_owned());
	state.path_id = if path_id.is_null() {
		None
	} else {
		Some(unsafe { CStr::from_ptr(path_id) }.to_owned())
	};
	if state.open {
		this.cast()
	} else {
		std::ptr::null_mut()
	}
}

unsafe extern "C" fn size(this: *mut sys::IBaseFileSystem, handle: sys::FileHandle_t) -> c_uint {
	let mut state = unsafe { mock(this) }.state.borrow_mut();
	state.sizes += 1;
	state.bad_handle |= handle != this.cast();
	state.size
}

unsafe extern "C" fn path_size(
	this: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: *const c_char,
) -> c_uint {
	unsafe { mock(this) }.state.borrow_mut().path_sizes += 1;
	u32::MAX
}

unsafe extern "C" fn close(this: *mut sys::IBaseFileSystem, handle: sys::FileHandle_t) {
	let mut state = unsafe { mock(this) }.state.borrow_mut();
	state.closes += 1;
	state.bad_handle |= handle != this.cast();
}

unsafe extern "C" fn read(
	this: *mut sys::IBaseFileSystem,
	destination: *mut c_void,
	requested: c_int,
	handle: sys::FileHandle_t,
) -> c_int {
	let mut state = unsafe { mock(this) }.state.borrow_mut();
	state.bad_handle |= handle != this.cast();
	let requested = requested as usize;
	state.requests.push(requested);
	let initialized = unsafe { std::slice::from_raw_parts(destination.cast::<u8>(), requested) };
	state.initialized &= initialized.iter().all(|&byte| byte == 0);
	let remaining = state.payload.len() - state.position;
	let returned = state
		.replies
		.pop_front()
		.unwrap_or(requested.min(remaining) as i32);
	if returned > 0 && returned as usize <= requested {
		let copied = (returned as usize).min(remaining);
		unsafe {
			std::ptr::copy_nonoverlapping(
				state.payload.as_ptr().add(state.position),
				destination.cast(),
				copied,
			);
		}
		state.position += copied;
	}
	returned
}

unsafe extern "C" fn write(
	_: *mut sys::IBaseFileSystem,
	_: *const c_void,
	_: c_int,
	_: sys::FileHandle_t,
) -> c_int {
	0
}
unsafe extern "C" fn seek(
	_: *mut sys::IBaseFileSystem,
	_: sys::FileHandle_t,
	_: c_int,
	_: sys::FileSystemSeek_t,
) {
}
unsafe extern "C" fn tell(_: *mut sys::IBaseFileSystem, _: sys::FileHandle_t) -> c_uint {
	0
}
unsafe extern "C" fn flush(_: *mut sys::IBaseFileSystem, _: sys::FileHandle_t) {}
unsafe extern "C" fn query(
	_: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: *const c_char,
) -> bool {
	false
}
unsafe extern "C" fn writable(
	_: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: bool,
	_: *const c_char,
) -> bool {
	false
}
unsafe extern "C" fn file_time(
	_: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: *const c_char,
) -> c_long {
	0
}
unsafe extern "C" fn read_file(
	_: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: *const c_char,
	_: *mut sys::CUtlBuffer,
	_: c_int,
	_: c_int,
	_: sys::FSAllocFunc_t,
) -> bool {
	false
}
unsafe extern "C" fn write_file(
	_: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: *const c_char,
	_: *mut sys::CUtlBuffer,
) -> bool {
	false
}
unsafe extern "C" fn unzip(
	_: *mut sys::IBaseFileSystem,
	_: *const c_char,
	_: *const c_char,
	_: *const c_char,
) -> bool {
	false
}

const VTABLE: sys::IBaseFileSystem__bindgen_vtable = sys::IBaseFileSystem__bindgen_vtable {
	IBaseFileSystem_Read: read,
	IBaseFileSystem_Write: write,
	IBaseFileSystem_Open: open,
	IBaseFileSystem_Close: close,
	IBaseFileSystem_Seek: seek,
	IBaseFileSystem_Tell: tell,
	IBaseFileSystem_Size: size,
	IBaseFileSystem_Size1: path_size,
	IBaseFileSystem_Flush: flush,
	IBaseFileSystem_Precache: query,
	IBaseFileSystem_FileExists: query,
	IBaseFileSystem_IsFileWritable: query,
	IBaseFileSystem_SetFileWritable: writable,
	IBaseFileSystem_GetFileTime: file_time,
	IBaseFileSystem_ReadFile: read_file,
	IBaseFileSystem_WriteFile: write_file,
	IBaseFileSystem_UnzipFile: unzip,
};

#[test]
fn accessor_uses_exact_engine_export() {
	let mock = Mock::new(b"map");
	crate::test_support::server::export(
		crate::Module::Engine,
		c"VBaseFileSystem011",
		std::ptr::from_ref(&mock.raw).cast_mut(),
	);
	let server = crate::test_support::server::mock_server(&mock);
	let fs = server.base_file_system().unwrap();
	assert_eq!(fs.as_ptr(), std::ptr::from_ref(&mock.raw).cast_mut());
	assert_eq!(
		fs.read_file(c"maps/example.bsp", None, ReadLimits::default())
			.unwrap(),
		b"map"
	);
	mock.closed_once();
}

#[test]
fn missing_export_is_an_interface_error() {
	let scope = ();
	let server = crate::test_support::server::null_server(crate::Game::TeamFortress2, &scope);
	let error = server.base_file_system().unwrap_err();
	assert_eq!(error.module(), crate::Module::Engine);
	assert_eq!(error.version(), c"VBaseFileSystem011");
}

#[test]
fn owned_bytes_leave_the_callback_and_keep_exact_arguments() {
	let bytes = {
		let mock = Mock::new(b"abc");
		let bytes = mock
			.fs()
			.read_file(c"maps/exact.bsp", Some(c"GAME"), ReadLimits::default())
			.unwrap();
		{
			let state = mock.state.borrow();
			assert_eq!(state.path.as_deref(), Some(c"maps/exact.bsp"));
			assert_eq!(state.options.as_deref(), Some(c"rb"));
			assert_eq!(state.path_id.as_deref(), Some(c"GAME"));
		}
		mock.closed_once();
		bytes
	};
	fn require_send<T: Send>(_: &T) {}
	require_send(&bytes);
	assert_eq!(bytes, b"abc");
}

#[test]
fn null_open_never_sizes_reads_or_closes() {
	let mock = Mock::new(b"abc");
	mock.state.borrow_mut().open = false;
	assert_eq!(
		mock.fs().read_file(c"missing", None, ReadLimits::default()),
		Err(ReadError::Open)
	);
	let state = mock.state.borrow();
	assert_eq!(state.opens, 1);
	assert_eq!(state.sizes, 0);
	assert!(state.requests.is_empty());
	assert_eq!(state.closes, 0);
	assert_eq!(state.path_id, None);
}

#[test]
fn an_open_empty_file_needs_no_read_budget() {
	let mock = Mock::new(b"");
	assert_eq!(
		mock.fs()
			.read_file(
				c"empty",
				None,
				ReadLimits {
					bytes: 0,
					read_calls: 0
				}
			)
			.unwrap(),
		Vec::<u8>::new()
	);
	assert!(mock.state.borrow().requests.is_empty());
	mock.closed_once();
}

#[test]
fn byte_budget_and_native_integer_cap_close_before_reading() {
	let mock = Mock::new(b"abc");
	assert_eq!(
		mock.fs().read_file(
			c"large",
			None,
			ReadLimits {
				bytes: 2,
				read_calls: 1
			}
		),
		Err(ReadError::ByteLimit { bytes: 3, limit: 2 })
	);
	assert!(mock.state.borrow().requests.is_empty());
	mock.closed_once();
	let mock = Mock::new(b"");
	mock.state.borrow_mut().size = u32::MAX;
	assert_eq!(
		mock.fs().read_file(
			c"large",
			None,
			ReadLimits {
				bytes: usize::MAX,
				read_calls: usize::MAX
			}
		),
		Err(ReadError::ByteLimit {
			bytes: u32::MAX as usize,
			limit: i32::MAX as usize
		})
	);
	assert!(mock.state.borrow().requests.is_empty());
	mock.closed_once();
}

#[test]
fn allocation_failure_closes_and_returns_no_partial_bytes() {
	let mock = Mock::new(b"abc");
	let result =
		mock.fs()
			.read_file_with_allocator(c"allocation", None, ReadLimits::default(), |bytes| {
				Err(ReadError::Allocation { bytes })
			});
	assert_eq!(result, Err(ReadError::Allocation { bytes: 3 }));
	assert!(mock.state.borrow().requests.is_empty());
	mock.closed_once();
}

#[test]
fn guard_closes_during_a_rust_unwind() {
	let mock = Mock::new(b"abc");
	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		mock.fs()
			.read_file_with_allocator(c"allocation", None, ReadLimits::default(), |_| {
				panic!("allocation fixture")
			})
	}));
	assert!(result.is_err());
	mock.closed_once();
}

#[test]
fn positive_short_reads_continue_with_initialized_remaining_storage() {
	let mock = Mock::new(b"abcdef");
	mock.state.borrow_mut().replies = [1, 2, 3].into();
	assert_eq!(
		mock.fs()
			.read_file(
				c"short",
				None,
				ReadLimits {
					bytes: 6,
					read_calls: 3
				}
			)
			.unwrap(),
		b"abcdef"
	);
	assert_eq!(mock.state.borrow().requests, [6, 5, 3]);
	mock.closed_once();
}

#[test]
fn chunks_and_read_calls_are_bounded() {
	let mock = Mock::new(&vec![7; 64 * 1024 + 1]);
	assert_eq!(
		mock.fs()
			.read_file(c"chunk", None, ReadLimits::default())
			.unwrap()
			.len(),
		64 * 1024 + 1
	);
	assert_eq!(mock.state.borrow().requests, [64 * 1024, 1]);
	mock.closed_once();
	let mock = Mock::new(b"abc");
	mock.state.borrow_mut().replies = [1, 1, 1].into();
	assert_eq!(
		mock.fs().read_file(
			c"budget",
			None,
			ReadLimits {
				bytes: 3,
				read_calls: 1
			}
		),
		Err(ReadError::ReadLimit {
			limit: 1,
			completed: 1
		})
	);
	assert_eq!(mock.state.borrow().requests, [3]);
	mock.closed_once();
	let mock = Mock::new(b"abc");
	assert_eq!(
		mock.fs().read_file(
			c"budget",
			None,
			ReadLimits {
				bytes: 3,
				read_calls: 0
			}
		),
		Err(ReadError::ReadLimit {
			limit: 0,
			completed: 0
		})
	);
	assert!(mock.state.borrow().requests.is_empty());
	mock.closed_once();
}

#[test]
fn eof_and_negative_reads_discard_an_already_read_prefix() {
	let mock = Mock::new(b"abc");
	mock.state.borrow_mut().replies = [1, 0].into();
	assert_eq!(
		mock.fs().read_file(c"eof", None, ReadLimits::default()),
		Err(ReadError::UnexpectedEof {
			expected: 3,
			completed: 1
		})
	);
	mock.closed_once();
	let mock = Mock::new(b"abc");
	mock.state.borrow_mut().replies = [1, -7].into();
	assert_eq!(
		mock.fs().read_file(c"failed", None, ReadLimits::default()),
		Err(ReadError::ReadFailed {
			returned: -7,
			completed: 1
		})
	);
	mock.closed_once();
}

#[test]
fn impossible_positive_count_is_rejected() {
	let mock = Mock::new(b"abc");
	mock.state.borrow_mut().replies = [4].into();
	assert_eq!(
		mock.fs().read_file(c"count", None, ReadLimits::default()),
		Err(ReadError::InvalidReadCount {
			requested: 3,
			returned: 4
		})
	);
	mock.closed_once();
}

#[test]
fn sampled_size_reads_only_a_prefix_and_makes_no_atomicity_claim() {
	let mock = Mock::new(b"abcdef");
	mock.state.borrow_mut().size = 3;
	assert_eq!(
		mock.fs()
			.read_file(c"growing", None, ReadLimits::default())
			.unwrap(),
		b"abc"
	);
	assert_eq!(mock.state.borrow().position, 3);
	mock.closed_once();
}

struct FactoryState {
	supplied: *mut c_void,
	other: *mut c_void,
	versions: Vec<CString>,
	other_calls: usize,
}

thread_local! {
	static FACTORY_STATE: RefCell<FactoryState> = const { RefCell::new(FactoryState {
		supplied: std::ptr::null_mut(),
		other: std::ptr::null_mut(),
		versions: Vec::new(),
		other_calls: 0,
	}) };
}

unsafe extern "C" fn supplied_factory(
	name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	// SAFETY: The SDK gives CreateInterface a live, terminated version name.
	let name = unsafe { CStr::from_ptr(name) };
	FACTORY_STATE.with_borrow_mut(|state| {
		state.versions.push(name.to_owned());
		if name == sdk_raw::interfaces::base_file_system::VERSION {
			state.supplied
		} else {
			std::ptr::null_mut()
		}
	})
}

unsafe extern "C" fn other_factory(_name: *const c_char, _return_code: *mut c_int) -> *mut c_void {
	FACTORY_STATE.with_borrow_mut(|state| {
		state.other_calls += 1;
		state.other
	})
}

struct FactoryScope;

impl FactoryScope {
	fn new(supplied: Option<&Mock>, other: &Mock) -> Self {
		FACTORY_STATE.with_borrow_mut(|state| {
			*state = FactoryState {
				supplied: supplied.map_or(std::ptr::null_mut(), |mock| {
					std::ptr::from_ref(&mock.raw).cast_mut().cast()
				}),
				other: std::ptr::from_ref(&other.raw).cast_mut().cast(),
				versions: Vec::new(),
				other_calls: 0,
			};
		});
		Self
	}
}

impl Drop for FactoryScope {
	fn drop(&mut self) {
		FACTORY_STATE.with_borrow_mut(|state| {
			state.supplied = std::ptr::null_mut();
			state.other = std::ptr::null_mut();
		});
	}
}

#[test]
fn supplied_factory_binds_the_exact_base_interface_without_other_factories() {
	let supplied = Mock::new(b"supplied map");
	let other = Mock::new(b"other map");
	let scope = FactoryScope::new(Some(&supplied), &other);
	// SAFETY: Both factories export only these live fakes. The callback scope,
	// boxes and generated vtables remain live on this thread throughout use.
	let server = unsafe {
		crate::Server::new(
			crate::InterfaceFactory::new(other_factory),
			crate::InterfaceFactory::new(other_factory),
			crate::Game::TeamFortress2,
			&scope,
		)
	};
	// SAFETY: The supplied factory returns the exact live IBaseFileSystem
	// fake from this same scope and thread; no native lifetime is extended.
	let fs = unsafe {
		server.base_file_system_from_factory(crate::InterfaceFactory::new(supplied_factory))
	}
	.unwrap();
	assert_eq!(fs.as_ptr(), std::ptr::from_ref(&supplied.raw).cast_mut());
	assert_eq!(
		fs.read_file(c"maps/supplied.bsp", Some(c"GAME"), ReadLimits::default())
			.unwrap(),
		b"supplied map"
	);
	supplied.closed_once();
	assert_eq!(other.state.borrow().opens, 0);
	FACTORY_STATE.with_borrow(|state| {
		assert_eq!(state.versions, [c"VBaseFileSystem011".to_owned()]);
		assert_eq!(state.other_calls, 0);
	});
}

#[test]
fn missing_supplied_export_names_its_route_and_never_falls_back() {
	let other = Mock::new(b"available from engine and game server");
	let scope = FactoryScope::new(None, &other);
	// SAFETY: Both stored factories export only the live same-thread fake.
	let server = unsafe {
		crate::Server::new(
			crate::InterfaceFactory::new(other_factory),
			crate::InterfaceFactory::new(other_factory),
			crate::Game::TeamFortress2,
			&scope,
		)
	};
	// SAFETY: The supplied same-thread factory is live and returns null.
	let error = unsafe {
		server.base_file_system_from_factory(crate::InterfaceFactory::new(supplied_factory))
	}
	.unwrap_err();
	assert_eq!(error.version(), c"VBaseFileSystem011");
	assert_eq!(
		error.to_string(),
		"the supplied filesystem factory does not export `VBaseFileSystem011`"
	);
	FACTORY_STATE.with_borrow(|state| {
		assert_eq!(state.versions, [c"VBaseFileSystem011".to_owned()]);
		assert_eq!(state.other_calls, 0);
	});
	assert_eq!(other.state.borrow().opens, 0);
}
