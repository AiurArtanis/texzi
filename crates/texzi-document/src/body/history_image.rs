//! Two immutable roots are enough to travel a bulk edit. Large transitions
//! prepare a private, read-only backing before publication. Weak handles
//! reuse the exact live body (in particular FileBinding's original body).
use super::*;
use crate::{
    bulk::check,
    files::history_file::{Backing, Temporary},
    provenance::archive::{Sources, Topology},
};
use std::sync::{Mutex, Weak, atomic::AtomicBool};

#[derive(Clone)]
pub(crate) enum HistoryRoot {
    Resident(Arc<Body>),
    Backed(Arc<Image>),
}
impl HistoryRoot {
    pub(crate) fn load(&self) -> Result<Arc<Body>, Error> {
        match self {
            Self::Resident(body) => Ok(body.clone()),
            Self::Backed(image) => image.load(),
        }
    }
    pub(crate) fn resident(&self) -> Option<Arc<Body>> {
        match self {
            Self::Resident(root) => Some(root.clone()),
            Self::Backed(image) => image.cache.lock().unwrap().upgrade(),
        }
    }
}
pub(crate) struct HistoryRoots {
    pub(crate) before: HistoryRoot,
    pub(crate) after: HistoryRoot,
}
impl HistoryRoots {
    pub(crate) fn prepare(
        before: &Arc<Body>,
        after: &Arc<Body>,
        resident: usize,
        map_bytes: usize,
        cancelled: &AtomicBool,
    ) -> Result<(Self, usize), Error> {
        if resident <= 16 * 1024 * 1024 {
            return Ok((
                Self {
                    before: HistoryRoot::Resident(before.clone()),
                    after: HistoryRoot::Resident(after.clone()),
                },
                resident,
            ));
        }
        let mut sources = Sources::default();
        let first = Image::save(before, &mut sources, cancelled)?;
        let second = Image::save(after, &mut sources, cancelled)?;
        let charge = 8192 + map_bytes + first.topology.allocation() + second.topology.allocation();
        Ok((
            Self {
                before: HistoryRoot::Backed(Arc::new(first)),
                after: HistoryRoot::Backed(Arc::new(second)),
            },
            charge,
        ))
    }
}
pub(crate) struct Image {
    cache: Mutex<Weak<Body>>,
    restoring: Mutex<()>,
    file: Backing,
    lengths: [usize; 3],
    counts: [usize; 3],
    profile: Profile,
    topology: Topology,
    legacy: Option<Arc<crate::files::legacy_aliases::Aliases>>,
}
impl Image {
    fn save(
        body: &Arc<Body>,
        sources: &mut Sources,
        cancelled: &AtomicBool,
    ) -> Result<Self, Error> {
        let mut file = Temporary::new()?;
        let ropes = [&body.text, &body.line_breaks, &body.eols];
        for rope in ropes {
            for chunk in rope.chunks() {
                check(cancelled)?;
                file.append(chunk.as_bytes())?;
            }
        }
        let topology = Topology::write(&body.origins, &mut file, sources, cancelled)?;
        Ok(Self {
            cache: Mutex::new(Arc::downgrade(body)),
            restoring: Mutex::new(()),
            file: file.seal()?,
            lengths: ropes.map(Rope::len_bytes),
            counts: body.counts,
            profile: body.profile,
            topology,
            legacy: body.legacy.clone(),
        })
    }
    fn load(&self) -> Result<Arc<Body>, Error> {
        if let Some(body) = self.cache.lock().unwrap().upgrade() {
            return Ok(body);
        }
        let _restoring = self.restoring.lock().unwrap();
        if let Some(body) = self.cache.lock().unwrap().upgrade() {
            return Ok(body);
        }
        let mut rest = &self.file[..];
        let mut read = |len| -> Result<Rope, Error> {
            let (bytes, tail) = rest.split_at_checked(len).ok_or(Error::HistoryBudget)?;
            rest = tail;
            Ok(Rope::from_str(
                std::str::from_utf8(bytes).map_err(|_| Error::HistoryBudget)?,
            ))
        };
        let text = read(self.lengths[0])?;
        let line_breaks = read(self.lengths[1])?;
        let eols = read(self.lengths[2])?;
        let body = Arc::new(Body {
            text,
            line_breaks,
            eols,
            counts: self.counts,
            profile: self.profile,
            origins: self.topology.read(rest)?,
            legacy: self.legacy.clone(),
        });
        let mut cache = self.cache.lock().unwrap();
        *cache = Arc::downgrade(&body);
        Ok(body)
    }
}
