//! The one-time HDR notice (CL-002). FreeCut converts PQ and HLG video to
//! SDR for preview, thumbnails and export, and tells the user once per
//! media item. The poster of every video item (requested on import and
//! again when a project opens) reports whether its source is HDR, so the
//! notice comes from there, off the UI thread. The app drops a poster that
//! finishes after another project opened (media ids restart with every
//! project), so the notice never names an item of the wrong project.

use std::collections::BTreeSet;

use freecut_media::color::HdrTransfer;
use freecut_timeline::ids::MediaId;

/// The media items of the open project already announced as HDR.
#[derive(Debug, Default)]
pub struct HdrNotices {
    told: BTreeSet<MediaId>,
}

impl HdrNotices {
    /// Whether the notice for `media`, whose source is `hdr`, is due now:
    /// the first time an HDR transfer (PQ or HLG) is reported for it.
    /// Later reports for the same item, and SDR sources, are not.
    pub fn due(&mut self, media: MediaId, hdr: Option<HdrTransfer>) -> bool {
        match hdr {
            Some(HdrTransfer::Pq | HdrTransfer::Hlg) => self.told.insert(media),
            Some(HdrTransfer::Sdr) | None => false,
        }
    }

    /// Another project opened: its items get their own notice.
    pub fn reset(&mut self) {
        self.told.clear();
    }
}

/// The transfer's short name in the notice.
fn transfer_name(tf: HdrTransfer) -> &'static str {
    match tf {
        HdrTransfer::Pq => "PQ",
        HdrTransfer::Hlg => "HLG",
        HdrTransfer::Sdr => "SDR",
    }
}

/// The status-line notice for HDR items that became known together (their
/// names and transfers): FreeCut shows and exports them converted to SDR.
/// `None` when there are none.
pub fn hdr_notice(items: &[(String, HdrTransfer)]) -> Option<String> {
    match items {
        [] => None,
        [(name, tf)] => Some(format!(
            "{name} is HDR video ({}): FreeCut converts it to SDR for preview and export",
            transfer_name(*tf)
        )),
        several => Some(format!(
            "{} files are HDR video: FreeCut converts them to SDR for preview and export",
            several.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;

    fn id(n: u64) -> MediaId {
        MediaId::from_raw(NonZeroU64::new(n).unwrap_or(NonZeroU64::MIN))
    }

    #[test]
    fn the_notice_is_due_once_per_hdr_item() {
        let mut notices = HdrNotices::default();
        assert!(notices.due(id(1), Some(HdrTransfer::Hlg)));
        assert!(!notices.due(id(1), Some(HdrTransfer::Hlg)), "not repeated");
        assert!(notices.due(id(2), Some(HdrTransfer::Pq)), "another item");
        assert!(!notices.due(id(3), None), "SDR");
        assert!(!notices.due(id(3), Some(HdrTransfer::Sdr)), "BT.2020 SDR");
        assert!(
            notices.due(id(3), Some(HdrTransfer::Pq)),
            "SDR didn't count"
        );
        // A project opened again: its HDR items are announced again.
        notices.reset();
        assert!(notices.due(id(1), Some(HdrTransfer::Hlg)));
    }

    #[test]
    fn the_notice_names_one_file_or_counts_several() {
        assert_eq!(hdr_notice(&[]), None);
        assert_eq!(
            hdr_notice(&[("beach.mov".to_owned(), HdrTransfer::Hlg)]).as_deref(),
            Some("beach.mov is HDR video (HLG): FreeCut converts it to SDR for preview and export")
        );
        assert_eq!(
            hdr_notice(&[
                ("a.mov".to_owned(), HdrTransfer::Pq),
                ("b.mov".to_owned(), HdrTransfer::Hlg),
            ])
            .as_deref(),
            Some("2 files are HDR video: FreeCut converts them to SDR for preview and export")
        );
    }
}
