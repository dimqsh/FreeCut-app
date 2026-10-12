//! Sources decoded from media files with `freecut-media`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use freecut_media::AudioReader;
use freecut_timeline::ids::MediaId;
use freecut_timeline::time::{SampleRate, Tick};

use crate::source::{ReadError, SourceKey, SourceOpener, SourceReader};

/// Opens sources as stereo [`AudioReader`]s from a table of asset paths.
///
/// Each source names its stream by container index: the stream a clip's
/// selection plays, or one of the streams of "All streams mixed" (AU-038),
/// each opened as its own reader. Mono streams play on both channels at
/// unity and wider ones are downmixed with the ITU coefficients (AU-010,
/// done by [`AudioReader`]).
///
/// Assets missing from the table, files that fail to open and streams a
/// file doesn't have (a stream added as a clip whose file was relinked) are
/// offline: they play as silence. Cloning is cheap (the table is shared).
#[derive(Debug, Clone, Default)]
pub struct FileOpener {
    paths: Arc<BTreeMap<u64, PathBuf>>,
}

impl FileOpener {
    /// An opener for the assets in `paths` (asset id → file).
    pub fn new(paths: BTreeMap<u64, PathBuf>) -> FileOpener {
        FileOpener {
            paths: Arc::new(paths),
        }
    }

    /// An opener for the library items the app has found on disk, keyed
    /// like [`DocSchedule`](crate::DocSchedule)'s sources. `paths` gives the
    /// resolved file of each item; items in `offline` play as silence even
    /// when `paths` lists a file for them (AU-010: offline media contributes
    /// silence).
    ///
    /// An extracted audio item (AU-004) is read through its own entry, so
    /// `paths` must list it too (with its parent's file).
    pub fn from_paths(
        paths: &BTreeMap<MediaId, PathBuf>,
        offline: &BTreeSet<MediaId>,
    ) -> FileOpener {
        FileOpener::new(
            paths
                .iter()
                .filter(|(id, _)| !offline.contains(id))
                .map(|(id, path)| (id.get(), path.clone()))
                .collect(),
        )
    }
}

impl SourceOpener for FileOpener {
    fn open(&self, source: &SourceKey, rate: SampleRate) -> Option<Box<dyn SourceReader>> {
        let path = self.paths.get(&source.asset)?;
        let stream = usize::try_from(source.stream).ok()?;
        match AudioReader::open(path, stream, rate, 2) {
            Ok(reader) => Some(Box::new(FileReader(reader))),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    stream = source.stream,
                    error = %e,
                    "can't open audio source; it plays as silence"
                );
                None
            }
        }
    }
}

struct FileReader(AudioReader);

impl SourceReader for FileReader {
    fn read(&mut self, at: Tick, frames: usize, out: &mut [f32]) -> Result<(), ReadError> {
        let samples = self.0.read(at, frames).map_err(ReadError::new)?;
        let n = samples.len().min(out.len());
        out[..n].copy_from_slice(&samples[..n]);
        out[n..].fill(0.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU64;

    use super::*;
    use crate::source::SourceKey;

    fn media(id: u64) -> MediaId {
        MediaId::from_raw(NonZeroU64::new(id).unwrap())
    }

    #[test]
    fn offline_media_is_left_out() {
        let paths = BTreeMap::from([
            (media(3), PathBuf::from("/media/a.wav")),
            (media(4), PathBuf::from("/media/b.wav")),
        ]);
        let opener = FileOpener::from_paths(&paths, &BTreeSet::from([media(4), media(9)]));
        assert_eq!(
            *opener.paths,
            BTreeMap::from([(3, PathBuf::from("/media/a.wav"))])
        );
        // Not listed: no reader, so the mixer plays silence.
        let rate = SampleRate::new(48_000).unwrap();
        let key = SourceKey {
            asset: 4,
            stream: 0,
        };
        assert!(opener.open(&key, rate).is_none());
    }
}
