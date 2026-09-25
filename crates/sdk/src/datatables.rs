//! Networked class descriptions (`public/server_class.h`, `public/dt_send.h`)
//! and access to the variables they describe.
//!
//! The game describes each networked entity class with a [`ServerClass`],
//! whose [`SendTable`] lists the [`SendProp`]s sent to clients. Tables nest:
//! a class's table embeds its base class's table as a `baseclass` property,
//! and embedded structures as further tables.
//!
//! [`NetProp`] resolves a property by name to where it lives in an entity, as
//! SourceMod's `Prop_Send` lookups do, and reads or writes it after checking
//! the entity's class and the variable's storage.

use crate::edicts::Edict;
use crate::entities::Entity;
use crate::ffi::{NotThreadSafe, borrow_cstr, copy_cstr};
use crate::interfaces::ValveEngine;
use crate::math::{QAngle, Vector};
use glam::Vec2;
use std::any::type_name;
use std::ffi::{CStr, CString, c_int, c_void};
use std::fmt::{self, Debug, Display, Formatter};
use std::marker::PhantomData;
use std::mem::zeroed;
use std::ptr::NonNull;

/// How deep [`NetProp`] lookups descend into nested tables before giving up.
const MAX_TABLE_DEPTH: usize = 32;

/// How many non-modifying proxies [`StandardSendProxies`] reads before
/// assuming the list is corrupt.
const MAX_NON_MODIFIED_PROXIES: usize = 256;

/// A networked entity class, as registered by the game DLL (`ServerClass`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerClass<'s> {
	raw: NonNull<sys::ServerClass>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ServerClass<'s> {
	/// # Safety
	///
	/// `raw` must be one of the game DLL's server classes, which are statics.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::ServerClass>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::ServerClass {
		self.raw.as_ptr()
	}

	/// The class's network name, such as `CTFPlayer`.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: Server classes and their names are statics of the game DLL.
		// Fields are read without forming references.
		unsafe { borrow_cstr((&raw const (*self.as_ptr()).m_pNetworkName).read()) }
			.unwrap_or_default()
	}

	/// The ID the engine assigned the class for networking.
	pub fn class_id(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { (&raw const (*self.as_ptr()).m_ClassID).read() }
	}

	/// The table of the class's networked variables.
	pub fn table(self) -> Option<SendTable<'s>> {
		// SAFETY: As for `name`.
		let table = NonNull::new(unsafe { (&raw const (*self.as_ptr()).m_pTable).read() })?;

		// SAFETY: Send tables are statics of the game DLL.
		Some(unsafe { SendTable::from_raw(table) })
	}

	fn next(self) -> Option<Self> {
		// SAFETY: As for `name`.
		let next = NonNull::new(unsafe { (&raw const (*self.as_ptr()).m_pNext).read() })?;

		// SAFETY: The list only links server classes.
		Some(unsafe { Self::from_raw(next) })
	}
}

/// Iterator over the game DLL's server classes, sorted by name.
#[derive(Debug, Clone)]
pub struct ServerClasses<'s> {
	next: Option<ServerClass<'s>>,
}

impl<'s> ServerClasses<'s> {
	/// # Safety
	///
	/// `head` must be null or the head of the game DLL's server class list.
	pub(crate) unsafe fn new(head: *mut sys::ServerClass) -> Self {
		Self {
			// SAFETY: The caller upholds the contract.
			next: NonNull::new(head).map(|head| unsafe { ServerClass::from_raw(head) }),
		}
	}
}

impl<'s> Iterator for ServerClasses<'s> {
	type Item = ServerClass<'s>;

	fn next(&mut self) -> Option<Self::Item> {
		let class = self.next?;

		self.next = class.next();
		Some(class)
	}
}

/// A table of networked variables (`SendTable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SendTable<'s> {
	raw: NonNull<sys::SendTable>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> SendTable<'s> {
	/// # Safety
	///
	/// `raw` must be one of the game DLL's send tables, which are statics.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::SendTable>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::SendTable {
		self.raw.as_ptr()
	}

	/// The table's name, such as `DT_TFPlayer`.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: Send tables and their names are statics of the game DLL.
		// Fields are read without forming references.
		unsafe { borrow_cstr((&raw const (*self.as_ptr()).m_pNetTableName).read()) }
			.unwrap_or_default()
	}

	#[doc(alias = "GetNumProps")]
	pub fn len(self) -> usize {
		// SAFETY: As for `name`.
		usize::try_from(unsafe { (&raw const (*self.as_ptr()).m_nProps).read() }).unwrap_or(0)
	}

	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	#[doc(alias = "GetProp")]
	pub fn prop(self, index: usize) -> Option<SendProp<'s>> {
		if index >= self.len() {
			return None;
		}

		// SAFETY: As for `name`.
		let props = NonNull::new(unsafe { (&raw const (*self.as_ptr()).m_pProps).read() })?;

		// SAFETY: The index is within the table's array of properties.
		Some(unsafe { SendProp::from_raw(props.add(index)) })
	}

	pub fn props(self) -> SendProps<'s> {
		SendProps {
			table: self,
			index: 0,
		}
	}

	/// The table of the base class, which classes embed as a property named
	/// `baseclass`.
	pub fn base(self) -> Option<SendTable<'s>> {
		self.props()
			.find(|prop| prop.kind() == PropKind::DataTable && prop.name() == c"baseclass")
			.and_then(SendProp::data_table)
	}

	/// Whether this is `base`, or derives from it through `baseclass` tables.
	///
	/// Base classes sit at offset 0 of their derived classes, so offsets
	/// resolved in `base` hold for this table's entities too.
	pub fn derives_from(self, base: SendTable<'_>) -> bool {
		let mut table = Some(self);

		for _ in 0..MAX_TABLE_DEPTH {
			match table {
				Some(current) if current.as_ptr() == base.as_ptr() => return true,
				Some(current) => table = current.base(),
				None => return false,
			}
		}

		false
	}
}

/// Iterator over a table's properties.
#[derive(Debug, Clone)]
pub struct SendProps<'s> {
	table: SendTable<'s>,
	index: usize,
}

impl<'s> Iterator for SendProps<'s> {
	type Item = SendProp<'s>;

