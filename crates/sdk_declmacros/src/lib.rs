#![cfg_attr(docsrs, feature(doc_cfg))]

/// Simplifies the declaration of Source SDK console commands.
///
/// Todo: invocation syntax
#[macro_export]
macro_rules! commands {
	(
		extern crate $Package:ident;
		$(
			$(#[$StaticMeta:meta])*
			$(@[$Method:ident $($MethodArgs:expr),* $(,)? ])*
			$StaticVis:vis static $Static:ident = fn $Command:ident ($Ctx:pat) {
				$($Body:tt)*
			}
		)+
	) => {
		$(
		$(#[$StaticMeta])*
		$StaticVis static $Static: ::$Package::commands::ConsoleCommand<::$Package::commands::CommandFn> =
			::$Package::commands::ConsoleCommand::new(
				$crate::__private_stringify_cstr!($Command),
				{
					#[allow(non_snake_case)]
					fn $Command($Ctx: &::$Package::commands::CommandContext<'_>) -> ::$Package::commands::CommandResult {
						$($Body)*
					}

					$Command as ::$Package::commands::CommandFn
				}
			)
			$( .$Method($($MethodArgs),*) )*;
		)+
	};

	($($Tokens:tt)*) => {
		$crate::commands! {
			extern crate source_sdk_2013;
			$($Tokens)*
		}
	};
}

#[doc(hidden)]
pub use crys_bricks::stringify_cstr as __private_stringify_cstr;
