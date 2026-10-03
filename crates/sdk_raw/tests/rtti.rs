//! Tests of matching the class names run-time type information holds.

use source_sdk_2013_raw::util::rtti::matches_class_name;

#[test]
fn names_follow_the_abi() {
	#[cfg(target_os = "windows")]
	{
		assert!(matches_class_name(b".?AVCGameClient@@", "CGameClient"));
		assert!(!matches_class_name(b".?AVCGameClientX@@", "CGameClient"));
		assert!(!matches_class_name(b"11CGameClient", "CGameClient"));
	}

	#[cfg(not(target_os = "windows"))]
	{
		assert!(matches_class_name(b"11CGameClient", "CGameClient"));
		assert!(!matches_class_name(b"12CGameClientX", "CGameClient"));
		assert!(!matches_class_name(b".?AVCGameClient@@", "CGameClient"));
	}
}
