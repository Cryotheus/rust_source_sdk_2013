#![cfg_attr(docsrs, feature(doc_cfg))]

/// Simplifies the declaration of Source SDK console commands.
///
/// Todo: invocation syntax
#[macro_export]
macro_rules! commands {
	($(
		$(#[$StaticMeta:meta])*
		$(@[$Method:ident $($MethodArgs:expr),* $(,)? ])*
		$StaticVis:vis static $Static:ident = fn $Command:ident ($Ctx:pat) {
			$($Body:tt)*
		}
	)+) => {
		$(
		$(#[$StaticMeta])*
		$StaticVis static $Static: ::source_sdk_2013::commands::ConsoleCommand<::source_sdk_2013::commands::CommandFn> =
			::source_sdk_2013::commands::ConsoleCommand::new(
				$crate::__private_stringify_cstr!($Command),
				{
					#[allow(non_snake_case)]
					fn $Command($Ctx: &::source_sdk_2013::commands::CommandContext<'_>) -> ::source_sdk_2013::commands::CommandResult {
						$($Body)*
					}

					$Command as ::source_sdk_2013::commands::CommandFn
				}
			)
			$( .$Method($($MethodArgs),*) )*;
		)+
	};
}

#[doc(hidden)]
pub use crys_bricks::stringify_cstr as __private_stringify_cstr;
