//! Host actions only the s2/s3 model scheduler and the host tests take;
//! production never calls them.
use super::*;

impl<F: HostIo> Host<F> {
    pub(super) fn abstract_queue(&self) -> Vec<Request> {
        self.applying
            .iter()
            .chain(self.queue.iter())
            .cloned()
            .collect()
    }

    pub(super) fn load(&mut self, key: &str) -> Disposition {
        if !self.alive || !self.keys.contains(key) || self.pages.contains_key(key) {
            return Disposition::Disabled;
        }
        let keys = BTreeSet::from([key.into()]);
        if self.busy(key) || self.allocator_busy() || self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| host.load_locked(key))
    }

    pub(super) fn switch_request(&mut self) -> Disposition {
        if self.worker.is_some() {
            return Disposition::Waiting;
        }
        let keys = self.pages.keys().cloned().collect();
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            for key in &keys {
                let mut page = host.pages[key].clone();
                if !page.clean() {
                    page.risk = true;
                }
                host.set_page(key, Some(page));
            }
        });
        Disposition::Applied
    }
}
