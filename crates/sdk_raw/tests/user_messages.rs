//! Tests of the recipient filters Rust implements for the engine.

use source_sdk_2013_raw::user_messages::RecipientFilter;

#[test]
fn filters_report_their_recipients_through_the_vtable() {
	let players = [3, 7];
	let filter = RecipientFilter::new(&players, true);
	let raw = filter.as_raw();

	// SAFETY: `raw` is the live filter, called through the bindings' vtable
	// as the engine calls it.
	unsafe {
		let vtable = (*raw).vtable_;

		assert!(((*vtable).IRecipientFilter_IsReliable)(raw));
		assert!(!((*vtable).IRecipientFilter_IsInitMessage)(raw));
		assert_eq!(((*vtable).IRecipientFilter_GetRecipientCount)(raw), 2);
		assert_eq!(((*vtable).IRecipientFilter_GetRecipientIndex)(raw, 1), 7);
		assert_eq!(((*vtable).IRecipientFilter_GetRecipientIndex)(raw, 2), -1);
		assert_eq!(((*vtable).IRecipientFilter_GetRecipientIndex)(raw, -1), -1);
	}
}
