use anyhow::ensure;
use std::env::{VarError, var, var_os};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Profile {
	Standard(RsProfile),
	Custom(String),
}

impl Profile {
	#[doc(alias = "new")]
	pub fn from_env() -> Result<Self, RequireVarError> {
		match RsProfile::from_env() {
			Ok(profile) => Ok(Self::Standard(profile)),
			Err(RsProfileError::NonStandard(profile)) => Ok(Self::Custom(profile)),
			Err(RsProfileError::RequireVar(error)) => Err(error),
		}
	}

	pub fn is_bench(&self) -> bool {
		matches!(self, Self::Standard(RsProfile::Bench))
	}

	pub fn is_debug(&self) -> bool {
		matches!(self, Self::Standard(RsProfile::Debug))
	}

	pub fn is_release(&self) -> bool {
		matches!(self, Self::Standard(RsProfile::Release))
	}

	pub fn is_test(&self) -> bool {
		matches!(self, Self::Standard(RsProfile::Test))
	}
}

#[derive(Debug, Clone)]
pub struct RequireFileError {
	description: String,
	path: PathBuf,
}

impl Display for RequireFileError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let Self { description, path } = self;

		write!(f, "{description} was not found at `{}`", path.display())
	}
}

impl Error for RequireFileError {}

#[derive(Debug, Clone)]
pub enum RequireVarError {
	NotPresent {
		key: &'static str,
		diagnostic: &'static str,
	},
	NotUnicode {
		key: &'static str,
		value: OsString,
	},
}

impl Display for RequireVarError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			RequireVarError::NotPresent { key, diagnostic } => writeln!(
				f,
				"missing required environment variable {key:?}; {diagnostic}"
			),
			RequireVarError::NotUnicode { key, .. } => writeln!(
				f,
				"value of environment variable {key:?} must be valid UTF-8"
			),
		}
	}
}

impl Error for RequireVarError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			RequireVarError::NotPresent { .. } => Some(&VarError::NotPresent),
			RequireVarError::NotUnicode { .. } => None,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RsProfile {
	#[doc(alias = "Dev")]
	Debug,

	Release,

	Test,

	Bench,
}

impl RsProfile {
	#[doc(alias = "new")]
	pub fn from_env() -> Result<Self, RsProfileError> {
		let string = cargo_var("PROFILE")?;

		Self::parse(&string).ok_or(RsProfileError::NonStandard(string.into_inner()))
	}

	pub const fn parse(str: &str) -> Option<Self> {
		// HACK: can't match `&str` in const yet
		match str.as_bytes() {
			b"debug" => Some(Self::Debug),
			b"release" => Some(Self::Release),
			b"test" => Some(Self::Test),
			b"bench" => Some(Self::Bench),
			_ => None,
		}
	}

	/// Returns `true` if this profile defaults to enabling debug symbols.
	/// If you need to know if debug symbols are enabled, parse the `"DEBUG"` environment variable instead.
	pub const fn debug_assertions(&self) -> bool {
		matches!(self, Self::Debug | Self::Test)
	}

	pub const fn is_bench(&self) -> bool {
		matches!(self, RsProfile::Bench)
	}

	pub const fn is_debug(&self) -> bool {
		matches!(self, RsProfile::Debug)
	}

	pub const fn is_release(&self) -> bool {
		matches!(self, RsProfile::Release)
	}

	pub const fn is_test(&self) -> bool {
		matches!(self, RsProfile::Test)
	}
}