	fn next(&mut self) -> Option<Self::Item> {
		let prop = self.table.prop(self.index)?;

		self.index += 1;
		Some(prop)
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		let remaining = self.table.len().saturating_sub(self.index);

		(remaining, Some(remaining))
	}
}

impl ExactSizeIterator for SendProps<'_> {}

/// The type a [`SendProp`] is networked as.
#[doc(alias = "SendPropType")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropKind {
	Int,
	Float,
	Vector,

	/// A vector of which only X and Y are networked.
	VectorXY,

	String,

	/// Elements described by [`SendProp::array_prop`].
	Array,

	/// A nested [`SendTable`].
	DataTable,

	/// A type this crate does not know.
	Unknown(i64),
}

impl PropKind {
	fn from_raw(raw: sys::SendPropType) -> Self {
		match raw {
			sys::SendPropType_DPT_Int => Self::Int,
			sys::SendPropType_DPT_Float => Self::Float,
			sys::SendPropType_DPT_Vector => Self::Vector,
			sys::SendPropType_DPT_VectorXY => Self::VectorXY,
			sys::SendPropType_DPT_String => Self::String,
			sys::SendPropType_DPT_Array => Self::Array,
			sys::SendPropType_DPT_DataTable => Self::DataTable,
			unknown => Self::Unknown(i64::from(unknown)),
		}
	}
}

impl Display for PropKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::Int => f.write_str("integer"),
			Self::Float => f.write_str("float"),
			Self::Vector => f.write_str("vector"),
			Self::VectorXY => f.write_str("XY vector"),
			Self::String => f.write_str("string"),
			Self::Array => f.write_str("array"),
			Self::DataTable => f.write_str("data table"),
			Self::Unknown(raw) => write!(f, "unknown type {raw}"),
		}
	}
}

/// The `SPROP_*` flags of a [`SendProp`], from `public/dt_common.h`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PropFlags(c_int);

impl PropFlags {
	pub const UNSIGNED: Self = Self(1 << 0);
	pub const COORD: Self = Self(1 << 1);
	pub const NO_SCALE: Self = Self(1 << 2);
	pub const ROUND_DOWN: Self = Self(1 << 3);
	pub const ROUND_UP: Self = Self(1 << 4);
	pub const NORMAL: Self = Self(1 << 5);

	/// The property names another property to exclude, rather than a variable.
	pub const EXCLUDE: Self = Self(1 << 6);

	pub const XYZE: Self = Self(1 << 7);

	/// The property describes the elements of the array property after it.
	pub const INSIDE_ARRAY: Self = Self(1 << 8);

	pub const PROXY_ALWAYS_YES: Self = Self(1 << 9);
	pub const CHANGES_OFTEN: Self = Self(1 << 10);
	pub const IS_A_VECTOR_ELEM: Self = Self(1 << 11);
	pub const COLLAPSIBLE: Self = Self(1 << 12);
	pub const COORD_MP: Self = Self(1 << 13);
	pub const COORD_MP_LOW_PRECISION: Self = Self(1 << 14);
	pub const COORD_MP_INTEGRAL: Self = Self(1 << 15);
	pub const ENCODED_AGAINST_TICK_COUNT: Self = Self(1 << 16);

	pub const fn from_bits(bits: c_int) -> Self {
		Self(bits)
	}

	pub const fn bits(self) -> c_int {
		self.0
	}

	pub const fn contains(self, flags: Self) -> bool {
		self.0 & flags.0 == flags.0
	}
}

impl Debug for PropFlags {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "PropFlags({:#x})", self.0)
	}
}

/// A networked variable's description (`SendProp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SendProp<'s> {
	raw: NonNull<sys::SendProp>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

/// Reads a field of a `SendProp` without forming a reference.
macro_rules! prop_field {
	($prop:expr, $field:ident) => {
		// SAFETY: Properties are statics of the game DLL, which the engine only
		// changes while initializing its tables at startup.
		unsafe { (&raw const (*$prop.as_ptr()).$field).read() }
	};
}

impl<'s> SendProp<'s> {
	/// # Safety
	///
	/// `raw` must be a property of one of the game DLL's send tables.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::SendProp>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::SendProp {
		self.raw.as_ptr()
	}

	/// The variable's name, such as `m_iHealth`.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'s CStr {
		let name = prop_field!(self, m_pVarName);

		// SAFETY: Names are string literals of the game DLL.
		unsafe { borrow_cstr(name) }.unwrap_or_default()
	}

	#[doc(alias = "GetType")]
	pub fn kind(self) -> PropKind {
		PropKind::from_raw(prop_field!(self, m_Type))
	}

	#[doc(alias = "GetFlags")]
	pub fn flags(self) -> PropFlags {
		PropFlags(prop_field!(self, m_Flags))
	}

	/// The number of bits the value is encoded with.
	pub fn bits(self) -> c_int {
		prop_field!(self, m_nBits)
	}

	/// The lowest value a float is encoded to.
	pub fn low_value(self) -> f32 {
		prop_field!(self, m_fLowValue)
	}

	/// The highest value a float is encoded to.
	pub fn high_value(self) -> f32 {
		prop_field!(self, m_fHighValue)
	}

	/// Bytes from the start of the structure the property's table describes.
	#[doc(alias = "GetOffset")]
	pub fn offset(self) -> c_int {
		prop_field!(self, m_Offset)
	}

	/// The nested table of a [`PropKind::DataTable`] property.
	#[doc(alias = "GetDataTable")]
	pub fn data_table(self) -> Option<SendTable<'s>> {
		// SAFETY: Send tables are statics of the game DLL.
		NonNull::new(prop_field!(self, m_pDataTable))
			.map(|table| unsafe { SendTable::from_raw(table) })
	}

	/// The property describing each element of a [`PropKind::Array`] property.
	#[doc(alias = "GetArrayProp")]
	pub fn array_prop(self) -> Option<SendProp<'s>> {
		// SAFETY: The array's element property precedes it in the same table.
		NonNull::new(prop_field!(self, m_pArrayProp))
			.map(|prop| unsafe { SendProp::from_raw(prop) })
	}

	/// The number of elements of a [`PropKind::Array`] property.
	#[doc(alias = "GetNumElements")]
	pub fn element_count(self) -> c_int {
		prop_field!(self, m_nElements)
	}

	/// Bytes between consecutive elements of a [`PropKind::Array`] property.
	#[doc(alias = "GetElementStride")]
	pub fn element_stride(self) -> c_int {
		prop_field!(self, m_ElementStride)
	}

	/// The table an [exclude](PropFlags::EXCLUDE) property excludes a property from.
	#[doc(alias = "GetExcludeDTName")]
	pub fn exclude_table_name(self) -> Option<&'s CStr> {
		if !self.flags().contains(PropFlags::EXCLUDE) {
			return None;
		}

		let table = prop_field!(self, m_pExcludeDTName);

		// SAFETY: As for `name`.
		unsafe { borrow_cstr(table) }
	}

	fn var_proxy(self) -> sys::SendVarProxyFn {
		prop_field!(self, m_ProxyFn)
	}

	fn table_proxy(self) -> sys::SendTableProxyFn {
		prop_field!(self, m_DataTableProxyFn)
	}
}

