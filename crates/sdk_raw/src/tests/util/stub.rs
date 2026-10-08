use super::*;

fn address(function: extern "C" fn(i32) -> i32) -> NonNull<c_void> {
	NonNull::new(function as *mut c_void).expect("functions have addresses")
}

fn call(stub: JumpStub, value: i32) -> i32 {
	// SAFETY: The stub jumps to one of this module's functions of this
	// signature, passing the argument through.
	let function = unsafe {
		std::mem::transmute::<*mut c_void, extern "C" fn(i32) -> i32>(stub.entry().as_ptr())
	};

	function(value)
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn code_page_is_executable() {
	let stub = JumpStub::new(address(first)).expect("the stub is allocated");

	assert!(crate::util::is_executable(stub.entry().addr().get()));
}

extern "C" fn first(value: i32) -> i32 {
	value + 1
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn jumps_to_its_target() {
	let stub = JumpStub::new(address(first)).expect("the stub is allocated");

	assert_eq!(call(stub, 41), 42);
	assert_eq!(stub.target(), address(first));
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn retargets() {
	let stub = JumpStub::new(address(first)).expect("the stub is allocated");

	stub.retarget(address(second));

	assert_eq!(call(stub, 21), 42);
	assert_eq!(stub.target(), address(second));
}

extern "C" fn second(value: i32) -> i32 {
	value * 2
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn stubs_are_independent() {
	let one = JumpStub::new(address(first)).expect("the stub is allocated");
	let two = JumpStub::new(address(second)).expect("the stub is allocated");

	assert_ne!(one.entry(), two.entry());
	assert_eq!(call(one, 1), 2);
	assert_eq!(call(two, 1), 2);

	one.retarget(address(second));

	assert_eq!(call(one, 5), 10);
	assert_eq!(call(two, 5), 10);
}
