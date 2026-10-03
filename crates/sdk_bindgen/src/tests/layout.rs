//! Tests of [`crate::layout`]: emulating C record layout against bindgen's
//! Clang-derived assertions.

use super::*;
use crate::SupportedTarget;
use syn::parse_quote;

const LINUX_64: TargetLayout = SupportedTarget::Linux64.layout();
const WINDOWS_64: TargetLayout = SupportedTarget::Windows64.layout();

#[test]
fn accepts_layouts_that_match_their_assertions() {
	let syntax: File = parse_quote! {
		#[repr(C, align(8))]
		pub struct __BindgenOpaqueArray8<T>(pub T);
		#[repr(C)]
		pub struct __BindgenBitfieldUnit<Storage> { storage: Storage }
		pub type _fieldtypes = ::std::os::raw::c_uint;
		pub use self::_fieldtypes as fieldtype_t;
		#[repr(C)]
		pub struct Inner<T> {
			pub _phantom_0: ::std::marker::PhantomData<::std::cell::UnsafeCell<T>>,
			pub value: T,
		}
		#[repr(C)]
		pub struct Outer__bindgen_vtable(::std::os::raw::c_void);
		#[repr(C)]
		pub struct Outer {
			pub vtable_: *const Outer__bindgen_vtable,
			pub flag: bool,
			pub inner: Inner<f64>,
			pub callback: ::std::option::Option<unsafe extern "C" fn(arg1: ::std::os::raw::c_long)>,
			pub blob: __BindgenOpaqueArray8<[u8; 12usize]>,
			pub _bindgen_align: [u32; 0],
			pub _bitfield_1: __BindgenBitfieldUnit<[u8; 2usize]>,
			pub kind: fieldtype_t,
			pub long_value: ::std::os::raw::c_long,
		}
		#[repr(C)]
		pub union Either { pub a: u8, pub b: ::std::mem::ManuallyDrop<[u16; 3usize]> }
		#[repr(C, packed(4))]
		pub struct Packed { pub a: u8, pub b: *mut Outer }
		const _: () = {
			["Size of Outer"][::std::mem::size_of::<Outer>() - 64usize];
			["Alignment of Outer"][::std::mem::align_of::<Outer>() - 8usize];
			["Offset of field: Outer::inner"][::std::mem::offset_of!(Outer, inner) - 16usize];
			["Offset of field: Outer::blob"][::std::mem::offset_of!(Outer, blob) - 32usize];
			["Offset of field: Outer::kind"][::std::mem::offset_of!(Outer, kind) - 52usize];
			["Offset of field: Outer::long_value"][::std::mem::offset_of!(Outer, long_value) - 56usize];
		};
		const _: () = {
			["Size of Either"][::std::mem::size_of::<Either>() - 6usize];
			["Alignment of Either"][::std::mem::align_of::<Either>() - 2usize];
		};
		const _: () = {
			["Size of Packed"][::std::mem::size_of::<Packed>() - 12usize];
			["Alignment of Packed"][::std::mem::align_of::<Packed>() - 4usize];
			["Offset of field: Packed::b"][::std::mem::offset_of!(Packed, b) - 4usize];
		};
		const _: () = {
			["Size of template specialization: Inner_open0_float_close0"][::std::mem::size_of::<Inner<f32>>() - 4usize];
			["Align of template specialization: Inner_open0_float_close0"][::std::mem::align_of::<Inner<f32>>() - 4usize];
		};
	};

	assert_eq!(
		find_layout_mismatches(&syntax, LINUX_64),
		LayoutMismatches::new()
	);
}

#[test]
fn c_long_follows_the_target_data_model() {
	let syntax: File = parse_quote! {
		#[repr(C)]
		pub struct Longs { pub a: ::std::os::raw::c_long, pub b: ::std::os::raw::c_int }
		const _: () = { ["Size of Longs"][::std::mem::size_of::<Longs>() - 8usize]; };
	};

	assert_eq!(
		find_layout_mismatches(&syntax, WINDOWS_64),
		LayoutMismatches::new()
	);
	assert!(find_layout_mismatches(&syntax, LINUX_64).contains_key("Longs"));
}

#[test]
fn removes_only_broken_aliases_nothing_uses() {
	let mut syntax: File = parse_quote! {
		#[repr(C)]
		pub struct CUtlRBTree<T, I> { pub root: I, pub elements: *mut T }
		pub type CUtlMap_CTree = CUtlRBTree<T, I>;
		pub type CUtlHash_Buckets = CUtlRBTree<T, A>;
		#[repr(C)]
		pub struct UsesHash { pub buckets: *mut CUtlHash_Buckets }
		pub type Broken = CUtlRBTree<T, u16>;
		pub type NamesBroken = Broken;
		#[repr(C)]
		pub struct CUtlMap { pub _bindgen_opaque_blob: [u64; 5usize] }
	};

	// A broken alias something still names must stay, so the record
	// using it is blamed instead.
	assert_eq!(
		remove_unreferenced_broken_aliases(&mut syntax),
		["CUtlMap_CTree"]
	);
	assert_eq!(syntax.items.len(), 6);
	assert_eq!(
		find_layout_mismatches(&syntax, LINUX_64)
			.keys()
			.collect::<Vec<_>>(),
		["Broken", "CUtlHash_Buckets"]
	);
}