/// How a networked variable is stored in its entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Storage {
	I8,
	U8,
	I16,
	U16,
	I32,
	U32,
	F32,

	/// Three floats, as a `Vector` or `QAngle`.
	Vector,

	/// Stored in a way this crate cannot verify, because the property has a
	/// custom send proxy; [`NetProp::value`] can still read what clients see.
	Unknown,
}

impl Storage {
	/// Whether a variable stored as `self` can be accessed as `other`.
	///
	/// Integers only need the same width: signedness comes from
	/// `SPROP_UNSIGNED`, which describes how a value is networked, and the game
	/// sends many `int` variables unsigned.
	pub const fn is_compatible(self, other: Self) -> bool {
		matches!(
			(self, other),
			(Self::I8 | Self::U8, Self::I8 | Self::U8)
				| (Self::I16 | Self::U16, Self::I16 | Self::U16)
				| (Self::I32 | Self::U32, Self::I32 | Self::U32)
				| (Self::F32, Self::F32)
				| (Self::Vector, Self::Vector)
		)
	}
}

impl Display for Storage {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::I8 => "i8",
			Self::U8 => "u8",
			Self::I16 => "i16",
			Self::U16 => "u16",
			Self::I32 => "i32",
			Self::U32 => "u32",
			Self::F32 => "f32",
			Self::Vector => "a vector",
			Self::Unknown => "an unknown type",
		})
	}
}

/// The game DLL's standard send proxies (`CStandardSendProxies`).
///
/// A property's proxy converts its variable for networking, so a standard
/// proxy reveals how the variable is stored. Comparisons use widths only,
/// since a linker may fold proxies whose code is identical into one.
#[doc(alias = "CStandardSendProxies")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StandardSendProxies<'s> {
	raw: NonNull<sys::CStandardSendProxies>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

/// The address of a variable proxy, for comparing proxies.
fn var_proxy_address(proxy: sys::SendVarProxyFn) -> Option<usize> {
	proxy.map(|proxy| proxy as usize)
}

/// The address of a table proxy, for comparing proxies.
fn table_proxy_address(proxy: sys::SendTableProxyFn) -> Option<usize> {
	proxy.map(|proxy| proxy as usize)
}

impl<'s> StandardSendProxies<'s> {
	/// # Safety
	///
	/// `raw` must be the game DLL's `g_StandardSendProxies`.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::CStandardSendProxies>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CStandardSendProxies {
		self.raw.as_ptr()
	}

	fn proxies(self) -> sys::CStandardSendProxies {
		// SAFETY: The proxies are a static of the game DLL, set up before any
		// plugin loads. They are copied without forming references.
		unsafe { self.as_ptr().read() }
	}

	/// How a scalar property's variable is stored, judged by its proxy.
	fn storage(self, prop: SendProp<'_>) -> Storage {
		let proxies = self.proxies()._base;
		let Some(proxy) = var_proxy_address(prop.var_proxy()) else {
			return Storage::Unknown;
		};

		let is = |candidates: &[sys::SendVarProxyFn]| {
			candidates
				.iter()
				.any(|&candidate| var_proxy_address(candidate) == Some(proxy))
		};
		let signed = !prop.flags().contains(PropFlags::UNSIGNED);

		match prop.kind() {
			PropKind::Int if is(&[proxies.m_Int8ToInt32, proxies.m_UInt8ToInt32]) => {
				[Storage::U8, Storage::I8][usize::from(signed)]
			}
			PropKind::Int if is(&[proxies.m_Int16ToInt32, proxies.m_UInt16ToInt32]) => {
				[Storage::U16, Storage::I16][usize::from(signed)]
			}
			PropKind::Int if is(&[proxies.m_Int32ToInt32, proxies.m_UInt32ToInt32]) => {
				[Storage::U32, Storage::I32][usize::from(signed)]
			}
			PropKind::Float
				if is(&[
					proxies.m_FloatToFloat,
					proxies.m_Int32ToInt32,
					proxies.m_UInt32ToInt32,
				]) =>
			{
				Storage::F32
			}
			PropKind::Vector if is(&[proxies.m_VectorToVector]) => Storage::Vector,
			_ => Storage::Unknown,
		}
	}

	/// Whether a table proxy passes its data through unchanged, so offsets
	/// into the nested table are relative to the containing structure.
	fn is_direct(self, proxy: sys::SendTableProxyFn) -> bool {
		let Some(proxy) = table_proxy_address(proxy) else {
			return false;
		};

		let proxies = self.proxies();

		if [proxies.m_DataTableToDataTable, proxies.m_SendLocalDataTable]
			.into_iter()
			.any(|candidate| table_proxy_address(candidate) == Some(proxy))
		{
			return true;
		}

		// The game registers every other pointer-preserving table proxy in a
		// list, which the engine consults for the same purpose.
		let Some(head) = NonNull::new(proxies.m_ppNonModifiedPointerProxies) else {
			return false;
		};

		// SAFETY: The list and its nodes are statics of the game DLL.
		let mut node = unsafe { head.as_ptr().read() };

		for _ in 0..MAX_NON_MODIFIED_PROXIES {
			let Some(current) = NonNull::new(node) else {
				return false;
			};

			// SAFETY: As above.
			let current = unsafe { current.as_ptr().read() };

			if table_proxy_address(current.m_Fn) == Some(proxy) {
				return true;
			}

			node = current.m_pNext;
		}

		false
	}
}

