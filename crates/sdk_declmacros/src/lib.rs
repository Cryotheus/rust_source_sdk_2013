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
		)*
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
		)*
	};

	(extern crate $Package:ident; $($Tokens:tt)*) => { ::core::compile_error!("Invalid `commands!` input") };

	($($Tokens:tt)*) => {
		$crate::commands! {
			extern crate source_sdk_2013;
			$($Tokens)*
		}
	};
}

/// Adaptation of the [`commands`] macro.
///
/// Todo: invocation syntax
#[macro_export]
macro_rules! convars {
	(
		extern crate $Package:ident;

		$(
			$(#[$StaticMeta:meta])*
			$(@[$Method:ident $($MethodArgs:expr),* $(,)? ])*
			$StaticVis:vis static $Static:ident $Convar:ident = $Default:expr $(, $($Min:literal)? ..= $($Max:literal)?)?;
		)*
	) => {
		$(
		$(#[$StaticMeta])*
		$StaticVis static $Static: ::$Package::commands::ConsoleVariable = ::$Package::commands::ConsoleVariable::new(
			$crate::__private_stringify_cstr!($Convar),
			$Default
		)
		$($(.min($Min))? $(.max($Max))?)?
		$( .$Method($($MethodArgs),*) )*;
		)*
	};

	(extern crate $Package:ident; $($Tokens:tt)*) => { ::core::compile_error!("Invalid `convars!` input") };

	($($Tokens:tt)*) => {
		$crate::convars! {
			extern crate source_sdk_2013;
			$($Tokens)*
		}
	};
}

#[doc(hidden)]
pub use crys_bricks::stringify_cstr as __private_stringify_cstr;