impl FromStr for RsProfile {
	type Err = RsProfileError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Self::parse(s).ok_or_else(|| RsProfileError::NonStandard(String::from(s)))
	}
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum RsProfileError {
	#[error("Non-standard profile {0:?}")]
	NonStandard(String),

	#[error("{0}")]
	RequireVar(#[from] RequireVarError),
}

#[derive(Debug, Clone)]
pub enum PathResolutionMethod {
	/// Use an environment variable as the path.
	Env(&'static OsStr),

	/// Use a path.
	Provided(PathBuf),
}

impl PathResolutionMethod {
	pub fn new_env(var: &'static str) -> Self {
		Self::Env(OsStr::new(var))
	}

	pub fn resolve(self) -> Result<PathBuf, PathResolutionError> {
		match self {
			Self::Env(var) => var_os(var)
				.map(PathBuf::from)
				.ok_or(PathResolutionError::MissingEnv { var }),
			Self::Provided(path) => path.canonicalize().map_err(PathResolutionError::StdIo),
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum PathResolutionError {
	#[error("{0}")]
	StdIo(#[from] std::io::Error),

	#[error("Missing environment var {:?}", var.to_string_lossy())]
	MissingEnv { var: &'static OsStr },
}

/// For build scripts to print cargo rerun lines.
#[derive(Debug, Clone, Copy)]
pub enum RerunEmitter<'a> {
	Path(&'a Path),
	Var(&'a str),
}

impl<'a> RerunEmitter<'a> {
	pub fn emit(self) {
		match self {
			RerunEmitter::Path(path) => println!("cargo:rerun-if-changed={}", path.display()),
			RerunEmitter::Var(var) => println!("cargo:rerun-if-env-changed={var}"),
		}
	}
}

#[derive(Debug, Clone, Copy)]
pub struct WithRerunEmitter<'a, T> {
	pub value: T,
	pub rerun: RerunEmitter<'a>,
}

impl<'a, T> WithRerunEmitter<'a, T> {
	pub fn new_path(value: T, path: &'a Path) -> Self {
		Self {
			value,
			rerun: RerunEmitter::Path(path),
		}
	}

	pub fn new_var(value: T, var: &'a str) -> Self {
		Self {
			value,
			rerun: RerunEmitter::Var(var),
		}
	}

	pub fn emit(self) -> T {
		self.rerun.emit();
		self.value
	}

	#[must_use]
	pub fn into_inner(self) -> T {
		self.value
	}
}

impl<'a, T> Deref for WithRerunEmitter<'a, T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&self.value
	}
}

impl<'a, T> DerefMut for WithRerunEmitter<'a, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.value
	}
}

/// [`require_var`] with a default diagnostic message.
pub fn cargo_var(key: &'static str) -> Result<WithRerunEmitter<'static, String>, RequireVarError> {
	require_var(
		key,
		"You must use or otherwise emulate `cargo` to run this build script",
	)
}

/// [`require_var_os`] with a default diagnostic message.
pub fn cargo_var_os(
	key: &'static str,
) -> Result<WithRerunEmitter<'static, OsString>, RequireVarError> {
	require_var_os(
		key,
		"You must use or otherwise emulate `cargo` to run this build script",
	)
}

pub fn require_dir(path: impl AsRef<Path>, description: &str) -> anyhow::Result<()> {
	let path = path.as_ref();

	ensure!(
		path.is_dir(),
		"{description} must exist at {}",
		path.display()
	);
	Ok(())
}

pub fn require_file<'path>(
	path: &'path Path,
	description: &str,
) -> anyhow::Result<RerunEmitter<'path>> {
	ensure!(
		path.is_file(),
		"{description} was not found at {}",
		path.display()
	);
	Ok(RerunEmitter::Path(path))
}

pub fn require_var(
	key: &'static str,
	diagnostic: &'static str,
) -> Result<WithRerunEmitter<'static, String>, RequireVarError> {
	match var(key) {
		Ok(string) => Ok(WithRerunEmitter::new_var(string, key)),
		Err(VarError::NotPresent) => Err(RequireVarError::NotPresent { key, diagnostic }),
		Err(VarError::NotUnicode(value)) => Err(RequireVarError::NotUnicode { key, value }),
	}
}

pub fn require_var_os(
	key: &'static str,
	diagnostic: &'static str,
) -> Result<WithRerunEmitter<'static, OsString>, RequireVarError> {
	match var_os(key) {
		None => Err(RequireVarError::NotPresent { key, diagnostic }),
		Some(os_string) => Ok(WithRerunEmitter::new_var(os_string, key)),
	}
}