/// A value as clients receive it, from [`NetProp::value`].
#[derive(Debug, Clone, PartialEq)]
pub enum NetValue {
	Int(c_int),
	Float(f32),
	Vector(Vector),
	VectorXY(Vec2),
	String(CString),
}

/// A type a networked variable can be read or written as.
///
/// Implemented for the primitive types of [`Storage`], `bool` for single-byte
/// flags, and [`Vector`] and [`QAngle`] for vectors. Integers are accessed
/// at the variable's width with the signedness of the type used.
pub trait NetVar: sealed::Sealed + Copy {
	#[doc(hidden)]
	const STORAGE: Storage;

	/// # Safety
	///
	/// `source` must point to a readable variable whose storage is
	/// [compatible](Storage::is_compatible) with [`Self::STORAGE`].
	#[doc(hidden)]
	unsafe fn read(source: *const u8) -> Self;

	/// # Safety
	///
	/// `destination` must point to a writable variable whose storage is
	/// [compatible](Storage::is_compatible) with [`Self::STORAGE`].
	#[doc(hidden)]
	unsafe fn write(self, destination: *mut u8);
}

mod sealed {
	pub trait Sealed {}
}

macro_rules! primitive_net_vars {
	($($Type:ty => $Storage:ident),* $(,)?) => {$(
		impl sealed::Sealed for $Type {}

		impl NetVar for $Type {
			const STORAGE: Storage = Storage::$Storage;

			unsafe fn read(source: *const u8) -> Self {
				// SAFETY: The caller upholds the contract. Every bit pattern is valid.
				unsafe { source.cast::<Self>().read_unaligned() }
			}

			unsafe fn write(self, destination: *mut u8) {
				// SAFETY: The caller upholds the contract.
				unsafe { destination.cast::<Self>().write_unaligned(self) }
			}
		}
	)*};
}

primitive_net_vars! {
	i8 => I8,
	u8 => U8,
	i16 => I16,
	u16 => U16,
	i32 => I32,
	u32 => U32,
	f32 => F32,
}

impl sealed::Sealed for bool {}

impl NetVar for bool {
	const STORAGE: Storage = Storage::U8;

	unsafe fn read(source: *const u8) -> Self {
		// SAFETY: The caller upholds the contract. The byte is read as an
		// integer, since the game may store any value in it.
		unsafe { source.read() != 0 }
	}

	unsafe fn write(self, destination: *mut u8) {
		// SAFETY: The caller upholds the contract.
		unsafe { destination.write(u8::from(self)) }
	}
}

impl sealed::Sealed for Vector {}

impl NetVar for Vector {
	const STORAGE: Storage = Storage::Vector;

	unsafe fn read(source: *const u8) -> Self {
		// SAFETY: The caller upholds the contract.
		unsafe { source.cast::<sys::Vector>().read_unaligned() }.into()
	}

	unsafe fn write(self, destination: *mut u8) {
		// SAFETY: The caller upholds the contract.
		unsafe {
			destination
				.cast::<sys::Vector>()
				.write_unaligned(self.into())
		}
	}
}

impl sealed::Sealed for QAngle {}

impl NetVar for QAngle {
	const STORAGE: Storage = Storage::Vector;

	unsafe fn read(source: *const u8) -> Self {
		// SAFETY: The caller upholds the contract.
		unsafe { source.cast::<sys::QAngle>().read_unaligned() }.into()
	}

	unsafe fn write(self, destination: *mut u8) {
		// SAFETY: The caller upholds the contract.
		unsafe {
			destination
				.cast::<sys::QAngle>()
				.write_unaligned(self.into())
		}
	}
}

/// Why a networked variable could not be found or accessed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetPropError {
	#[error("server class `{class}` has no send table")]
	NoTable { class: String },

	#[error("the game DLL provides no standard send proxies to recognize storage by")]
	NoStandardProxies,

	#[error("`{table}` has no networked variable named `{name}`")]
	NotFound { table: String, name: String },

	#[error(
		"`{name}` is inside `{table}`, whose send proxy relocates its data, so its address is unknown"
	)]
	Relocated { name: String, table: String },

	#[error("`{name}` has an invalid offset of {offset} bytes")]
	InvalidOffset { name: String, offset: c_int },

	#[error("`{name}` is {kind}, which has no single value")]
	NotAValue { name: String, kind: PropKind },

	#[error("`{name}` is {kind}, which has no elements")]
	NotAnArray { name: String, kind: PropKind },

	#[error("`{name}` has {len} elements, so it has no element {index}")]
	ElementOutOfRange {
		name: String,
		index: usize,
		len: usize,
	},

	#[error("`{name}` is stored as {storage}, not {requested}")]
	TypeMismatch {
		name: String,
		storage: Storage,
		requested: &'static str,
	},

	#[error(
		"`{name}` has a custom send proxy, so how it is stored is unknown; read it with `NetProp::value` instead"
	)]
	UnknownStorage { name: String },

	#[error("`{name}` has no send proxy")]
	NoProxy { name: String },

	#[error("`{class_name}` is not networked")]
	NotNetworked { class_name: String },

	#[error(
		"`{name}` belongs to `{expected}`, which `{class_name}`'s table `{found}` does not derive from"
	)]
	ClassMismatch {
		name: String,
		expected: String,
		class_name: String,
		found: String,
	},
}

fn lossy(string: &CStr) -> String {
	string.to_string_lossy().into_owned()
}

/// A networked variable resolved by name to where it lives in the entities of
/// one class and every class deriving from it.
///
/// Resolve one with
/// [`ServerGameDll::net_prop`](crate::interfaces::ServerGameDll::net_prop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetProp<'s> {
	prop: SendProp<'s>,
	class_table: SendTable<'s>,
	proxies: StandardSendProxies<'s>,
	/// Offset of the structure containing the variable, which proxies receive.
	struct_offset: usize,
	/// Offset of the variable.
	offset: usize,
	/// The element index proxies receive.
	element: c_int,
}

impl<'s> NetProp<'s> {
	/// Finds the first variable named `name` in `table` or the tables nested
	/// within it, depth first, as SourceMod does.
	pub(crate) fn resolve(
		table: SendTable<'s>,
		name: &CStr,
		proxies: StandardSendProxies<'s>,
	) -> Result<Self, NetPropError> {
		struct Found<'s> {
			prop: SendProp<'s>,
			struct_offset: usize,
			offset: c_int,
			relocated_by: Option<SendProp<'s>>,
		}

