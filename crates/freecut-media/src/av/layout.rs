//! Channel layouts: which speaker each channel of a stream feeds, and the
//! layout's name.
//!
//! Channels are reported as FFmpeg `AVChannel` numbers read as plain
//! integers (never as the Rust enum, which a newer libav could extend, see
//! the module docs of [`crate::av`]). The numbering is part of FFmpeg's
//! public ABI: bit `n` of a native-order mask is channel `n`.

use std::ffi::{CStr, c_char, c_int};
use std::mem::MaybeUninit;
use std::ptr;

use ffmpeg_next::ffi;
use ffmpeg_next::format::context::Input;

/// `AV_CHANNEL_ORDER_NATIVE`: channels in mask bit order.
const ORDER_NATIVE: c_int = 1;
/// `AV_CHANNEL_ORDER_CUSTOM`: channels listed in `u.map`.
const ORDER_CUSTOM: c_int = 2;

/// The channels of `layout`, in stream order, for native and custom
/// orders; `None` for unspecified and ambisonic layouts.
///
/// # Safety
/// `layout` must point to a valid `AVChannelLayout` that stays alive and
/// unchanged for the call.
unsafe fn channel_ids_of(layout: *const ffi::AVChannelLayout) -> Option<Vec<i32>> {
    // SAFETY: the caller guarantees `layout` is valid. The order is read as a
    // raw integer. For a native order the union holds `mask`; for a custom
    // order it holds `map`, an array of `nb_channels` entries owned by the
    // layout, whose `id` fields are read as raw integers.
    unsafe {
        let order = ptr::addr_of!((*layout).order).cast::<c_int>().read();
        let count = usize::try_from((*layout).nb_channels).ok()?;
        match order {
            ORDER_NATIVE => {
                let mask = (*layout).u.mask;
                let ids: Vec<i32> = (0..64).filter(|b| mask & (1u64 << b) != 0).collect();
                (ids.len() == count && count > 0).then_some(ids)
            }
            ORDER_CUSTOM => {
                let map = (*layout).u.map;
                if map.is_null() || count == 0 {
                    return None;
                }
                Some(
                    (0..count)
                        .map(|i| ptr::addr_of!((*map.add(i)).id).cast::<c_int>().read())
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

/// The codec parameters' channel layout of stream `stream`, or null.
fn stream_layout(input: &Input, stream: usize) -> *const ffi::AVChannelLayout {
    // SAFETY: read-only access to the context owned by `input`; only streams
    // below `nb_streams` are dereferenced. The returned pointer is only used
    // while `input` is borrowed by the callers below.
    unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return ptr::null();
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() || (*st).codecpar.is_null() {
            return ptr::null();
        }
        ptr::addr_of!((*(*st).codecpar).ch_layout)
    }
}

/// The speaker of each channel of stream `stream`, in stream order, when
/// the file says (native or custom layouts).
pub(crate) fn stream_channel_ids(input: &Input, stream: usize) -> Option<Vec<i32>> {
    let layout = stream_layout(input, stream);
    if layout.is_null() {
        return None;
    }
    // SAFETY: `layout` points into the codec parameters owned by `input`,
    // which is borrowed (unchanged) for the call.
    unsafe { channel_ids_of(layout) }
}

/// FFmpeg's name of stream `stream`'s channel layout ("mono", "stereo",
/// "5.1(side)", …); `None` when the file doesn't say which speakers the
/// channels feed.
pub(crate) fn stream_layout_name(input: &Input, stream: usize) -> Option<String> {
    let layout = stream_layout(input, stream);
    if layout.is_null() {
        return None;
    }
    // SAFETY: as in `stream_channel_ids`; the order is read as an integer.
    let order = unsafe { ptr::addr_of!((*layout).order).cast::<c_int>().read() };
    if order != ORDER_NATIVE && order != ORDER_CUSTOM {
        return None;
    }
    let mut buf = [0 as c_char; 128];
    // SAFETY: `layout` is valid (see above); `buf` is writable for its full
    // length, which is passed, and `av_channel_layout_describe` always
    // NUL-terminates what it writes when the size is positive.
    let ret = unsafe { ffi::av_channel_layout_describe(layout, buf.as_mut_ptr(), buf.len()) };
    if ret <= 0 {
        return None;
    }
    // SAFETY: the buffer is NUL-terminated (see above) and outlives the read.
    let name = unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    (!name.is_empty()).then_some(name)
}

/// The channels of FFmpeg's default layout for `channels` channels, in
/// order (what an unlabelled stream is assumed to carry).
pub(crate) fn default_channel_ids(channels: u16) -> Option<Vec<i32>> {
    let mut layout = MaybeUninit::<ffi::AVChannelLayout>::zeroed();
    // SAFETY: an all-zero `AVChannelLayout` is a valid "unspecified, 0
    // channels" layout; `av_channel_layout_default` overwrites it with a
    // native (or unspecified) layout that owns no heap memory, so nothing
    // needs to be freed.
    let layout = unsafe {
        ffi::av_channel_layout_default(layout.as_mut_ptr(), c_int::from(channels));
        layout.assume_init()
    };
    // SAFETY: `layout` is a valid local value for the call.
    unsafe { channel_ids_of(&raw const layout) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layouts_have_the_usual_speakers() {
        assert_eq!(default_channel_ids(1), Some(vec![2]), "mono is centre");
        assert_eq!(default_channel_ids(2), Some(vec![0, 1]));
        let six = default_channel_ids(6).unwrap();
        assert_eq!(six.len(), 6);
        assert_eq!(&six[..4], &[0, 1, 2, 3], "FL FR FC LFE first");
    }
}
