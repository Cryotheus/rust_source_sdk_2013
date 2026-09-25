#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod artifact;

#[cfg(feature = "bindings")]
pub mod bindings;

pub mod config;

#[cfg(feature = "determinism")]
pub mod determinism;

#[macro_export]
macro_rules! env_cstr {
	($Key:literal) => {
		const {
			let ::core::result::Result::Ok(cstr) = ::core::ffi::CStr::from_bytes_with_nul(
				::core::concat!(::core::env!($Key), "\0").as_bytes(),
			) else {
				::core::panic!(
					"{}",
					::core::concat!("Environment variable ", $Key, " must not contain nul bytes")
				);
			};

			cstr
		}
	};
}