		fn search<'s>(
			table: SendTable<'s>,
			name: &CStr,
			base: usize,
			relocated_by: Option<SendProp<'s>>,
			proxies: StandardSendProxies<'s>,
			depth: usize,
		) -> Option<Found<'s>> {
			if depth > MAX_TABLE_DEPTH {
				return None;
			}

			for prop in table.props() {
				// Array element templates are found through their array, and
				// exclude properties only name variables of other tables.
				if prop.flags().contains(PropFlags::INSIDE_ARRAY)
					|| prop.flags().contains(PropFlags::EXCLUDE)
				{
					continue;
				}

				if prop.name() == name {
					return Some(Found {
						prop,
						struct_offset: base,
						offset: prop.offset(),
						relocated_by,
					});
				}

				if prop.kind() == PropKind::DataTable
					&& let Some(nested) = prop.data_table()
					&& let Some(nested_base) = usize::try_from(prop.offset())
						.ok()
						.and_then(|offset| base.checked_add(offset))
				{
					let relocated_by = relocated_by
						.or_else(|| (!proxies.is_direct(prop.table_proxy())).then_some(prop));

					if let Some(found) =
						search(nested, name, nested_base, relocated_by, proxies, depth + 1)
					{
						return Some(found);
					}
				}
			}

			None
		}

		let Found {
			prop,
			struct_offset,
			offset,
			relocated_by,
		} = search(table, name, 0, None, proxies, 0).ok_or_else(|| NetPropError::NotFound {
			table: lossy(table.name()),
			name: lossy(name),
		})?;

		if let Some(relocated_by) = relocated_by {
			return Err(NetPropError::Relocated {
				name: lossy(name),
				table: lossy(relocated_by.name()),
			});
		}

		let offset = usize::try_from(offset)
			.ok()
			.and_then(|offset| struct_offset.checked_add(offset))
			.ok_or_else(|| NetPropError::InvalidOffset {
				name: lossy(name),
				offset,
			})?;

		Ok(Self {
			prop,
			class_table: table,
			proxies,
			struct_offset,
			offset,
			element: 0,
		})
	}

	/// The property describing the variable.
	pub const fn prop(self) -> SendProp<'s> {
		self.prop
	}

	/// The table of the class the variable was resolved in.
	pub const fn class_table(self) -> SendTable<'s> {
		self.class_table
	}

	/// Bytes from the start of the entity to the variable.
	pub const fn offset(self) -> usize {
		self.offset
	}

	/// How the variable is stored.
	pub fn storage(self) -> Storage {
		self.proxies.storage(self.prop)
	}

	/// The number of elements of an array, or of variables in a nested table.
	pub fn element_count(self) -> Option<usize> {
		match self.prop.kind() {
			PropKind::Array => usize::try_from(self.prop.element_count()).ok(),
			PropKind::DataTable => self.prop.data_table().map(SendTable::len),
			_ => None,
		}
	}

	/// An element of an array, or a variable of a nested table, which
	/// `SendPropArray3` uses for arrays.
	pub fn element(self, index: usize) -> Result<Self, NetPropError> {
		let name = || lossy(self.prop.name());
		let out_of_range = |len| NetPropError::ElementOutOfRange {
			name: name(),
			index,
			len,
		};

		match self.prop.kind() {
			PropKind::Array => {
				let len = usize::try_from(self.prop.element_count()).unwrap_or(0);
				let element = self
					.prop
					.array_prop()
					.filter(|_| index < len)
					.ok_or_else(|| out_of_range(len))?;
				let stride = usize::try_from(self.prop.element_stride()).ok();
				let offset = usize::try_from(element.offset())
					.ok()
					.zip(stride)
					.and_then(|(offset, stride)| {
						self.struct_offset
							.checked_add(offset)?
							.checked_add(stride.checked_mul(index)?)
					})
					.ok_or_else(|| NetPropError::InvalidOffset {
						name: name(),
						offset: element.offset(),
					})?;

				Ok(Self {
					prop: element,
					offset,
					element: c_int::try_from(index).map_err(|_| out_of_range(len))?,
					..self
				})
			}

			PropKind::DataTable => {
				let table = self.prop.data_table().ok_or_else(|| out_of_range(0))?;
				let element = table.prop(index).ok_or_else(|| out_of_range(table.len()))?;

				if !self.proxies.is_direct(self.prop.table_proxy()) {
					return Err(NetPropError::Relocated {
						name: lossy(element.name()),
						table: name(),
					});
				}

				let offset = usize::try_from(element.offset())
					.ok()
					.and_then(|offset| self.offset.checked_add(offset))
					.ok_or_else(|| NetPropError::InvalidOffset {
						name: lossy(element.name()),
						offset: element.offset(),
					})?;

				Ok(Self {
					prop: element,
					struct_offset: self.offset,
					offset,
					element: 0,
					..self
				})
			}

			kind => Err(NetPropError::NotAnArray { name: name(), kind }),
		}
	}

	/// Reads the variable from an entity, as stored.
	pub fn get<T: NetVar>(self, entity: Entity<'_>) -> Result<T, NetPropError> {
		self.check_entity(entity)?;
		self.check_storage::<T>()?;

		// SAFETY: The entity's class derives from the class the offset was
		// resolved in, and the variable's storage is compatible with `T`. Entities are zeroed
		// when allocated, so every byte is initialized.
		Ok(unsafe { T::read(entity.as_ptr().cast::<u8>().add(self.offset)) })
	}

	/// Reads the variable from an entity as clients receive it, by calling
	/// its send proxy as the engine does when networking the entity.
	///
	/// This works for any scalar variable, including those whose storage is
	/// [unknown](Storage::Unknown), such as entity handles.
	#[doc(alias = "SendVarProxyFn")]
	pub fn value(self, entity: Entity<'_>) -> Result<NetValue, NetPropError> {
		let edict = self.check_entity(entity)?;
		let kind = self.prop.kind();

		if matches!(
			kind,
			PropKind::Array | PropKind::DataTable | PropKind::Unknown(_)
		) {
			return Err(NetPropError::NotAValue {
				name: lossy(self.prop.name()),
				kind,
			});
		}

		let proxy = self.prop.var_proxy().ok_or_else(|| NetPropError::NoProxy {
			name: lossy(self.prop.name()),
		})?;

		let base = entity.as_ptr().cast::<u8>();

		// SAFETY: The union is plain data, for which zeroes are valid.
		let mut value: sys::DVariant = unsafe { zeroed() };

		// SAFETY: The proxy is the game's own, called with the arguments the
		// engine passes: the variable, the structure containing it, and the
		// entity's index. The entity's class was checked, so both addresses lie
		// within the entity.
		unsafe {
			proxy(
				self.prop.as_ptr(),
				base.add(self.struct_offset).cast::<c_void>(),
				base.add(self.offset).cast::<c_void>(),
				&mut value,
				self.element,
				edict.index(),
			)
		};

		let value = value.__bindgen_anon_1;

		// SAFETY: Proxies fill in the union member matching the property type.
		Ok(unsafe {
			match kind {
				PropKind::Int => NetValue::Int(value.m_Int),
				PropKind::Float => NetValue::Float(value.m_Float),
				PropKind::Vector => NetValue::Vector(Vector::new(
					value.m_Vector[0],
					value.m_Vector[1],
					value.m_Vector[2],
				)),
				PropKind::VectorXY => {
					NetValue::VectorXY(Vec2::new(value.m_Vector[0], value.m_Vector[1]))
				}
				PropKind::String => {
					NetValue::String(copy_cstr(value.m_pString).unwrap_or_default())
				}
				PropKind::Array | PropKind::DataTable | PropKind::Unknown(_) => unreachable!(),
			}
		})
	}

	/// Writes the variable of an entity, and records the change so the engine
	/// sends it to clients.
	///
	/// # Safety
	///
	/// The game must accept `value` for this variable. Game code trusts its
	/// networked variables, such as a player's class or team, to hold values
	/// it could have assigned itself, and may index arrays with them.
	pub unsafe fn set<T: NetVar>(
		self,
		engine: ValveEngine<'_>,
		entity: Entity<'_>,
		value: T,
	) -> Result<(), NetPropError> {
		let edict = self.check_entity(entity)?;
		self.check_storage::<T>()?;

		// SAFETY: As for `get`. The game writes its variables the same way,
		// through its own pointers, on the main thread.
		unsafe { value.write(entity.as_ptr().cast::<u8>().add(self.offset)) };

		match u16::try_from(self.offset) {
			Ok(offset) => edict.state_changed(engine, offset),
			Err(_) => edict.full_state_changed(engine),
		}

		Ok(())
	}

	/// Checks that the variable's offset holds for `entity`, returning its edict.
	fn check_entity<'e>(self, entity: Entity<'e>) -> Result<Edict<'e>, NetPropError> {
		let not_networked = || NetPropError::NotNetworked {
			class_name: lossy(entity.class_name()),
		};

		let edict = entity.edict().ok_or_else(not_networked)?;
		let table = entity
			.server_class()
			.and_then(ServerClass::table)
			.ok_or_else(not_networked)?;

		if !table.derives_from(self.class_table) {
			return Err(NetPropError::ClassMismatch {
				name: lossy(self.prop.name()),
				expected: lossy(self.class_table.name()),
				class_name: lossy(entity.class_name()),
				found: lossy(table.name()),
			});
		}

		Ok(edict)
	}

	fn check_storage<T: NetVar>(self) -> Result<(), NetPropError> {
		let name = || lossy(self.prop.name());

		match self.prop.kind() {
			kind @ (PropKind::Array | PropKind::DataTable) => {
				Err(NetPropError::NotAValue { name: name(), kind })
			}

			_ => match self.storage() {
				Storage::Unknown => Err(NetPropError::UnknownStorage { name: name() }),
				storage if storage.is_compatible(T::STORAGE) => Ok(()),
				storage => Err(NetPropError::TypeMismatch {
					name: name(),
					storage,
					requested: type_name::<T>(),
				}),
			},
		}
	}
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use std::ffi::c_char;

	pub(crate) unsafe extern "C" fn int8_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe { (*out).__bindgen_anon_1.m_Int = c_int::from(data.cast::<i8>().read()) };
	}

	pub(crate) unsafe extern "C" fn int16_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe { (*out).__bindgen_anon_1.m_Int = c_int::from(data.cast::<i16>().read()) };
	}

	pub(crate) unsafe extern "C" fn int32_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe { (*out).__bindgen_anon_1.m_Int = data.cast::<c_int>().read() };
	}

	pub(crate) unsafe extern "C" fn vector_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe { (*out).__bindgen_anon_1.m_Vector = data.cast::<[f32; 3]>().read() };
	}

	/// A custom proxy, like `SendProxy_EHandleToInt`, that adds one to show it ran.
	pub(crate) unsafe extern "C" fn custom_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe { (*out).__bindgen_anon_1.m_Int = data.cast::<c_int>().read() + 1 };
	}

	pub(crate) unsafe extern "C" fn direct_table(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		_: *mut sys::CSendProxyRecipients,
		_: c_int,
	) -> *mut c_void {
		data.cast_mut()
	}

	pub(crate) unsafe extern "C" fn registered_table(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		_: *mut sys::CSendProxyRecipients,
		_: c_int,
	) -> *mut c_void {
		data.cast_mut()
	}

	pub(crate) unsafe extern "C" fn pointer_table(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		_: *mut sys::CSendProxyRecipients,
		_: c_int,
	) -> *mut c_void {
		unsafe { data.cast::<*mut c_void>().read() }
	}

	pub(crate) fn proxies(
		non_modified: *mut *mut sys::CNonModifiedPointerProxy,
	) -> sys::CStandardSendProxies {
		sys::CStandardSendProxies {
			_base: sys::CStandardSendProxiesV1 {
				m_Int8ToInt32: Some(int8_proxy),
				m_Int16ToInt32: Some(int16_proxy),
				m_Int32ToInt32: Some(int32_proxy),
				// Linkers fold identical code, so unsigned proxies may share addresses.
				m_UInt8ToInt32: Some(int8_proxy),
				m_UInt16ToInt32: Some(int16_proxy),
				m_UInt32ToInt32: Some(int32_proxy),
				m_FloatToFloat: Some(int32_proxy),
				m_VectorToVector: Some(vector_proxy),
			},
			m_DataTableToDataTable: Some(direct_table),
			m_SendLocalDataTable: Some(direct_table),
			m_ppNonModifiedPointerProxies: non_modified,
		}
	}

	pub(crate) fn prop(
		name: &'static CStr,
		kind: sys::SendPropType,
		offset: c_int,
		flags: PropFlags,
		proxy: sys::SendVarProxyFn,
	) -> sys::SendProp {
		// SAFETY: Properties are plain data apart from the vtable, which is never used.
		let mut prop: sys::SendProp = unsafe { zeroed() };

		prop.m_pVarName = name.as_ptr();
		prop.m_Type = kind;
		prop.m_Offset = offset;
		prop.m_Flags = flags.bits();
		prop.m_ProxyFn = proxy;
		prop.m_nElements = 1;
		prop
	}

	pub(crate) fn table_prop(
		name: &'static CStr,
		offset: c_int,
		table: *mut sys::SendTable,
		proxy: sys::SendTableProxyFn,
	) -> sys::SendProp {
		let mut prop = self::prop(
			name,
			sys::SendPropType_DPT_DataTable,
			offset,
			PropFlags::default(),
			None,
		);

		prop.m_pDataTable = table;
		prop.m_DataTableProxyFn = proxy;
		prop
	}

	pub(crate) fn table(name: &'static CStr, props: &mut [sys::SendProp]) -> sys::SendTable {
		// SAFETY: Tables are plain data.
		let mut table: sys::SendTable = unsafe { zeroed() };

		table.m_pNetTableName = name.as_ptr().cast::<c_char>();
		table.m_pProps = props.as_mut_ptr();
		table.m_nProps = props.len() as c_int;
		table
	}
}

