//! Raw Source SDK bindings generated for TF2's server configuration.
//!
//! Game-side records include economy items and NextBot. Generated virtual
//! tables describe each class's primary address point for the target C++ ABI;
//! secondary base interfaces retain their own generated tables.

#![cfg_attr(docsrs, feature(doc_cfg))]

cfg_select! {
	all(target_os = "linux", target_arch = "x86", target_env = "gnu") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu") => {
		pub use source_sdk_2013_sys_x64_linux::*;
	}

	all(target_os = "windows", target_arch = "x86", target_env = "msvc") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc") => {
		pub use source_sdk_2013_sys_x64_windows::*;
	}

	_ => {
		compile_error!("Unsupported target");
	}
}
