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

#[macro_export]
macro_rules! stringify_cstr {
	($($Tokens:tt)*) => {
		{
			let Ok(__macro_expansion__stringify_cstr) = ::core::ffi::CStr::from_bytes_until_nul(
				::core::concat!(::core::stringify!($($Tokens)*), "\x00").as_bytes()
			) else { panic!() };

			__macro_expansion__stringify_cstr
		}
	};
}