#[cfg(test)]
mod tests {
	use super::test_support::*;
	use super::*;
	use crate::entities::test_support::{MockEntity, set_networking};
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::ptr::null_mut;

	fn int(name: &'static CStr, offset: c_int, proxy: sys::SendVarProxyFn) -> sys::SendProp {
		prop(
			name,
			sys::SendPropType_DPT_Int,
			offset,
			PropFlags::default(),
			proxy,
		)
	}

	/// `DT_Base` holds `m_iHealth` and a local table; `DT_Derived` embeds it
	/// as its base class, then an array, a relocated table, and variables with
	/// custom and unsigned proxies.
	struct Tables {
		base_props: Box<[sys::SendProp]>,
		local_props: Box<[sys::SendProp]>,
		derived_props: Box<[sys::SendProp]>,
		pointer_props: Box<[sys::SendProp]>,
		other_props: Box<[sys::SendProp]>,
		base: Box<sys::SendTable>,
		local: Box<sys::SendTable>,
		derived: Box<sys::SendTable>,
		pointer: Box<sys::SendTable>,
		other: Box<sys::SendTable>,
		registered: Box<sys::CNonModifiedPointerProxy>,
		registered_head: Box<*mut sys::CNonModifiedPointerProxy>,
		proxies: Box<sys::CStandardSendProxies>,
	}