#[test]
fn reports_tail_padding_reuse_on_the_derived_record_only() {
	// `Derived::extra` lives in the tail padding of the non-POD base.
	let syntax: File = parse_quote! {
		#[repr(C)]
		pub struct Base__bindgen_vtable(::std::os::raw::c_void);
		#[repr(C)]
		pub struct Base { pub vtable_: *const Base__bindgen_vtable, pub value: u32 }
		#[repr(C)]
		pub struct Derived { pub _base: Base, pub extra: u32 }
		#[repr(C)]
		pub struct Holder { pub derived: Derived, pub tail: u8 }
		const _: () = {
			["Size of Base"][::std::mem::size_of::<Base>() - 16usize];
			["Offset of field: Base::value"][::std::mem::offset_of!(Base, value) - 8usize];
		};
		const _: () = {
			["Size of Derived"][::std::mem::size_of::<Derived>() - 16usize];
			["Alignment of Derived"][::std::mem::align_of::<Derived>() - 8usize];
			["Offset of field: Derived::extra"][::std::mem::offset_of!(Derived, extra) - 12usize];
		};
		const _: () = {
			["Size of Holder"][::std::mem::size_of::<Holder>() - 24usize];
			["Offset of field: Holder::tail"][::std::mem::offset_of!(Holder, tail) - 16usize];
		};
	};

	let mismatches = find_layout_mismatches(&syntax, LINUX_64);
	assert_eq!(
		mismatches.keys().collect::<Vec<_>>(),
		["Derived"],
		"{mismatches:?}"
	);
}

#[test]
fn reports_the_record_owning_an_unbound_template_parameter() {
	let syntax: File = parse_quote! {
		#[repr(C)]
		pub struct CUtlRBTree<T, I> { pub root: I, pub elements: *mut T }
		pub type CUtlMap_CTree = CUtlRBTree<T, I>;
		pub type CUtlMap_ElemType_t<T> = T;
		#[repr(C)]
		pub struct CUtlMap { pub m_Tree: CUtlMap_CTree }
		#[repr(C)]
		pub struct CUtlHash<C> { pub m_Buckets: CUtlVector<T, A>, pub m_Compare: C }
		#[repr(C)]
		pub struct CUtlVector<T, A> { pub memory: A, pub elements: *mut T }
		pub type Linked_const_iterator = Linked_iterator_t<List_t>;
		#[repr(C)]
		pub struct Linked { pub head: u16 }
		#[repr(C)]
		pub struct Linked_iterator_t<List_t> { pub list: *const List_t }
	};

	let mismatches = find_layout_mismatches(&syntax, LINUX_64);
	assert_eq!(
		mismatches.keys().collect::<Vec<_>>(),
		["CUtlHash", "CUtlMap_CTree", "Linked_const_iterator"],
		"{mismatches:?}"
	);
}

#[test]
fn reports_the_template_behind_a_wrong_specialization() {
	// `#pragma pack(4)` is lost on the template, so its instantiation is over-aligned.
	let syntax: File = parse_quote! {
		#[repr(C)]
		pub struct serializedstudioptr_t<T> {
			pub _phantom_0: ::std::marker::PhantomData<::std::cell::UnsafeCell<T>>,
			pub m_pData: *mut T,
		}
		#[repr(C, packed(4))]
		pub struct mstudiomesh_t { pub material: ::std::os::raw::c_int, pub data: serializedstudioptr_t<::std::os::raw::c_void> }
		#[repr(C)]
		pub struct Unpacked { pub first: u32, pub data: serializedstudioptr_t<u8> }
		const _: () = {
			["Size of mstudiomesh_t"][::std::mem::size_of::<mstudiomesh_t>() - 12usize];
			["Alignment of mstudiomesh_t"][::std::mem::align_of::<mstudiomesh_t>() - 4usize];
		};
		const _: () = {
			["Size of Unpacked"][::std::mem::size_of::<Unpacked>() - 12usize];
			["Offset of field: Unpacked::data"][::std::mem::offset_of!(Unpacked, data) - 4usize];
		};
		const _: () = {
			["Size of template specialization: serializedstudioptr_t_open0_unsigned_char_close0"][::std::mem::size_of::<serializedstudioptr_t<u8>>() - 8usize];
			["Align of template specialization: serializedstudioptr_t_open0_unsigned_char_close0"][::std::mem::align_of::<serializedstudioptr_t<u8>>() - 4usize];
		};
	};

	let mismatches = find_layout_mismatches(&syntax, LINUX_64);
	assert_eq!(
		mismatches.keys().collect::<Vec<_>>(),
		["serializedstudioptr_t"],
		"{mismatches:?}"
	);
}