	impl Tables {
		fn new() -> Box<Self> {
			let mut tables = Box::new(Self {
				base_props: Box::new([]),
				local_props: Box::new([]),
				derived_props: Box::new([]),
				pointer_props: Box::new([]),
				other_props: Box::new([]),
				base: Box::new(unsafe { zeroed() }),
				local: Box::new(unsafe { zeroed() }),
				derived: Box::new(unsafe { zeroed() }),
				pointer: Box::new(unsafe { zeroed() }),
				other: Box::new(unsafe { zeroed() }),
				registered: Box::new(sys::CNonModifiedPointerProxy {
					m_Fn: Some(registered_table),
					m_pNext: null_mut(),
				}),
				registered_head: Box::new(null_mut()),
				proxies: Box::new(proxies(null_mut())),
			});

			*tables.registered_head = &raw mut *tables.registered;
			tables.proxies.m_ppNonModifiedPointerProxies = &raw mut *tables.registered_head;

			tables.local_props = Box::new([
				prop(
					c"m_bDucked",
					sys::SendPropType_DPT_Int,
					4,
					PropFlags::UNSIGNED,
					Some(int8_proxy),
				),
				prop(
					c"m_vecPunchAngle",
					sys::SendPropType_DPT_Vector,
					8,
					PropFlags::default(),
					Some(vector_proxy),
				),
			]);
			tables.local = Box::new(table(c"DT_Local", &mut tables.local_props));

			tables.base_props = Box::new([
				int(c"m_iHealth", 16, Some(int32_proxy)),
				table_prop(
					c"m_Local",
					64,
					&raw mut *tables.local,
					Some(registered_table),
				),
			]);
			tables.base = Box::new(table(c"DT_Base", &mut tables.base_props));

			tables.pointer_props = Box::new([int(c"m_iHidden", 0, Some(int32_proxy))]);
			tables.pointer = Box::new(table(c"DT_Pointer", &mut tables.pointer_props));

			let mut element = int(c"m_iAmmo", 100, Some(int16_proxy));
			element.m_Flags = PropFlags::INSIDE_ARRAY.bits();
			let mut array = prop(
				c"m_iAmmo",
				sys::SendPropType_DPT_Array,
				0,
				PropFlags::default(),
				None,
			);
			array.m_nElements = 4;
			array.m_ElementStride = 2;

			tables.derived_props = Box::new([
				table_prop(c"baseclass", 0, &raw mut *tables.base, Some(direct_table)),
				prop(
					c"m_iHealth",
					sys::SendPropType_DPT_Int,
					0,
					PropFlags::EXCLUDE,
					None,
				),
				element,
				array,
				table_prop(
					c"m_Pointer",
					200,
					&raw mut *tables.pointer,
					Some(pointer_table),
				),
				int(c"m_hOwner", 120, Some(custom_proxy)),
				prop(
					c"m_nFlags",
					sys::SendPropType_DPT_Int,
					124,
					PropFlags::UNSIGNED,
					Some(int32_proxy),
				),
			]);

			let array_prop = &raw mut tables.derived_props[2];
			tables.derived_props[3].m_pArrayProp = array_prop;
			tables.derived = Box::new(table(c"DT_Derived", &mut tables.derived_props));

			tables.other_props = Box::new([int(c"m_iHealth", 16, Some(int32_proxy))]);
			tables.other = Box::new(table(c"DT_Other", &mut tables.other_props));

			tables
		}

		fn proxies(&self) -> StandardSendProxies<'_> {
			unsafe { StandardSendProxies::from_raw(NonNull::from(&*self.proxies)) }
		}

		fn table(table: &sys::SendTable) -> SendTable<'_> {
			unsafe { SendTable::from_raw(NonNull::from(table)) }
		}
	}

	#[test]
	fn lookups_follow_nested_tables_and_report_why_they_fail() {
		let tables = Tables::new();
		let proxies = tables.proxies();
		let derived = Tables::table(&tables.derived);
		let resolve = |name: &CStr| NetProp::resolve(derived, name, proxies);

		assert_eq!(derived.base().map(SendTable::name), Some(c"DT_Base"));
		assert!(derived.derives_from(Tables::table(&tables.base)));
		assert!(!Tables::table(&tables.base).derives_from(derived));

		// Found in the base class rather than as the exclude property.
		let health = resolve(c"m_iHealth").unwrap();
		assert_eq!((health.offset(), health.storage()), (16, Storage::I32));

		// Found through a registered, pointer-preserving table proxy.
		let ducked = resolve(c"m_bDucked").unwrap();
		assert_eq!((ducked.offset(), ducked.storage()), (68, Storage::U8));
		assert_eq!(
			resolve(c"m_vecPunchAngle").unwrap().storage(),
			Storage::Vector
		);

		// Unsigned flags pick the signedness even when proxies were folded.
		assert_eq!(resolve(c"m_nFlags").unwrap().storage(), Storage::U32);
		assert_eq!(resolve(c"m_hOwner").unwrap().storage(), Storage::Unknown);

		// Arrays resolve to their array property, and elements through it.
		let ammo = resolve(c"m_iAmmo").unwrap();
		assert_eq!(
			(ammo.prop().kind(), ammo.element_count()),
			(PropKind::Array, Some(4))
		);
		let third = ammo.element(2).unwrap();
		assert_eq!(
			(third.offset(), third.storage(), third.element),
			(104, Storage::I16, 2)
		);
		assert_eq!(
			ammo.element(4).unwrap_err().to_string(),
			"`m_iAmmo` has 4 elements, so it has no element 4"
		);

		let local = resolve(c"m_Local").unwrap();
		assert_eq!(local.element(1).unwrap().offset(), 72);

		assert_eq!(
			resolve(c"m_iHidden").unwrap_err().to_string(),
			"`m_iHidden` is inside `m_Pointer`, whose send proxy relocates its data, so its address is unknown"
		);
		assert_eq!(
			resolve(c"m_iMissing").unwrap_err().to_string(),
			"`DT_Derived` has no networked variable named `m_iMissing`"
		);
	}

	unsafe extern "C" fn change_accessor(
		_: *mut sys::IVEngineServer,
		_: *const sys::edict_t,
	) -> *mut sys::IChangeInfoAccessor {
		null_mut()
	}

	unsafe extern "C" fn shared_change_info(
		_: *mut sys::IVEngineServer,
	) -> *mut sys::CSharedEdictChangeInfo {
		null_mut()
	}

	#[test]
	fn variables_are_read_and_written_only_as_stored() {
		let tables = Tables::new();
		let proxies = tables.proxies();
		let derived = Tables::table(&tables.derived);
		let mut mock = MockEntity::new(3);

		// Make the mock entity networked, as a `DT_Derived`.
		let mut class = sys::ServerClass {
			m_pNetworkName: c"CDerived".as_ptr(),
			m_pTable: (&raw const *tables.derived).cast_mut(),
			m_pNext: null_mut(),
			m_ClassID: 1,
			m_InstanceBaselineIndex: 0,
		};
		let mut slot = crate::edicts::test_support::mock_edict(3, false);
		set_networking(&raw mut class, &raw mut slot);

		let entity = mock.entity();
		let at = |offset: usize| unsafe { entity.as_ptr().cast::<u8>().add(offset) };

		unsafe {
			at(16).cast::<i32>().write(125);
			at(68).write(1);
			at(104).cast::<i16>().write(-3);
			at(120).cast::<i32>().write(41);
		}

		let health = NetProp::resolve(derived, c"m_iHealth", proxies).unwrap();
		assert_eq!(health.get::<i32>(entity), Ok(125));
		assert_eq!(health.get::<u32>(entity), Ok(125));
		assert!(matches!(
			health.get::<i16>(entity),
			Err(NetPropError::TypeMismatch { .. })
		));
		assert_eq!(health.value(entity), Ok(NetValue::Int(125)));
		assert_eq!(
			health.get::<f32>(entity).unwrap_err().to_string(),
			"`m_iHealth` is stored as i32, not f32"
		);

		let ducked = NetProp::resolve(derived, c"m_bDucked", proxies).unwrap();
		assert_eq!(ducked.get::<bool>(entity), Ok(true));

		let third_ammo = NetProp::resolve(derived, c"m_iAmmo", proxies)
			.unwrap()
			.element(2)
			.unwrap();
		assert_eq!(third_ammo.get::<i16>(entity), Ok(-3));

		// Custom proxies hide the storage, but still produce the networked value.
		let owner = NetProp::resolve(derived, c"m_hOwner", proxies).unwrap();
		assert_eq!(owner.value(entity), Ok(NetValue::Int(42)));
		assert!(matches!(
			owner.get::<i32>(entity),
			Err(NetPropError::UnknownStorage { .. })
		));

		// Writes land at the offset and mark the edict changed.
		let vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
					(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo)
						.write(shared_change_info);
				},
			)
		};
		let mut interface = sys::IVEngineServer {
			vtable_: &raw const *vtable,
		};
		let engine = unsafe { ValveEngine::from_raw(NonNull::from(&mut interface)) };

		unsafe { health.set(engine, entity, 300) }.unwrap();
		assert_eq!(health.get::<i32>(entity), Ok(300));
		assert_ne!(slot._base.m_fStateFlags & 1, 0);

		// Offsets resolved for another class are refused.
		let other = NetProp::resolve(Tables::table(&tables.other), c"m_iHealth", proxies).unwrap();
		assert_eq!(
			other.get::<i32>(entity).unwrap_err().to_string(),
			"`m_iHealth` belongs to `DT_Other`, which `tf_player`'s table `DT_Derived` does not derive from"
		);

		set_networking(null_mut(), null_mut());
	}
}
